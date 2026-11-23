//! chastitube-tagger: asks Laya which topics each video you opened is about.
//!
//! Optional (./install.sh --tags builds it and downloads the model). The
//! server starts it at low priority when there's something new; it answers
//! whatever hasn't been asked yet and exits, so its memory is given back.
//!
//! Reads, in ~/.local/share/chastitube:
//!   topics.txt     the topics, one per line ("name: what it covers"); written with defaults if missing
//!   details.jsonl  the videos (title, channel, category, keywords, description)
//!   laya/          the model: encoder.onnx, head.onnx, tokenizer.json, rl_agent_config.json
//! Writes:
//!   tags.jsonl          {"id", "scores": {topic: P(yes)}, "kind"}, one line per run per video
//!   tag_baselines.json  each topic's median score over a fixed set of ordinary videos;
//!                       the server tags a topic when a video scores well above it
mod laya;

use laya::{Laya, Question};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const THREADS: usize = 4;

const DEFAULT_TOPICS: &str = "\
# Topics chastitube tags videos with, one per line: name: what it covers.
# Each topic is one more question per video (about 0.3 s each).
# Rename a topic to have every video asked about it again.
games: video games: playing, guides, playthroughs, news about games
programming: writing software, programming languages, code
technology: computers, phones, gadgets, tech news
science: physics, chemistry, astronomy, space, experiments
math: mathematics: algebra, calculus, geometry, proofs
biology and medicine: living things, the body, health, medicine
history: past events, empires, wars, historical people
politics and news: current events, politics, world news
music: songs, performances, composers, albums
film and tv: movies, shows, trailers, reviews
comedy: stand-up, sketches, jokes
sports: sports games, highlights, athletes
fitness: workouts, exercise, yoga
cooking: food, recipes, cooking
travel: places, trips, travel vlogs
diy and crafts: building, repairing, woodworking, crafts
art and design: drawing, painting, design, animation
vehicles: cars, motorbikes, planes, trains
money and business: finance, investing, business, careers
language learning: learning to speak a foreign language like Spanish, Japanese or English
nature and animals: animals, pets, wildlife, nature
";

const KINDS: [(&str, &str); 9] = [
    ("lecture", "a talk or class teaching a subject"),
    ("tutorial or guide", "step by step how-to"),
    ("explainer", "short explanation of an idea"),
    ("playthrough or let's play", "someone playing a game through"),
    ("devlog or project log", "someone showing what they built"),
    ("performance or song", "music being played"),
    ("vlog", "someone's day or trip"),
    ("documentary", "long factual film"),
    ("entertainment", "for fun: comedy, challenges, reactions"),
];

/// Ordinary videos from unrelated searches, for each topic's baseline.
const REFERENCE: &str = include_str!("../reference.jsonl");

fn data_dir() -> PathBuf {
    std::env::var_os("CHASTITUBE_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".local/share/chastitube")
    })
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// (name, question) for each topic in topics.txt.
fn topics(dir: &PathBuf) -> Vec<(String, String)> {
    let path = dir.join("topics.txt");
    let text = fs::read_to_string(&path).unwrap_or_else(|_| {
        let _ = fs::write(&path, DEFAULT_TOPICS);
        DEFAULT_TOPICS.to_string()
    });
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (name, about) = line.split_once(':').map(|(n, a)| (n.trim(), a.trim())).unwrap_or((line, ""));
        let name = name.to_lowercase();
        if name.is_empty() || out.iter().any(|t| t.0 == name) {
            continue;
        }
        let q = if about.is_empty() { format!("Is this video about {name}?") } else { format!("Is this video about {name} ({about})?") };
        out.push((name, q));
    }
    out
}

/// The text Laya reads about a video.
fn state(v: &Value) -> String {
    let kw: Vec<&str> = v["keywords"].as_array().map(|a| a.iter().take(25).filter_map(Value::as_str).collect()).unwrap_or_default();
    let desc: String = v["description"].as_str().unwrap_or("").chars().take(600).collect();
    format!("Title: {}\nChannel: {}\nYouTube category: {}\nKeywords: {}\nDescription: {}",
            v["title"].as_str().unwrap_or(""), v["channel"].as_str().unwrap_or(""),
            v["category"].as_str().unwrap_or(""), kw.join(", "), desc)
}

