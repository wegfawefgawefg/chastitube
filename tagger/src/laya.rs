//! Laya (convaiinnovations/laya, Apache 2.0) through ONNX Runtime.
//!
//! The input is built the way laya's own common.py does it (sequential option
//! layout): [CLS] "<type> question: <text>" [SEP] [MASK] option0 [MASK] option1 ... [SEP] <state> [SEP]
//! The encoder reads that, the head scores each [MASK], and a softmax with the
//! checkpoint's temperature turns the scores into probabilities.
//! Checked against the Python package: same answers to within 0.01.
use ort::session::Session;
use ort::value::Tensor;
use serde_json::Value;
use std::path::Path;
use tokenizers::Tokenizer;

pub enum Question {
    /// yes/no: the answer is P(yes)
    YesNo(String),
    /// pick one of these (label, description) options
    Choice(String, Vec<(String, String)>),
}

pub struct Laya {
    tok: Tokenizer,
    encoder: Session,
    head: Session,
    cls: u32,
    sep: u32,
    mask: u32,
    max_len: usize,
    head_max_len: usize,
    config: Value,
}

type Res<T> = Result<T, String>;

fn err<E: std::fmt::Display>(what: &str) -> impl Fn(E) -> String + '_ {
    move |e| format!("{what}: {e}")
}

impl Laya {
    pub fn load(dir: &Path, threads: usize) -> Res<Laya> {
        let mut tok = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(err("tokenizer"))?;
        tok.with_truncation(None).map_err(err("tokenizer"))?;
        tok.with_padding(None);
        let id = |t: &str| tok.token_to_id(t).ok_or(format!("tokenizer has no {t}"));
        let (cls, sep, mask) = (id("[CLS]")?, id("[SEP]")?, id("[MASK]")?);
        let config: Value = std::fs::read_to_string(dir.join("rl_agent_config.json")).map_err(err("rl_agent_config.json"))
            .and_then(|t| serde_json::from_str(&t).map_err(err("rl_agent_config.json")))?;
        let env = ort::init().with_telemetry(false).build().map_err(err("onnx runtime"))?;
        let session = |file: &str| -> Res<Session> {
            Session::builder(&env).map_err(err(file))?
                .with_intra_threads(threads).map_err(err(file))?
                // memory patterns plan for the largest input seen; off keeps peak RAM near the model size
                .with_memory_pattern(false).map_err(err(file))?
                .commit_from_file(dir.join(file)).map_err(err(file))
        };
        Ok(Laya {
            encoder: session("encoder.onnx")?,
            head: session("head.onnx")?,
            cls, sep, mask,
            max_len: config["max_len"].as_u64().unwrap_or(512) as usize,
            head_max_len: config["head_max_len"].as_u64().unwrap_or(192) as usize,
            config,
            tok,
        })
    }

    pub fn encode(&self, text: &str) -> Vec<u32> {
        self.tok.encode(text.replace("[MASK]", " "), false).map(|e| e.get_ids().to_vec()).unwrap_or_default()
    }

