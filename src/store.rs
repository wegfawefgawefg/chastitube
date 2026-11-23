//! Everything kept on disk, in ~/.local/share/chastitube:
//!   history.jsonl   what you opened, when, and how you got there
//!   searches.jsonl  what you searched
//!   positions.json  where you stopped in each video
//!   watch.json      seconds actually played, per day and video
//!   details.jsonl   YouTube's details for each video you opened (category,
//!                   keywords, description), for searching history
//! All of it is read once at start and kept in memory.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::util::{data_dir, lock, now, today};
use crate::youtube::Details;

const REPEAT_WINDOW: f64 = 30.0 * 60.0; // reopening a video or repeating a search within this counts once
const RESUME_MIN: f64 = 30.0; // don't bother resuming in the first or last 30 seconds

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Entry {
    pub time: f64,
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub struct Pos {
    pub pos: f64,
    pub dur: f64,
    #[serde(default)]
    pub time: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Search {
    pub time: f64,
    pub q: String,
}

pub type Watch = BTreeMap<String, BTreeMap<String, f64>>; // day -> video -> seconds

struct Store {
    history: Vec<Entry>,
    searches: Vec<Search>,
    positions: HashMap<String, Pos>,
    watch: Watch,
    watch_dirty: bool,
    details: HashMap<String, Details>,
}

fn path(name: &str) -> PathBuf {
    data_dir().join(name)
}

fn read_jsonl<T: for<'de> Deserialize<'de>>(name: &str) -> Vec<T> {
    fs::read_to_string(path(name)).unwrap_or_default().lines()
        .filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn append_line<T: Serialize>(name: &str, item: &T) {
    let _ = fs::create_dir_all(data_dir());
    if let (Ok(mut f), Ok(line)) = (fs::OpenOptions::new().create(true).append(true).open(path(name)),
                                    serde_json::to_string(item)) {
        let _ = writeln!(f, "{line}");
    }
}

fn write_json<T: Serialize>(name: &str, value: &T) {
    let _ = fs::create_dir_all(data_dir());
    let tmp = path(&format!("{name}.tmp"));
    if let Ok(text) = serde_json::to_string(value) {
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, path(name));
        }
    }
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| {
        let history: Vec<Entry> = read_jsonl("history.jsonl");
        let searches = if path("searches.jsonl").exists() {
            read_jsonl("searches.jsonl")
        } else {
            searches_from_history(&history)
        };
        let positions = fs::read_to_string(path("positions.json")).ok()
            .and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let watch = fs::read_to_string(path("watch.json")).ok()
            .and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let details = read_jsonl::<Details>("details.jsonl").into_iter().map(|d| (d.id.clone(), d)).collect();
        Mutex::new(Store { history, searches, positions, watch, watch_dirty: false, details })
    })
}

/// Before searches were kept, history still recorded what each video was
/// found with, so the first run rebuilds the list from that.
fn searches_from_history(history: &[Entry]) -> Vec<Search> {
    let mut out: Vec<Search> = Vec::new();
    for e in history.iter().filter(|e| e.source == "search" && !e.detail.is_empty()) {
        if !out.iter().any(|s| s.q == e.detail && e.time - s.time < REPEAT_WINDOW) {
            out.push(Search { time: e.time, q: e.detail.clone() });
        }
    }
    for s in &out {
        append_line("searches.jsonl", s);
    }
    out
}

/// Load everything now, and write watch time out every 20 seconds.
pub fn start() {
    let _ = store();
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(20));
        flush();
    });
}

pub fn flush() {
    let mut st = lock(store());
    if st.watch_dirty {
        write_json("watch.json", &st.watch);
        st.watch_dirty = false;
    }
}

// ------------------------------------------------------------------ history

/// source: search | link | paste | history | saved | playlist | direct.
/// detail is the search, the app name for links, or the playlist.
pub fn add_history(id: &str, title: &str, channel: &str, source: &str, detail: &str) {
    let t = now();
    let mut st = lock(store());
    if st.history.iter().rev().take_while(|e| t - e.time < REPEAT_WINDOW).any(|e| e.id == id) {
        return;
    }
    let e = Entry { time: t, id: id.into(), title: title.into(), channel: channel.into(),
                    source: source.into(), detail: detail.into() };
    append_line("history.jsonl", &e);
    st.history.push(e);
}

pub fn history() -> Vec<Entry> {
    lock(store()).history.iter().rev().cloned().collect()
}

/// The newest title and channel history has for a video.
pub fn known_title(id: &str) -> Option<(String, String)> {
    let st = lock(store());
    if let Some(d) = st.details.get(id) {
        return Some((d.title.clone(), d.channel.clone()));
    }
    st.history.iter().rev().find(|e| e.id == id && !e.title.is_empty() && e.title != e.id)
        .map(|e| (e.title.clone(), e.channel.clone()))
}

// ------------------------------------------------------------------ searches

pub fn add_search(q: &str) {
    let t = now();
    let mut st = lock(store());
    if st.searches.iter().rev().take_while(|s| t - s.time < REPEAT_WINDOW).any(|s| s.q == q) {
        return;
    }
    let s = Search { time: t, q: q.into() };
    append_line("searches.jsonl", &s);
    st.searches.push(s);
}

pub fn searches() -> Vec<Search> {
    lock(store()).searches.iter().rev().cloned().collect()
}

// ------------------------------------------------------------------ resume points + watch time

/// Saves where you are, and adds `played` seconds to today's watch time.
pub fn set_position(id: &str, pos: f64, dur: f64, played: f64) {
    let mut st = lock(store());
    st.positions.insert(id.into(), Pos { pos: (pos * 10.0).round() / 10.0, dur: (dur * 10.0).round() / 10.0, time: now() });
    write_json("positions.json", &st.positions);
    // the page reports every 5 seconds; anything much bigger is a clock jump
    if played > 0.0 && played <= 30.0 {
        *st.watch.entry(today()).or_default().entry(id.into()).or_default() += played;
        st.watch_dirty = true;
    }
}

pub fn positions() -> HashMap<String, Pos> {
    lock(store()).positions.clone()
}

pub fn watch() -> Watch {
    lock(store()).watch.clone()
}

/// Seconds to resume from, or 0 if the video wasn't started or was finished.
pub fn resume_point(p: Option<&Pos>) -> u64 {
    match p {
        Some(p) if p.pos >= RESUME_MIN && !(p.dur > 0.0 && p.pos > p.dur - RESUME_MIN) => (p.pos as u64).saturating_sub(3),
        _ => 0,
    }
}

pub fn finished(p: Option<&Pos>) -> bool {
    matches!(p, Some(p) if p.dur > 0.0 && p.pos > p.dur - RESUME_MIN)
}

// ------------------------------------------------------------------ video details

pub fn details(id: &str) -> Option<Details> {
    lock(store()).details.get(id).cloned()
}

pub fn all_details() -> HashMap<String, Details> {
    lock(store()).details.clone()
}

pub fn put_details(d: Details) {
    append_line("details.jsonl", &d);
    lock(store()).details.insert(d.id.clone(), d);
}
