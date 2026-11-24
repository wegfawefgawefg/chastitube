//! The local web server on http://localhost:8790.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use tiny_http::{Header, Request, Response, StatusCode};

use crate::links::{classify, parse_time, Link};
use crate::util::{app_dir, base, esc, own_origins, port, qget, save_dir, urlencode, valid_id, valid_list, NAME_IP};
use crate::{channel, pages, player, store, tabs, tags, yt};

const NOT_OPENED: &str = "chastitube opens videos, playlists and channels. Mixes, Shorts and the home page stay shut.";

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("header")
}

fn send(req: Request, code: u16, body: impl Into<Vec<u8>>, ctype: &str) {
    let body = body.into();
    let len = body.len();
    let resp = Response::new(StatusCode(code), vec![header("Content-Type", ctype), header("Cache-Control", "no-store")],
                             std::io::Cursor::new(body), Some(len), None);
    let _ = req.respond(resp);
}

fn html(req: Request, code: u16, body: String) {
    send(req, code, body, "text/html; charset=utf-8");
}

fn text(req: Request, code: u16, body: &str) {
    send(req, code, body.to_string(), "text/plain; charset=utf-8");
}

fn json(req: Request, v: serde_json::Value) {
    send(req, 200, v.to_string(), "application/json");
}

fn redirect(req: Request, to: &str) {
    let _ = req.respond(Response::empty(StatusCode(303)).with_header(header("Location", to)));
}

/// A file from disk, with Range support so the player can seek.
fn send_file(req: Request, path: &Path, cache: bool) {
    let Ok(mut f) = File::open(path) else { return text(req, 404, "no such file") };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let ctype = match ext {
        "mkv" => "video/x-matroska", "mp4" => "video/mp4", "webm" => "video/webm",
        "m4a" => "audio/mp4", "opus" => "audio/ogg", "png" => "image/png",
        _ => "application/octet-stream",
    };
    let range = req.headers().iter().find(|h| h.field.equiv("Range")).map(|h| h.value.as_str().to_string());
    let (mut start, mut end, mut code) = (0u64, size.saturating_sub(1), 200);
    if let Some(r) = range.as_deref().and_then(|r| r.strip_prefix("bytes=")) {
        let (a, b) = r.split(',').next().unwrap_or("").split_once('-').unwrap_or(("", ""));
        let parsed = if !a.is_empty() {
            a.parse::<u64>().ok().map(|a| (a, if b.is_empty() { size.saturating_sub(1) } else { b.parse::<u64>().map(|b| b.min(size.saturating_sub(1))).unwrap_or(u64::MAX) }))
        } else {
            b.parse::<u64>().ok().map(|b| (size.saturating_sub(b), size.saturating_sub(1)))
        };
        if let Some((s, e)) = parsed.filter(|(_, e)| *e != u64::MAX) {
            if s > e || s >= size {
                let _ = req.respond(Response::empty(StatusCode(416)).with_header(header("Content-Range", &format!("bytes */{size}"))));
                return;
            }
            (start, end, code) = (s, e, 206);
        }
    }
    let len = if size == 0 { 0 } else { end - start + 1 };
    if f.seek(SeekFrom::Start(start)).is_err() {
        return text(req, 500, "seek failed");
    }
    let mut headers = vec![header("Content-Type", ctype), header("Accept-Ranges", "bytes")];
    if code == 206 {
        headers.push(header("Content-Range", &format!("bytes {start}-{end}/{size}")));
    }
    if cache {
        headers.push(header("Cache-Control", "max-age=86400"));
    }
    let _ = req.respond(Response::new(StatusCode(code), headers, f.take(len), Some(len as usize), None));
}

