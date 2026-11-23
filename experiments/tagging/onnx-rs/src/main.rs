//! Laya (typed-decisions) from Rust through ONNX Runtime, no Python.
//! Same input building as laya's common.py / laya-js common.ts (sequential option layout).
//!
//!   laya-onnx-test <model dir> <questions.json> <videos.jsonl> <out.json> [threads]
//!
//! Writes results in the same shape as tag.py, so score.py can read them.

use ort::session::Session;
use ort::value::Tensor;
use serde_json::{json, Map, Value};
use std::time::Instant;
use tokenizers::Tokenizer;

struct Q {
    id: String,
    t: &'static str, // "choice" | "noul"
    ins: String,
    opts: Vec<String>, // rendered options
    keys: Vec<String>, // choice keys
}

struct Laya {
    tok: Tokenizer,
    enc: Session,
    head: Session,
    cls: u32,
    sep: u32,
    mask: u32,
    pad: u32,
    max_len: usize,
    head_max_len: usize,
    temp: Value,
}

fn qtype(t: &str) -> i64 {
    match t {
        "choice" => 0,
        "score" => 1,
        _ => 2,
    }
}

fn bucket(t: &str, k: usize) -> String {
    let size = if k <= 2 { "2" } else if k <= 5 { "3-5" } else if k <= 10 { "6-10" } else { "11+" };
    format!("{t}:{size}")
}

fn softmax(z: &[f32]) -> Vec<f32> {
    let m = z.iter().cloned().fold(f32::MIN, f32::max);
    let e: Vec<f32> = z.iter().map(|v| (v - m).exp()).collect();
    let s: f32 = e.iter().sum();
    e.iter().map(|v| v / s).collect()
}

impl Laya {
    fn load(dir: &str, threads: usize) -> Laya {
        let mut tok = Tokenizer::from_file(format!("{dir}/tokenizer.json")).expect("tokenizer");
        tok.with_truncation(None).unwrap();
        tok.with_padding(None);
        let id = |t: &str| tok.token_to_id(t).unwrap_or_else(|| panic!("no {t} token"));
        let (cls, sep, mask, pad) = (id("[CLS]"), id("[SEP]"), id("[MASK]"), id("[PAD]"));
        let cfg: Value = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/rl_agent_config.json")).unwrap()).unwrap();
        let env = ort::init().with_telemetry(false).build().unwrap();
        let sess = |f: &str| {
            let b = Session::builder(&env).unwrap().with_intra_threads(threads).unwrap();
            let mut b = if std::env::var("MEMPAT").as_deref() == Ok("0") { b.with_memory_pattern(false).unwrap() } else { b };
            b.commit_from_file(format!("{dir}/{f}")).unwrap()
        };
        Laya {
            enc: sess("encoder.onnx"),
            head: sess("head.onnx"),
            cls, sep, mask, pad,
            max_len: cfg["max_len"].as_u64().unwrap_or(512) as usize,
            head_max_len: cfg["head_max_len"].as_u64().unwrap_or(192) as usize,
            temp: cfg,
            tok,
        }
    }

    fn encode(&self, text: &str) -> Vec<u32> {
        self.tok.encode(text, false).unwrap().get_ids().to_vec()
    }

    /// [CLS] "<type> question: <ins>" [SEP] [MASK] opt0 [MASK] opt1 ... [SEP], plus marker positions.
    fn prefix(&self, q: &Q) -> (Vec<u32>, Vec<usize>) {
        let ins = q.ins.replace("[MASK]", " ");
        let mut head = self.encode(&format!("{} question: {}", q.t, ins));
        let mut opts: Vec<Vec<u32>> = q.opts.iter().map(|o| {
            let mut v = vec![self.mask];
            v.extend(self.encode(&format!(" {}", o.replace("[MASK]", " "))).into_iter().take(48));
            v
        }).collect();
        let total = |o: &Vec<Vec<u32>>| o.iter().map(|x| x.len()).sum::<usize>() as i64;
        let mut budget = self.head_max_len as i64 - total(&opts);
        if budget < 16 {
            let per = 4.max((self.head_max_len.saturating_sub(16)) / opts.len().max(1));
            for o in opts.iter_mut() { o.truncate(per); }
            budget = self.head_max_len as i64 - total(&opts);
        }
        head.truncate(8.max(budget.max(0) as usize));
        let mut ids = vec![self.cls];
        ids.extend(head);
        ids.push(self.sep);
        let mut markers = vec![];
        for o in opts {
            markers.push(ids.len());
            ids.extend(o);
        }
        ids.push(self.sep);
        (ids, markers)
    }

