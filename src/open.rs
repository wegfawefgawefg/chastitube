//! The link handler: `chastitube open URL...`. Every http(s) link you click
//! in another app comes through here. YouTube videos and playlists open in
//! chastitube (in the chastitube tab that's on screen, if there is one);
//! everything else goes to Firefox unchanged.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::links::{classify, Link};
use crate::util::{base, port, urlencode};

fn addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port()))
}

fn server_up() -> bool {
    TcpStream::connect_timeout(&addr(), Duration::from_millis(300)).is_ok()
}

fn ensure_server() {
    if server_up() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("serve").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .process_group(0).spawn();
    }
    for _ in 0..30 {
        if server_up() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

const LAUNCHERS: &[&str] = &["python3", "python", "xdg-open", "sh", "bash", "dash", "env", "gio", "exo-open",
                             "kioclient5", "gnome-open", "nohup", "setsid", "chastitube"];

/// Name of the app that opened the link (Discord, ...), found by walking up
/// the process tree past xdg-open and shells.
fn calling_app() -> String {
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..12 {
        let Ok(name) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else { return String::new() };
        let name = name.trim().to_string();
        let ppid = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("PPid:").and_then(|p| p.trim().parse::<u32>().ok())));
        if !LAUNCHERS.contains(&name.as_str()) {
            return if name == "systemd" || name == "init" { String::new() } else { name };
        }
        match ppid {
            Some(p) if p > 1 => pid = p,
            _ => return String::new(),
        }
    }
    String::new()
}

fn browser(urls: &[String]) {
    let browser = std::env::var("CHASTITUBE_BROWSER").unwrap_or_else(|_| "firefox".into());
    let _ = Command::new(browser).args(urls).stdin(Stdio::null()).process_group(0).spawn();
}

/// Send path to a chastitube tab that's on screen and raise its window.
/// False if there's no such tab.
fn open_in_tab(path: &str) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&addr(), Duration::from_secs(2)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let req = format!("POST /api/goto HTTP/1.1\r\nHost: localhost:{}\r\nOrigin: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{path}",
                      port(), base(), path.len());
    let mut resp = String::new();
    if s.write_all(req.as_bytes()).is_err() || s.read_to_string(&mut resp).is_err() && resp.is_empty() {
        return false;
    }
    let body = resp.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    let opened = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v["opened"].as_bool()).unwrap_or(false);
    if opened {
        // page titles end in "chastitube", and Firefox adds " — Mozilla Firefox"
        let _ = Command::new("i3-msg").args(["-q", r#"[class="(?i)firefox" title="chastitube — Mozilla Firefox$"] focus"#])
            .stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    opened
}

/// i3: focus the Firefox window showing a chastitube page. False if there isn't one.
fn focus_tab() -> bool {
    let out = Command::new("i3-msg").arg(r#"[class="(?i)firefox" title="chastitube — Mozilla Firefox$"] focus"#)
        .stderr(Stdio::null()).output();
    matches!(out, Ok(o) if String::from_utf8_lossy(&o.stdout).contains("\"success\":true"))
}

/// `chastitube` with nothing after it (or `chube` from dmenu): start the server
/// if needed and show the page, reusing a chastitube tab that's on screen.
pub fn open_app() {
    ensure_server();
    if focus_tab() {
        return;
    }
    browser(&[base() + "/"]);
    // Firefox opens the tab in its existing window; bring that window forward once the tab is there
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(150));
        if focus_tab() {
            return;
        }
    }
}

pub fn open_links(args: &[String]) {
    if args.is_empty() {
        return browser(&[]);
    }
    let mut to_browser = Vec::new();
    let app = calling_app();
    for arg in args {
        let path = match classify(arg) {
            None => {
                to_browser.push(arg.clone());
                continue;
            }
            Some(Link::Video { id, start, list }) => {
                let start = start.map(|s| s.to_string());
                let mut params = vec![("from", "link"), ("app", app.as_str())];
                if let Some(s) = &start {
                    params.push(("t", s));
                }
                if let Some(l) = &list {
                    params.push(("list", l));
                }
                format!("/v/{id}?{}", urlencode(&params))
            }
            Some(Link::Playlist(list)) => format!("/p/{list}?{}", urlencode(&[("from", "link"), ("app", &app)])),
            Some(Link::Search(q)) => format!("/?{}", urlencode(&[("q", &q)])),
            Some(Link::Channel(c)) => format!("/c/{}", crate::util::quote_path(&c)),
            Some(Link::YouTube) => format!("/?{}", urlencode(&[("q", arg)])),
        };
        ensure_server();
        if !open_in_tab(&path) {
            to_browser.push(base() + &path);
        }
    }
    if !to_browser.is_empty() {
        browser(&to_browser);
    }
}