    /// Probabilities for each option (for yes/no: [no, yes]). `state` comes from encode().
    pub fn ask(&mut self, state: &[u32], q: &Question) -> Res<Vec<f32>> {
        let (kind, text, options) = match q {
            Question::YesNo(t) => ("noul", t.as_str(), vec!["false: no, the statement does not hold".to_string(),
                                                            "true: yes, the statement holds".to_string()]),
            Question::Choice(t, opts) => ("choice", t.as_str(), opts.iter()
                .map(|(k, d)| if d.is_empty() { k.clone() } else { format!("{k}: {d}") }).collect()),
        };
        let (mut ids, markers) = self.question_ids(kind, text, &options);
        let room = self.max_len.saturating_sub(ids.len() + 1);
        ids.extend(state.iter().take(room));
        ids.push(self.sep);
        ids.truncate(self.max_len);
        let markers: Vec<usize> = markers.into_iter().filter(|&m| m < self.max_len).collect();
        if markers.len() != options.len() {
            return Err("question too long".into());
        }

        let (l, k) = (ids.len(), markers.len());
        let ids: Vec<i64> = ids.iter().map(|&t| t as i64).collect();
        let out = self.encoder.run(ort::inputs![
            "input_ids" => Tensor::from_array(([1, l], ids)).map_err(err("input"))?,
            "attention_mask" => Tensor::from_array(([1, l], vec![1i64; l])).map_err(err("input"))?,
        ]).map_err(err("encoder"))?;
        let (shape, hidden) = out[0].try_extract_tensor::<f32>().map_err(err("encoder output"))?;
        let width = shape[2] as usize;
        let hidden = hidden.to_vec();
        drop(out);
        let qtype = if kind == "choice" { 0i64 } else { 2 };
        let scale = self.temperature(kind, k);
        let out = self.head.run(ort::inputs![
            "hidden_states" => Tensor::from_array(([1, l, width], hidden)).map_err(err("head input"))?,
            "marker_pos" => Tensor::from_array(([1, k], markers.iter().map(|&m| m as i64).collect::<Vec<_>>())).map_err(err("head input"))?,
            "marker_mask" => Tensor::from_array(([1, k], vec![true; k])).map_err(err("head input"))?,
            "qtype" => Tensor::from_array(([1, 1], vec![qtype])).map_err(err("head input"))?,
            "attention_mask" => Tensor::from_array(([1, l], vec![1i64; l])).map_err(err("head input"))?,
        ]).map_err(err("head"))?;
        let (_, logits) = out["logits"].try_extract_tensor::<f32>().map_err(err("head output"))?;
        Ok(softmax(&logits[..k].iter().map(|v| v / scale).collect::<Vec<_>>()))
    }

    /// [CLS] "<kind> question: <text>" [SEP] then [MASK] + each option, then [SEP];
    /// returns the ids and where each [MASK] is.
    fn question_ids(&self, kind: &str, text: &str, options: &[String]) -> (Vec<u32>, Vec<usize>) {
        let mut head = self.encode(&format!("{kind} question: {text}"));
        let mut opts: Vec<Vec<u32>> = options.iter().map(|o| {
            let mut v = vec![self.mask];
            v.extend(self.encode(&format!(" {o}")).into_iter().take(48));
            v
        }).collect();
        let used = |o: &Vec<Vec<u32>>| o.iter().map(Vec::len).sum::<usize>() as i64;
        let mut budget = self.head_max_len as i64 - used(&opts);
        if budget < 16 {
            let per = 4.max(self.head_max_len.saturating_sub(16) / opts.len().max(1));
            for o in opts.iter_mut() {
                o.truncate(per);
            }
            budget = self.head_max_len as i64 - used(&opts);
        }
        head.truncate(8.max(budget.max(0) as usize));
        let mut ids = vec![self.cls];
        ids.extend(head);
        ids.push(self.sep);
        let mut markers = Vec::new();
        for o in opts {
            markers.push(ids.len());
            ids.extend(o);
        }
        ids.push(self.sep);
        (ids, markers)
    }

    /// The checkpoint's calibration: a temperature per question type and option count.
    fn temperature(&self, kind: &str, k: usize) -> f32 {
        let size = match k { 0..=2 => "2", 3..=5 => "3-5", 6..=10 => "6-10", _ => "11+" };
        let qt = if kind == "choice" { 0 } else { 2 };
        self.config["temperature_by_options"][format!("{kind}:{size}")].as_f64()
            .or_else(|| self.config["temperature"][qt].as_f64())
            .unwrap_or(1.0).clamp(0.5, 5.0) as f32
    }
}

fn softmax(z: &[f32]) -> Vec<f32> {
    let m = z.iter().cloned().fold(f32::MIN, f32::max);
    let e: Vec<f32> = z.iter().map(|v| (v - m).exp()).collect();
    let s: f32 = e.iter().sum();
    e.iter().map(|v| v / s).collect()
}