    fn predict(&mut self, state: &str, qs: &[Q]) -> Vec<Vec<f32>> {
        let st = self.encode(&state.replace("[MASK]", " "));
        let rows: Vec<(Vec<u32>, Vec<usize>)> = qs.iter().map(|q| {
            let (mut ids, markers) = self.prefix(q);
            let room = self.max_len.saturating_sub(ids.len() + 1);
            ids.extend(st.iter().take(room));
            ids.push(self.sep);
            ids.truncate(self.max_len);
            let markers = markers.into_iter().filter(|&m| m < self.max_len).collect();
            (ids, markers)
        }).collect();
        let n = rows.len();
        let l = rows.iter().map(|r| r.0.len()).max().unwrap();
        let k = rows.iter().map(|r| r.1.len()).max().unwrap();
        let mut ids = vec![self.pad as i64; n * l];
        let mut am = vec![0i64; n * l];
        let mut mpos = vec![0i64; n * k];
        let mut mmask = vec![false; n * k];
        for (i, (r, m)) in rows.iter().enumerate() {
            for (j, &t) in r.iter().enumerate() { ids[i * l + j] = t as i64; am[i * l + j] = 1; }
            for (j, &p) in m.iter().enumerate() { mpos[i * k + j] = p as i64; mmask[i * k + j] = true; }
        }
        let qt: Vec<i64> = qs.iter().map(|q| qtype(q.t)).collect();

        let out = self.enc.run(ort::inputs![
            "input_ids" => Tensor::from_array(([n, l], ids)).unwrap(),
            "attention_mask" => Tensor::from_array(([n, l], am.clone())).unwrap(),
        ]).unwrap();
        let (shape, hidden) = out[0].try_extract_tensor::<f32>().unwrap();
        let hd = shape[2] as usize;
        let hidden = hidden.to_vec();
        drop(out);
        let out = self.head.run(ort::inputs![
            "hidden_states" => Tensor::from_array(([n, l, hd], hidden)).unwrap(),
            "marker_pos" => Tensor::from_array(([n, k], mpos)).unwrap(),
            "marker_mask" => Tensor::from_array(([n, k], mmask)).unwrap(),
            "qtype" => Tensor::from_array(([n, 1], qt)).unwrap(),
            "attention_mask" => Tensor::from_array(([n, l], am)).unwrap(),
        ]).unwrap();
        let (lshape, logits) = out["logits"].try_extract_tensor::<f32>().unwrap();
        let kk = lshape[1] as usize;
        qs.iter().enumerate().map(|(i, q)| {
            let nk = rows[i].1.len();
            let t = &self.temp;
            let scale = t["temperature_by_options"][bucket(q.t, nk)].as_f64()
                .or_else(|| t["temperature"][qtype(q.t) as usize].as_f64()).unwrap_or(1.0)
                .clamp(0.5, 5.0) as f32;
            let z: Vec<f32> = logits[i * kk..i * kk + nk].iter().map(|v| v / scale).collect();
            softmax(&z)
        }).collect()
    }
}

fn state(v: &Value) -> String {
    let kw: Vec<&str> = v["keywords"].as_array().map(|a| a.iter().take(25).filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    let desc: String = v["description"].as_str().unwrap_or("").chars().take(600).collect();
    format!("Title: {}\nChannel: {}\nYouTube category: {}\nKeywords: {}\nDescription: {}",
            v["title"].as_str().unwrap_or(""), v["channel"].as_str().unwrap_or(""),
            v["category"].as_str().unwrap_or(""), kw.join(", "), desc)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let threads = a.get(5).and_then(|s| s.parse().ok()).unwrap_or(4);
    let t0 = Instant::now();
    let mut laya = Laya::load(&a[1], threads);
    eprintln!("loaded in {:.1} s", t0.elapsed().as_secs_f32());
    let qdefs: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let qs: Vec<Q> = qdefs.iter().map(|d| {
        let t = if d["type"] == "choice" { "choice" } else { "noul" };
        let (opts, keys) = if t == "choice" {
            let c = d["criteria"].as_object().unwrap();
            (c.iter().map(|(k, v)| match v.as_str() {
                Some("") | None => k.clone(),
                Some(s) => format!("{k}: {s}"),
            }).collect(), c.keys().cloned().collect())
        } else {
            (vec!["false: no, the statement does not hold".into(), "true: yes, the statement holds".into()], vec![])
        };
        Q { id: d["id"].as_str().unwrap().into(), t, ins: d["instructions"].as_str().unwrap().into(), opts, keys }
    }).collect();
    let mut results = vec![];
    let mut times = vec![];
    for line in std::fs::read_to_string(&a[3]).unwrap().lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        let t = Instant::now();
        // CHUNK=n asks n questions per model run instead of all at once (less memory)
        let chunk = std::env::var("CHUNK").ok().and_then(|c| c.parse().ok()).unwrap_or(qs.len());
        let st = state(&v);
        let ps: Vec<Vec<f32>> = qs.chunks(chunk).flat_map(|c| laya.predict(&st, c)).collect();
        times.push(t.elapsed().as_secs_f32());
        let mut yes = Map::new();
        let mut raw = Map::new();
        let mut format = String::new();
        let mut choices = Map::new();
        let mut probs = Map::new();
        for (q, p) in qs.iter().zip(&ps) {
            if q.t == "noul" {
                yes.insert(q.id.clone(), json!((p[1] as f64 * 100.0).round() / 100.0));
                raw.insert(q.id.clone(), json!(p[1]));
            } else {
                let best = (0..p.len()).max_by(|&i, &j| p[i].total_cmp(&p[j])).unwrap();
                if q.id == "format" { format = q.keys[best].clone(); }
                choices.insert(q.id.clone(), json!(q.keys[best]));
            }
            probs.insert(q.id.clone(), json!(p));
        }
        eprintln!("{:.2} s  {}", times.last().unwrap(), v["title"].as_str().unwrap_or("").chars().take(60).collect::<String>());
        results.push(json!({"id": v["id"], "title": v["title"], "yes": yes, "raw": raw, "format": format, "choices": choices, "probs": probs}));
    }
    std::fs::write(&a[4], serde_json::to_string_pretty(&results).unwrap()).unwrap();
    let rest = &times[1.min(times.len() - 1)..];
    eprintln!("per video {:.2} s (first {:.2} s), {} threads", rest.iter().sum::<f32>() / rest.len() as f32, times[0], threads);
}