fn read_jsonl(path: PathBuf) -> Vec<Value> {
    fs::read_to_string(path).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// What each video has been asked already: topic names, and whether it has a kind.
fn asked(dir: &PathBuf) -> HashMap<String, (HashSet<String>, bool)> {
    let mut out: HashMap<String, (HashSet<String>, bool)> = HashMap::new();
    for line in read_jsonl(dir.join("tags.jsonl")) {
        let Some(id) = line["id"].as_str() else { continue };
        let e = out.entry(id.to_string()).or_default();
        if let Some(s) = line["scores"].as_object() {
            e.0.extend(s.keys().cloned());
        }
        e.1 |= line["kind"].is_string();
    }
    out
}

fn main() {
    let dir = data_dir();
    let topics = topics(&dir);
    let done = asked(&dir);
    let baselines: Map<String, Value> = fs::read_to_string(dir.join("tag_baselines.json")).ok()
        .and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();

    // newest details last in the file; latest copy of each video wins; newest first
    let mut videos: Vec<Value> = Vec::new();
    let mut seen = HashSet::new();
    for v in read_jsonl(dir.join("details.jsonl")).into_iter().rev() {
        if let Some(id) = v["id"].as_str() {
            if seen.insert(id.to_string()) {
                videos.push(v);
            }
        }
    }
    let todo: Vec<(Value, Vec<usize>, bool)> = videos.into_iter().filter_map(|v| {
        let (have, has_kind) = done.get(v["id"].as_str()?).cloned().unwrap_or_default();
        let missing: Vec<usize> = (0..topics.len()).filter(|&i| !have.contains(&topics[i].0)).collect();
        (!missing.is_empty() || !has_kind).then_some((v, missing, !has_kind))
    }).collect();
    let no_baseline: Vec<usize> = (0..topics.len()).filter(|&i| !baselines.contains_key(&topics[i].0)).collect();
    if todo.is_empty() && no_baseline.is_empty() {
        return;
    }

    let mut laya = match Laya::load(&dir.join("laya"), THREADS) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("chastitube-tagger: can't load the model in {}: {e}", dir.join("laya").display());
            std::process::exit(1);
        }
    };
    let kind_q = Question::Choice("What kind of video is this?".into(),
                                  KINDS.iter().map(|(k, d)| (k.to_string(), d.to_string())).collect());

    for (v, missing, need_kind) in todo {
        let st = laya.encode(&state(&v));
        let mut scores = Map::new();
        for &i in &missing {
            match laya.ask(&st, &Question::YesNo(topics[i].1.clone())) {
                Ok(p) => { scores.insert(topics[i].0.clone(), json!((p[1] as f64 * 1000.0).round() / 1000.0)); }
                Err(e) => eprintln!("chastitube-tagger: {}: {e}", topics[i].0),
            }
        }
        let mut line = json!({"id": v["id"], "scores": scores, "time": now()});
        if need_kind {
            if let Ok(p) = laya.ask(&st, &kind_q) {
                let best = (0..p.len()).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap_or(0);
                line["kind"] = json!(KINDS[best].0);
            }
        }
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(dir.join("tags.jsonl")) {
            let _ = writeln!(f, "{line}");
        }
    }

    if no_baseline.is_empty() {
        return;
    }
    let reference: Vec<Vec<u32>> = REFERENCE.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .map(|v| laya.encode(&state(&v))).collect();
    let mut baselines = baselines;
    for i in no_baseline {
        let mut s: Vec<f32> = reference.iter().filter_map(|st| laya.ask(st, &Question::YesNo(topics[i].1.clone())).ok()).map(|p| p[1]).collect();
        if s.is_empty() {
            continue;
        }
        s.sort_by(|a, b| a.total_cmp(b));
        baselines.insert(topics[i].0.clone(), json!((s[s.len() / 2] as f64 * 1000.0).round() / 1000.0));
        // written after each topic, so a stopped run keeps what it finished
        let tmp = dir.join("tag_baselines.json.tmp");
        if fs::write(&tmp, serde_json::to_string_pretty(&baselines).unwrap_or_default()).is_ok() {
            let _ = fs::rename(&tmp, dir.join("tag_baselines.json"));
        }
    }
}
