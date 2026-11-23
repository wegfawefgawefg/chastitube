//! Topic tags, optional. The tagger program (tagger/, built by `./install.sh --tags`)
//! asks Laya how likely each video is to be about each topic and writes the answers
//! to tags.jsonl. This file turns those scores into tags, keeps your own fixes
//! (tag_fixes.jsonl, which always win), and starts the tagger when there's
//! something new. Tagging is on when the tagger and its model are both there.
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::util::{app_dir, data_dir, lock, now};

/// A topic is tagged when a video scores this far above the topic's usual
/// score (its median over ordinary videos, in tag_baselines.json)...
const ABOVE_USUAL: f64 = 0.3;
/// ...and the best topic only if it stands out at least this much; below
/// that the video isn't about any of the topics.
const BEST_ABOVE_USUAL: f64 = 0.15;
/// At most this many tags per video.
const MAX_TAGS: usize = 3;

#[derive(Default)]
struct Loaded {
    stamps: Vec<Option<SystemTime>>,
    scores: HashMap<String, HashMap<String, f64>>,
    kinds: HashMap<String, String>,
    fixes: HashMap<String, HashMap<String, bool>>,
    baselines: HashMap<String, f64>,
    topics: Vec<String>,
}

/// What the pages need: each video's tags and kind, and the list of topics.
#[derive(Default)]
pub struct View {
    pub tags: HashMap<String, Vec<String>>,
    pub kinds: HashMap<String, String>,
    pub topics: Vec<String>,
    pub enabled: bool,
}

const FILES: [&str; 4] = ["tags.jsonl", "tag_fixes.jsonl", "tag_baselines.json", "topics.txt"];

fn loaded() -> &'static Mutex<Loaded> {
    static L: OnceLock<Mutex<Loaded>> = OnceLock::new();
    L.get_or_init(Default::default)
}

fn tagger() -> PathBuf {
    app_dir().join("bin/chastitube-tagger")
}

pub fn enabled() -> bool {
    tagger().is_file() && data_dir().join("laya/encoder.onnx").is_file()
}

fn read_lines(name: &str) -> Vec<serde_json::Value> {
    fs::read_to_string(data_dir().join(name)).unwrap_or_default().lines()
        .filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Topic names from topics.txt, the same way the tagger reads them.
fn read_topics() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in fs::read_to_string(data_dir().join("topics.txt")).unwrap_or_default().lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let name = line.split_once(':').map(|(n, _)| n).unwrap_or(line).trim().to_lowercase();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Re-read the files when the tagger (or a fix) has changed them.
fn refresh(l: &mut Loaded) {
    let stamps: Vec<Option<SystemTime>> = FILES.iter()
        .map(|f| fs::metadata(data_dir().join(f)).and_then(|m| m.modified()).ok()).collect();
    if stamps == l.stamps {
        return;
    }
    let mut fresh = Loaded { stamps, topics: read_topics(), ..Default::default() };
    for line in read_lines("tags.jsonl") {
        let Some(id) = line["id"].as_str() else { continue };
        if let Some(s) = line["scores"].as_object() {
            let e = fresh.scores.entry(id.to_string()).or_default();
            for (t, v) in s {
                if let Some(v) = v.as_f64() {
                    e.insert(t.clone(), v);
                }
            }
        }
        if let Some(k) = line["kind"].as_str() {
            fresh.kinds.insert(id.to_string(), k.to_string());
        }
    }
    for line in read_lines("tag_fixes.jsonl") {
        if let (Some(id), Some(tag), Some(on)) = (line["id"].as_str(), line["tag"].as_str(), line["on"].as_bool()) {
            fresh.fixes.entry(id.to_string()).or_default().insert(tag.to_string(), on);
        }
    }
    fresh.baselines = fs::read_to_string(data_dir().join("tag_baselines.json")).ok()
        .and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    *l = fresh;
}

/// The model's tags for one video: topics well above their usual score.
fn model_tags(scores: &HashMap<String, f64>, l: &Loaded) -> Vec<String> {
    let mut above: Vec<(&String, f64)> = l.topics.iter()
        .filter_map(|t| Some((t, scores.get(t)? - l.baselines.get(t)?))).collect();
    above.sort_by(|a, b| b.1.total_cmp(&a.1));
    let Some(&(_, best)) = above.first() else { return Vec::new() };
    if best < BEST_ABOVE_USUAL {
        return Vec::new();
    }
    above.iter().enumerate()
        .filter(|(i, (_, s))| *i == 0 || (*s >= ABOVE_USUAL && *s >= best - 0.25))
        .take(MAX_TAGS).map(|(_, (t, _))| t.to_string()).collect()
}

pub fn view() -> View {
    let mut l = lock(loaded());
    refresh(&mut l);
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    for (id, scores) in &l.scores {
        tags.insert(id.clone(), model_tags(scores, &l));
    }
    for (id, fixes) in &l.fixes {
        let t = tags.entry(id.clone()).or_default();
        for (tag, on) in fixes {
            t.retain(|x| x != tag);
            if *on {
                t.push(tag.clone());
            }
        }
    }
    View { tags, kinds: l.kinds.clone(), topics: l.topics.clone(), enabled: enabled() }
}

/// You added (on) or removed a tag; this always wins over the model.
pub fn fix(id: &str, tag: &str, on: bool) {
    let line = serde_json::json!({"id": id, "tag": tag.trim().to_lowercase(), "on": on, "time": now()});
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(data_dir().join("tag_fixes.jsonl")) {
        let _ = writeln!(f, "{line}");
    }
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static AGAIN: AtomicBool = AtomicBool::new(false);

/// Start the tagger if tagging is on and it isn't already running (if it is,
/// it runs once more when it finishes). It's started at the lowest priority,
/// tags what's new and exits.
pub fn kick() {
    if !enabled() {
        return;
    }
    AGAIN.store(true, Ordering::SeqCst);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        while AGAIN.swap(false, Ordering::SeqCst) {
            let status = Command::new("nice").arg("-n").arg("19").arg(tagger())
                .stdin(std::process::Stdio::null()).status();
            if !matches!(status, Ok(s) if s.success()) {
                eprintln!("tagger: {status:?}");
                break;
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
        // a kick that came in just as the last run ended
        if AGAIN.load(Ordering::SeqCst) {
            kick();
        }
    });
}