pub fn serve() {
    let server = match tiny_http::Server::http(("127.0.0.1", port())) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("can't listen on port {}: {e}", port());
            std::process::exit(1);
        }
    };
    // http://chastitube: port 80 on chastitube's own loopback address, if ./install.sh --name set that up
    if port() == 8790 {
        match tiny_http::Server::http(std::net::SocketAddr::from((NAME_IP, 80))) {
            Ok(named) => {
                std::thread::spawn(move || {
                    for req in named.incoming_requests() {
                        std::thread::spawn(move || handle(req));
                    }
                });
            }
            Err(e) => eprintln!("http://chastitube isn't available ({e}); ./install.sh --name sets it up"),
        }
    }
    store::start();
    std::thread::spawn(|| {
        yt::ensure_ytdlp();
        yt::backfill_details();
        tags::kick();
    });
    println!("chastitube on {}", base());
    for req in server.incoming_requests() {
        std::thread::spawn(move || handle(req));
    }
}

fn handle(req: Request) {
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let q = crate::util::parse_query(query);
    if *req.method() == tiny_http::Method::Post {
        return post(req, &parts);
    }
    match parts.as_slice() {
        [] => search(req, &q),
        ["p", list] if valid_list(list) => open_playlist(req, list, &q),
        ["v", vid] if valid_id(vid) => video(req, vid, &q),
        ["c", ..] if parts.len() >= 2 => {
            let key = percent_encoding::percent_decode_str(&parts[1..].join("/")).decode_utf8_lossy().into_owned();
            channel_route(req, &key, &q)
        }
        ["file", ..] if parts.len() >= 2 => {
            let name = percent_encoding::percent_decode_str(&parts[1..].join("/")).decode_utf8_lossy().into_owned();
            match yt::saved_path(&name) {
                Some(p) => send_file(req, &p, false),
                None => text(req, 404, "no such file"),
            }
        }
        ["icon", name] if icon_name(name) => send_file(req, &app_dir().join("art/renders").join(format!("cage_{name}")), true),
        ["history"] => html(req, 200, pages::history_page(&store::history(), &store::positions(), &store::all_details(),
                                                          &tags::view(), qget(&q, "q").unwrap_or("").trim(),
                                                          qget(&q, "tag").unwrap_or("").trim())),
        ["searches"] => html(req, 200, pages::searches_page(&store::searches(), &store::history())),
        ["time"] => html(req, 200, pages::time_page(&store::watch(), &titles(), &tags::view())),
        ["saved"] => html(req, 200, pages::saved_page(&yt::saved(), &save_dir().to_string_lossy())),
        ["api", "status", vid] if valid_id(vid) => json(req, serde_json::to_value(yt::status(vid)).unwrap_or_default()),
        ["api", "list-status", list] if valid_list(list) => match yt::playlist_status(list) {
            Ok(v) => json(req, v),
            Err(e) => text(req, 500, &e),
        },
        ["api", "events"] => tabs::stream(req.into_writer()),
        _ => html(req, 404, pages::message_page("Not here.")),
    }
}

fn icon_name(name: &str) -> bool {
    let Some((color, size)) = name.strip_suffix(".png").and_then(|n| n.split_once('_')) else { return false };
    ["rust", "black", "zinc", "enamel"].contains(&color) && ["512", "256", "128", "64", "48", "32"].contains(&size)
}

/// The address this page was opened under (http://chastitube or http://localhost:8790);
/// the YouTube player needs it to talk to the page.
fn page_origin(req: &Request) -> String {
    let host = req.headers().iter().find(|h| h.field.equiv("Host")).map(|h| format!("http://{}", h.value.as_str()));
    host.filter(|h| own_origins().contains(h)).unwrap_or_else(base)
}

/// Title and channel for every video we know, for the time page.
fn titles() -> std::collections::HashMap<String, (String, String)> {
    let mut t = std::collections::HashMap::new();
    for e in store::history().into_iter().rev() {
        t.insert(e.id, (e.title, e.channel));
    }
    for (id, d) in store::all_details() {
        t.insert(id, (d.title, d.channel));
    }
    t
}

