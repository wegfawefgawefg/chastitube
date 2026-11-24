//! yt-dlp (downloads, and search if YouTube's API stops working), the saved
//! folder, and the caches that keep pages from waiting on YouTube.
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::store;
use crate::util::{app_dir, lock, save_dir, valid_id, valid_list};
use crate::youtube::{self, Playlist, PlaylistHit, Video};

const FORMAT: &str = "bv*[height<=1080]+ba/b";
const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "webm", "m4a", "opus"];
const PLAYLIST_CACHE: Duration = Duration::from_secs(600);

fn ytdlp() -> PathBuf {
    app_dir().join("bin/yt-dlp")
}

pub fn watch_url(vid: &str) -> String {
    format!("https://www.youtube.com/watch?v={vid}")
}

// ------------------------------------------------------------------ yt-dlp setup

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|f| f.is_file()))
}

fn version_key(p: &Path) -> Vec<u64> {
    p.to_string_lossy().split('/').find(|s| s.starts_with('v') && s[1..].split('.').all(|n| n.parse::<u64>().is_ok()))
        .map(|s| s[1..].split('.').filter_map(|n| n.parse().ok()).collect()).unwrap_or_default()
}

/// yt-dlp needs a JS runtime for YouTube. Apps started from i3 don't get the
/// nvm PATH, so look in the usual places by hand.
fn js_runtime() -> Option<String> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let node = which("node").or_else(|| {
        let mut found: Vec<PathBuf> = fs::read_dir(home.join(".nvm/versions/node")).ok()?
            .filter_map(|e| e.ok()).map(|e| e.path().join("bin/node")).filter(|p| p.is_file()).collect();
        found.sort_by_key(|p| version_key(p));
        found.pop()
    });
    if let Some(n) = node {
        return Some(format!("node:{}", n.display()));
    }
    if let Some(d) = which("deno") {
        return Some(format!("deno:{}", d.display()));
    }
    let bun = which("bun").unwrap_or_else(|| home.join(".bun/bin/bun"));
    bun.is_file().then(|| format!("bun:{}", bun.display()))
}

fn ytdlp_cmd() -> Command {
    let mut cmd = Command::new(ytdlp());
    if let Some(rt) = js_runtime() {
        cmd.args(["--js-runtimes", &rt]);
    }
    cmd.stdin(Stdio::null());
    cmd
}

/// Fetch yt-dlp if missing, otherwise self-update. YouTube breaks old
/// versions every few weeks, so this runs every time the server starts.
pub fn ensure_ytdlp() {
    let path = ytdlp();
    let _ = fs::create_dir_all(path.parent().unwrap());
    if path.exists() {
        let _ = Command::new(&path).arg("-U").stdout(Stdio::null()).stderr(Stdio::null()).status();
        return;
    }
    let url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp";
    let tmp = path.with_extension("new");
    let ok = youtube::agent().get(url).call().ok().and_then(|r| {
        let mut f = fs::File::create(&tmp).ok()?;
        std::io::copy(&mut r.into_reader(), &mut f).ok()
    });
    if ok.is_some() {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755));
        let _ = fs::rename(&tmp, &path);
    } else {
        eprintln!("couldn't fetch yt-dlp");
    }
}

fn ytdlp_json(args: &[&str]) -> Result<Value, String> {
    let out = ytdlp_cmd().args(["--flat-playlist", "-J", "--no-warnings"]).args(args)
        .output().map_err(|e| format!("yt-dlp: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.trim().lines().last().unwrap_or("yt-dlp failed").to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

fn entries(d: &Value) -> Vec<Value> {
    d["entries"].as_array().cloned().unwrap_or_default()
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn channel_of(e: &Value) -> String {
    e["channel"].as_str().or(e["uploader"].as_str()).unwrap_or("").to_string()
}

// ------------------------------------------------------------------ search (API first, yt-dlp if it fails)

pub fn search(query: &str, count: usize) -> Result<Vec<Video>, String> {
    let found = youtube::videos(query, count).or_else(|e| {
        eprintln!("search API failed, using yt-dlp: {e}");
        let d = ytdlp_json(&[&format!("ytsearch{count}:{query}")])?;
        Ok::<_, String>(entries(&d).iter().filter(|e| valid_id(e["id"].as_str().unwrap_or(""))
                                                && !s(&e["url"]).contains("/shorts/"))
            .map(|e| Video { id: s(&e["id"]), title: s(&e["title"]), channel: channel_of(e),
                             duration: e["duration"].as_f64().map(|d| d as u64),
                             live: e["live_status"] == "is_live", ..Default::default() }).collect())
    })?;
    remember(found.iter());
    Ok(found)
}

pub fn search_playlists(query: &str, count: usize) -> Result<Vec<PlaylistHit>, String> {
    youtube::playlists(query, count).or_else(|e| {
        eprintln!("playlist search API failed, using yt-dlp: {e}");
        let url = format!("https://www.youtube.com/results?search_query={}&sp=EgIQAw%3D%3D",
                          url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>());
        let d = ytdlp_json(&["--playlist-end", &count.to_string(), &url])?;
        Ok(entries(&d).iter().filter(|e| valid_list(e["id"].as_str().unwrap_or("")))
            .map(|e| PlaylistHit { id: s(&e["id"]), title: s(&e["title"]), channel: channel_of(e),
                                   thumb: s(&e["thumbnails"][0]["url"]), count: String::new() }).collect())
    })
}

fn playlists() -> &'static Mutex<HashMap<String, (Instant, Arc<Playlist>)>> {
    static P: OnceLock<Mutex<HashMap<String, (Instant, Arc<Playlist>)>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// A playlist, kept for 10 minutes so moving through it doesn't refetch.
pub fn playlist(list_id: &str) -> Result<Arc<Playlist>, String> {
    if let Some((t, pl)) = lock(playlists()).get(list_id) {
        if t.elapsed() < PLAYLIST_CACHE {
            return Ok(pl.clone());
        }
    }
    let pl = youtube::playlist(list_id).or_else(|e| {
        eprintln!("playlist API failed, using yt-dlp: {e}");
        let d = ytdlp_json(&[&format!("https://www.youtube.com/playlist?list={list_id}")])?;
        let videos = entries(&d).iter()
            .filter(|e| valid_id(e["id"].as_str().unwrap_or(""))
                    && !matches!(e["title"].as_str(), Some("[Private video]" | "[Deleted video]")))
            .map(|e| Video { id: s(&e["id"]), title: s(&e["title"]), channel: channel_of(e),
                             duration: e["duration"].as_f64().map(|d| d as u64), ..Default::default() }).collect();
        Ok::<_, String>(Playlist { id: list_id.into(), title: s(&d["title"]), channel: channel_of(&d), videos })
    })?;
    remember(pl.videos.iter());
    let pl = Arc::new(pl);
    lock(playlists()).insert(list_id.into(), (Instant::now(), pl.clone()));
    Ok(pl)
}

// ------------------------------------------------------------------ titles

fn titles() -> &'static Mutex<HashMap<String, (String, String)>> {
    static T: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// Titles seen in search results and playlists, so opening one of them
/// doesn't have to ask YouTube for it again.
pub fn remember<'a>(videos: impl Iterator<Item = &'a Video>) {
    let mut t = lock(titles());
    if t.len() > 20000 {
        t.clear();
    }
    for v in videos {
        t.insert(v.id.clone(), (v.title.clone(), v.channel.clone()));
    }
}

/// Title and channel of one video, as fast as we can know them. If YouTube's
/// details for it aren't stored yet, they're fetched (in the background when
/// the title is already known).
pub fn info(vid: &str) -> (String, String) {
    if store::details(vid).is_some() {
        return store::known_title(vid).unwrap_or_default();
    }
    let known = lock(titles()).get(vid).cloned().or_else(|| store::known_title(vid));
    if let Some(t) = known {
        let vid = vid.to_string();
        std::thread::spawn(move || fetch_details(&vid));
        return t;
    }
    if let Some(d) = fetch_details(vid) {
        return (d.title, d.channel);
    }
    youtube::oembed(vid).unwrap_or_else(|_| (vid.to_string(), String::new()))
}

fn fetch_details(vid: &str) -> Option<youtube::Details> {
    let d = youtube::details(vid).map_err(|e| eprintln!("details for {vid}: {e}")).ok()?;
    store::put_details(d.clone());
    crate::tags::kick();
    Some(d)
}

/// Fill in details for videos in history from before they were kept, a few
/// at a time, so history search covers everything.
pub fn backfill_details() {
    let mut seen = HashSet::new();
    for e in store::history() {
        if !seen.insert(e.id.clone()) || store::details(&e.id).is_some() {
            continue;
        }
        fetch_details(&e.id);
        std::thread::sleep(Duration::from_millis(700));
    }
}

// ------------------------------------------------------------------ the saved folder

/// Saved videos as names relative to the folder: files in it, and files one
/// folder down (a downloaded playlist gets its own folder).
fn saved_names() -> Vec<String> {
    let is_video = |n: &str| VIDEO_EXTS.iter().any(|e| n.ends_with(&format!(".{e}")));
    let mut names = Vec::new();
    let Ok(dir) = fs::read_dir(save_dir()) else { return names };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if path.is_dir() {
            if let Ok(sub) = fs::read_dir(&path) {
                names.extend(sub.flatten().map(|s| s.file_name().to_string_lossy().into_owned())
                    .filter(|n| is_video(n)).map(|n| format!("{name}/{n}")));
            }
        } else if is_video(&name) {
            names.push(name);
        }
    }
    names
}

/// The video ID at the end of a saved file's name: "... [dQw4w9WgXcQ].mkv"
pub fn id_in_name(name: &str) -> Option<&str> {
    let stem = &name[..name.rfind('.')?];
    let inner = stem.strip_suffix(']')?;
    let id = &inner[inner.len().checked_sub(11)?..];
    (inner[..inner.len() - 11].ends_with(" [") && valid_id(id)).then_some(id)
}

pub fn saved_file(vid: &str) -> Option<String> {
    saved_names().into_iter().find(|n| id_in_name(n) == Some(vid))
}

pub fn saved_ids() -> HashSet<String> {
    saved_names().iter().filter_map(|n| id_in_name(n).map(String::from)).collect()
}

pub struct SavedFile {
    pub name: String,
    pub size: u64,
}

pub fn saved() -> Vec<SavedFile> {
    let mut files: Vec<(std::time::SystemTime, SavedFile)> = saved_names().into_iter().filter_map(|name| {
        let m = fs::metadata(save_dir().join(&name)).ok()?;
        Some((m.modified().ok()?, SavedFile { name, size: m.len() }))
    }).collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().map(|(_, f)| f).collect()
}

/// A name from the saved list as a real path, refusing anything outside the
/// folder or more than one folder down. An empty name is the folder itself.
pub fn saved_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        let _ = fs::create_dir_all(save_dir());
        return save_dir().canonicalize().ok();
    }
    let root = save_dir().canonicalize().ok()?;
    let path = save_dir().join(name).canonicalize().ok()?;
    let rel = path.strip_prefix(&root).ok()?;
    (rel.components().count() <= 2 && path.is_file()).then_some(path)
}