fn search(req: Request, q: &[(String, String)]) {
    let Some(raw) = qget(q, "q").map(str::trim).filter(|s| !s.is_empty()) else {
        return html(req, 200, pages::search_page("", None, None, 20, &Default::default(), &[], None));
    };
    let mut query = raw.to_string();
    match classify(raw) {
        Some(Link::Video { id, start, list }) => {
            let start = start.map(|s| s.to_string());
            let mut params = vec![("from", "paste")];
            if let Some(s) = &start {
                params.push(("t", s));
            }
            if let Some(l) = &list {
                params.push(("list", l));
            }
            return redirect(req, &format!("/v/{id}?{}", urlencode(&params)));
        }
        Some(Link::Playlist(list)) => return redirect(req, &format!("/p/{list}?from=paste")),
        Some(Link::Search(s)) => query = s,
        Some(Link::Channel(c)) => return redirect(req, &format!("/c/{}", crate::util::quote_path(&c))),
        Some(Link::YouTube) => return html(req, 200, pages::message_page(NOT_OPENED)),
        None => {}
    }
    let count = qget(q, "n").and_then(|n| n.parse::<usize>().ok()).unwrap_or(20).clamp(10, 60);
    if qget(q, "n").is_none() {
        store::add_search(&query);
    }
    // videos and playlists are two separate searches: run them side by side
    let (videos, lists) = std::thread::scope(|s| {
        let lists = s.spawn(|| yt::search_playlists(&query, count / 2));
        let videos = yt::search(&query, count);
        (videos, lists.join().unwrap_or_else(|_| Err("crashed".into())))
    });
    let positions = store::positions();
    let page = match videos {
        Err(e) => pages::search_page(&query, None, Some(&format!("Search failed: {e}")), count, &positions, &[], None),
        Ok(v) => {
            let (pls, pl_err) = match lists {
                Ok(p) => (p, None),
                Err(e) => (Vec::new(), Some(format!("Playlist search failed: {e}"))),
            };
            pages::search_page(&query, Some(&v), None, count, &positions, &pls, pl_err.as_deref())
        }
    };
    html(req, 200, page);
}

/// /c/UC... shows the channel; /c/@handle (or c/name, user/name) finds its id first.
fn channel_route(req: Request, key: &str, q: &[(String, String)]) {
    if !channel::valid_channel_id(key) {
        return match channel::resolve(key) {
            Ok(id) => redirect(req, &format!("/c/{id}")),
            Err(e) => html(req, 200, pages::message_page(&format!("Couldn't find that channel: {e}"))),
        };
    }
    let ch = match channel::channel(key) {
        Ok(ch) => ch,
        Err(e) => return html(req, 200, pages::message_page(&format!("Couldn't open that channel: {e}"))),
    };
    let sort = qget(q, "sort").filter(|s| channel::SORTS.contains(s)).unwrap_or("latest");
    // the asked-for kind if the channel has it, else its first (normally "videos")
    let kind = qget(q, "kind").filter(|k| ch.kinds.iter().any(|(c, _)| c == k))
        .or(ch.kinds.first().map(|(k, _)| k.as_str())).unwrap_or("videos").to_string();
    let count = qget(q, "n").and_then(|n| n.parse::<usize>().ok()).unwrap_or(30).clamp(30, 600);
    match channel::videos(&ch, &kind, sort, count) {
        Ok((videos, more)) => {
            yt::remember(videos.iter());
            html(req, 200, channel::channel_page(&ch, &kind, sort, &videos, more, count, &store::positions()))
        }
        Err(e) => html(req, 200, pages::message_page(&format!("Couldn't list that channel's videos: {e}"))),
    }
}

/// A playlist opens on its first video you haven't finished.
fn open_playlist(req: Request, list: &str, q: &[(String, String)]) {
    let pl = match yt::playlist(list) {
        Ok(pl) => pl,
        Err(e) => return html(req, 200, pages::message_page(&format!("Couldn't open that playlist: {e}"))),
    };
    let Some(first) = ({
        let pos = store::positions();
        pl.videos.iter().find(|v| !store::finished(pos.get(&v.id))).or(pl.videos.first())
    }) else {
        return html(req, 200, pages::message_page("That playlist is empty."));
    };
    let mut params = vec![("list", list), ("from", qget(q, "from").unwrap_or("direct"))];
    for key in ["q", "app"] {
        if let Some(v) = qget(q, key) {
            params.push((key, v));
        }
    }
    redirect(req, &format!("/v/{}?{}", first.id, urlencode(&params)));
}