/// Open the file manager with the file selected (Thunar does this through
/// the FileManager1 D-Bus call), or on the folder.
pub fn show_in_folder(path: &Path) {
    use std::os::unix::fs::MetadataExt;
    let uid = fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(1000);
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
    let bus = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_else(|_| format!("unix:path=/run/user/{uid}/bus"));
    let mut dir = path.to_path_buf();
    if path.is_file() {
        let uri = format!("file://{}", crate::util::quote_path(&path.to_string_lossy()));
        let ok = Command::new("gdbus").args(["call", "--session", "--dest", "org.freedesktop.FileManager1",
                "--object-path", "/org/freedesktop/FileManager1",
                "--method", "org.freedesktop.FileManager1.ShowItems", &format!("['{uri}']"), ""])
            .env("DISPLAY", &display).env("DBUS_SESSION_BUS_ADDRESS", &bus)
            .stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
        if ok {
            return;
        }
        dir = path.parent().map(PathBuf::from).unwrap_or(dir);
    }
    use std::os::unix::process::CommandExt;
    let _ = Command::new("xdg-open").arg(dir).env("DISPLAY", &display).env("DBUS_SESSION_BUS_ADDRESS", &bus)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0).spawn();
}

// ------------------------------------------------------------------ downloads

#[derive(Clone, Default, serde::Serialize)]
pub struct Dl {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub state: String, // queued | downloading | done | failed
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pct: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

fn downloads() -> &'static Mutex<HashMap<String, Dl>> {
    static D: OnceLock<Mutex<HashMap<String, Dl>>> = OnceLock::new();
    D.get_or_init(Default::default)
}

fn busy(d: Option<&Dl>) -> bool {
    matches!(d.map(|d| d.state.as_str()), Some("queued" | "downloading"))
}

fn set(vid: &str, f: impl FnOnce(&mut Dl)) {
    f(lock(downloads()).entry(vid.into()).or_default());
}

pub fn status(vid: &str) -> Dl {
    let st = lock(downloads()).get(vid).cloned().unwrap_or_default();
    if st.state.is_empty() && saved_file(vid).is_some() {
        return Dl { state: "done".into(), ..Default::default() };
    }
    st
}

/// How far a "download all" has got.
pub fn playlist_status(list_id: &str) -> Result<serde_json::Value, String> {
    let pl = playlist(list_id)?;
    let ids = saved_ids();
    let dl = lock(downloads());
    let busy = pl.videos.iter().filter(|v| busy(dl.get(&v.id))).count();
    let saved = pl.videos.iter().filter(|v| ids.contains(&v.id)).count();
    Ok(serde_json::json!({"total": pl.videos.len(), "saved": saved, "busy": busy}))
}

pub fn download(vid: &str) {
    {
        let mut dl = lock(downloads());
        if busy(dl.get(vid)) {
            return;
        }
        dl.insert(vid.into(), Dl { state: "downloading".into(), ..Default::default() });
    }
    let vid = vid.to_string();
    std::thread::spawn(move || run_download(&vid, &save_dir(), ""));
}

fn folder_name(title: &str) -> String {
    let name: String = title.chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '-' } else { c }).collect();
    let name = name.trim_matches(|c| c == ' ' || c == '.');
    if name.is_empty() { "playlist".into() } else { name.chars().take(120).collect() }
}

/// Save every video of a playlist, one at a time, into a folder named after
/// it, numbered in playlist order. Videos already saved are skipped.
pub fn download_playlist(list_id: &str) -> Result<usize, String> {
    let pl = playlist(list_id)?;
    let folder = save_dir().join(folder_name(&pl.title));
    let saved = saved_ids();
    let todo: Vec<(usize, String)> = {
        let mut dl = lock(downloads());
        let todo: Vec<(usize, String)> = pl.videos.iter().enumerate()
            .filter(|(_, v)| !busy(dl.get(&v.id)) && !saved.contains(&v.id))
            .map(|(i, v)| (i + 1, v.id.clone())).collect();
        for (_, vid) in &todo {
            dl.insert(vid.clone(), Dl { state: "queued".into(), ..Default::default() });
        }
        todo
    };
    let width = pl.videos.len().to_string().len();
    let n = todo.len();
    std::thread::spawn(move || {
        for (i, vid) in todo {
            set(&vid, |d| d.state = "downloading".into());
            run_download(&vid, &folder, &format!("{i:0width$} - "));
        }
    });
    Ok(n)
}

/// YouTube sometimes answers a download with a one-off 403, so try three times.
fn run_download(vid: &str, folder: &Path, prefix: &str) {
    let _ = fs::create_dir_all(folder);
    let template = folder.join(format!("{}%(title).150B [%(id)s].%(ext)s", prefix.replace('%', "%%")));
    let mut err = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(2));
        }
        match download_once(vid, &template) {
            Ok(()) => {
                set(vid, |d| { d.state = "done".into(); d.pct.clear(); });
                return;
            }
            Err(e) => err = e,
        }
    }
    set(vid, |d| { d.state = "failed".into(); d.error = err; });
}

fn download_once(vid: &str, template: &Path) -> Result<(), String> {
    let (reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
    let mut cmd = ytdlp_cmd();
    cmd.args(["-f", FORMAT, "--merge-output-format", "mkv", "--no-playlist", "--embed-metadata",
              "--newline", "--progress-template", "download:CTPROG %(progress._percent_str)s", "-o"])
        .arg(template).arg(watch_url(vid))
        .stdout(writer.try_clone().map_err(|e| e.to_string())?).stderr(writer);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    drop(cmd); // closes our copy of the pipe's write end, so reading ends with yt-dlp
    let mut last_err = String::new();
    for line in BufReader::new(reader).split(b'\n').map_while(Result::ok) {
        let line = String::from_utf8_lossy(&line);
        let line = line.trim();
        if let Some(pct) = line.strip_prefix("CTPROG") {
            let pct = pct.trim().to_string();
            set(vid, |d| d.pct = pct);
        } else if line.starts_with("ERROR") {
            last_err = line.to_string();
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        return Ok(());
    }
    Err(if last_err.is_empty() { format!("yt-dlp exited {status}") } else { last_err })
}