fn video(req: Request, vid: &str, q: &[(String, String)]) {
    // the playlist first: fetching it also teaches us the video's title
    let pl = qget(q, "list").filter(|l| valid_list(l))
        .and_then(|l| yt::playlist(l).ok())
        .filter(|pl| pl.videos.iter().any(|v| v.id == vid));
    let (title, channel) = yt::info(vid);
    let positions = store::positions();
    let mut start = parse_time(qget(q, "t"));
    let mut resumed = 0;
    if start.is_none() && qget(q, "restart").is_none() {
        resumed = store::resume_point(positions.get(vid));
        start = Some(resumed);
    }
    let source = qget(q, "from").filter(|s| ["search", "link", "paste", "history", "saved", "playlist", "channel"].contains(s)).unwrap_or("direct");
    let detail = qget(q, "q").or(qget(q, "app")).or(qget(q, "pl")).unwrap_or("");
    store::add_history(vid, &title, &channel, source, &crate::util::truncate(detail, 200));
    let saved_name = yt::saved_file(vid);
    let saved = if pl.is_some() { yt::saved_ids() } else { Default::default() };
    let origin = page_origin(&req);
    let channel_id = store::details(vid).map(|d| d.channel_id).unwrap_or_default();
    html(req, 200, player::video_page(&player::VideoPage {
        vid, title: &title, channel: &channel, channel_id: &channel_id, start: start.unwrap_or(0), origin: &origin, resumed,
        saved_name: saved_name.as_deref(), pl: pl.as_deref(), positions: &positions, saved: &saved,
    }));
}

fn post(mut req: Request, parts: &[&str]) {
    // Only our own page may press buttons; other websites can't reach in.
    let origin = req.headers().iter().find(|h| h.field.equiv("Origin")).map(|h| h.value.as_str().to_string());
    if !origin.is_some_and(|o| own_origins().contains(&o)) {
        return text(req, 403, "forbidden");
    }
    let mut body = String::new();
    let _ = req.as_reader().take(1 << 16).read_to_string(&mut body);
    match parts {
        ["api", "download", vid] if valid_id(vid) => {
            yt::download(vid);
            json(req, serde_json::json!({"ok": true}))
        }
        ["api", "download-list", list] if valid_list(list) => match yt::download_playlist(list) {
            Ok(n) => json(req, serde_json::json!({"queued": n})),
            Err(e) => text(req, 500, &e),
        },
        ["api", "pos", vid] if valid_id(vid) => {
            let d: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            match (d["pos"].as_f64(), d["dur"].as_f64()) {
                (Some(pos), Some(dur)) => {
                    store::set_position(vid, pos, dur, d["played"].as_f64().unwrap_or(0.0));
                    json(req, serde_json::json!({"ok": true}))
                }
                _ => text(req, 400, "bad position"),
            }
        }
        ["api", "tag", vid] if valid_id(vid) => {
            let d: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            match (d["tag"].as_str().map(str::trim).filter(|t| !t.is_empty() && t.len() <= 60), d["on"].as_bool()) {
                (Some(tag), Some(on)) => {
                    tags::fix(vid, tag, on);
                    json(req, serde_json::json!({"ok": true}))
                }
                _ => text(req, 400, "bad tag"),
            }
        }
        ["api", "goto"] => {
            if !body.starts_with('/') || body.starts_with("//") {
                return text(req, 400, "bad path");
            }
            json(req, serde_json::json!({"opened": tabs::send(&body)}))
        }
        ["api", "ack", n] => match n.parse() {
            Ok(n) => json(req, serde_json::json!({"ok": tabs::ack(n)})),
            Err(_) => text(req, 400, "bad ack"),
        },
        ["api", "showfile"] => match yt::saved_path(&body) {
            Some(p) => {
                yt::show_in_folder(&p);
                json(req, serde_json::json!({"ok": true}))
            }
            None => text(req, 404, "no such file"),
        },
        _ => text(req, 404, &esc("not found")),
    }
}
