//! Channel pages: /c/<channel id>. A channel's own uploads from its Videos,
//! Shorts and Live tabs, sorted the ways YouTube offers (latest, popular,
//! oldest), and its playlists. Nothing YouTube picks for you: the Home tab
//! ("featured") and related channels are never fetched.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::pages::{dl_button, duration, join, page, progress, row, thumb};
use crate::store::Pos;
use crate::util::{esc, lock, urlencode};
use crate::youtube::{found, lockup_badge, next_token, playlist_hit, post, s, seconds, short_lockup, PlaylistHit, Video};

const VIDEOS_TAB: &str = "EgZ2aWRlb3PyBgQKAjoA";
const PLAYLISTS_TAB: &str = "EglwbGF5bGlzdHPyBgQKAkIA";
const CACHE: Duration = Duration::from_secs(600);
pub const SORTS: [&str; 3] = ["latest", "popular", "oldest"];
/// The tabs we show, by their lowercased tab title.
pub const KINDS: [&str; 3] = ["videos", "shorts", "live"];

pub struct Channel {
    pub id: String,
    pub title: String,
    pub handle: String,
    pub avatar: String,
    /// "8.78M subscribers · 250 videos"
    pub facts: String,
    pub about: String,
    pub playlists: Vec<PlaylistHit>,
    /// The kinds this channel has ("videos", "shorts", "live"), with the
    /// params that open that tab, in KINDS order.
    pub kinds: Vec<(String, String)>,
}

/// Videos fetched so far in one order, and where the next page starts.
#[derive(Clone, Default)]
struct Listing {
    videos: Vec<Video>,
    next: Option<String>,
}

/// One tab (videos, shorts or live): its sort buttons and its first page, newest first.
#[derive(Clone)]
struct Tab {
    /// "latest" / "popular" / "oldest" -> the token that asks for that order
    sorts: HashMap<String, String>,
    latest: Listing,
}

type Cache<T> = Mutex<HashMap<String, (Instant, T)>>;

fn channels() -> &'static Cache<Arc<Channel>> {
    static C: OnceLock<Cache<Arc<Channel>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn tabs() -> &'static Cache<Tab> {
    static T: OnceLock<Cache<Tab>> = OnceLock::new();
    T.get_or_init(Default::default)
}

fn listings() -> &'static Cache<Listing> {
    static L: OnceLock<Cache<Listing>> = OnceLock::new();
    L.get_or_init(Default::default)
}

fn cached<T: Clone>(cache: &Cache<T>, key: &str) -> Option<T> {
    lock(cache).get(key).filter(|(t, _)| t.elapsed() < CACHE).map(|(_, v)| v.clone())
}

pub fn valid_channel_id(id: &str) -> bool {
    id.len() == 24 && id.starts_with("UC") && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// "@savannahxyz", "c/name" or "user/name" (as in a youtube.com link) -> "UC..."
pub fn resolve(path: &str) -> Result<String, String> {
    if valid_channel_id(path) {
        return Ok(path.to_string());
    }
    let d = post("navigation/resolve_url", json!({"url": format!("https://www.youtube.com/{path}")}))?;
    let id = s(&d["endpoint"]["browseEndpoint"]["browseId"]);
    if valid_channel_id(&id) { Ok(id) } else { Err("YouTube doesn't know that channel".into()) }
}

/// The uploads on one page of a tab (its first page or a continuation): normal
/// videos and streams come as "lockups", Shorts as "shorts lockups".
fn uploads(d: &Value, ch_title: &str, ch_id: &str) -> Vec<Video> {
    let mut out: Vec<Video> = Vec::new();
    for lk in found(d, "lockupViewModel") {
        if lk["contentType"] != "LOCKUP_CONTENT_TYPE_VIDEO" {
            continue;
        }
        let md = &lk["metadata"]["lockupMetadataViewModel"];
        let info: Vec<String> = found(md, "metadataParts").iter()
            .flat_map(|parts| parts.as_array().into_iter().flatten())
            .map(|p| s(&p["text"]["content"])).filter(|t| !t.is_empty()).collect();
        out.push(Video {
            id: s(&lk["contentId"]),
            title: s(&md["title"]["content"]),
            duration: seconds(&lockup_badge(lk)),
            info: info.join(" · "),
            ..Default::default()
        });
    }
    out.extend(found(d, "shortsLockupViewModel").into_iter().filter_map(short_lockup));
    out.retain(|v| crate::util::valid_id(&v.id));
    for v in out.iter_mut() {
        if v.title.is_empty() {
            v.title = v.id.clone();
        }
        v.channel = ch_title.to_string();
        v.channel_id = ch_id.to_string();
    }
    out
}

/// A tab's sort buttons and first page, if the response really is that tab.
/// (Asking for a tab a channel doesn't have gets its Home tab instead.)
fn read_tab(d: &Value, kind: &str, ch_title: &str, ch_id: &str) -> Option<Tab> {
    let selected = found(d, "tabRenderer").into_iter().find(|t| t["selected"] == true)?;
    if s(&selected["title"]).to_lowercase() != kind {
        return None;
    }
    let mut sorts = HashMap::new();
    for chip in found(d, "chipViewModel") {
        if let Some(tok) = found(chip, "token").first() {
            sorts.insert(s(&chip["text"]).to_lowercase(), s(tok));
        }
    }
    Some(Tab { sorts, latest: Listing { videos: uploads(d, ch_title, ch_id), next: next_token(d) } })
}

/// The channel's header, which tabs it has, its playlists, and its Videos tab.
fn fetch(id: &str) -> Result<(Channel, Option<Tab>), String> {
    let (videos_tab, playlists_tab) = std::thread::scope(|sc| {
        let pl = sc.spawn(|| post("browse", json!({"browseId": id, "params": PLAYLISTS_TAB})));
        (post("browse", json!({"browseId": id, "params": VIDEOS_TAB})), pl.join().unwrap_or_else(|_| Err("crashed".into())))
    });
    let d = videos_tab?;
    let md = &d["metadata"]["channelMetadataRenderer"];
    let title = s(&md["title"]);
    if title.is_empty() {
        return Err("YouTube didn't return that channel".into());
    }
    let handle = s(&md["vanityChannelUrl"]).rsplit('/').next().filter(|h| h.starts_with('@')).unwrap_or("").to_string();
    let avatar = found(&md["avatar"], "url").first().map(|u| s(u)).unwrap_or_default();
    let header = found(&d, "pageHeaderViewModel").first().copied().unwrap_or(&Value::Null);
    let facts: Vec<String> = found(header, "content").iter().filter_map(|c| c.as_str())
        .filter(|c| c.contains("subscriber") || c.ends_with("videos") || c.ends_with("video"))
        .map(String::from).collect();
    let mut kinds = Vec::new();
    for kind in KINDS {
        let tab = found(&d, "tabRenderer").into_iter().find(|t| s(&t["title"]).to_lowercase() == kind);
        if let Some(params) = tab.map(|t| s(&t["endpoint"]["browseEndpoint"]["params"])).filter(|p| !p.is_empty()) {
            kinds.push((kind.to_string(), params));
        }
    }
    let playlists = playlists_tab.map(|p| found(&p, "lockupViewModel").into_iter().filter_map(playlist_hit).collect()).unwrap_or_default();
    let videos = read_tab(&d, "videos", &title, id);
    let ch = Channel { id: id.to_string(), title, handle, avatar, facts: facts.join(" · "), about: s(&md["description"]), playlists, kinds };
    Ok((ch, videos))
}

pub fn channel(id: &str) -> Result<Arc<Channel>, String> {
    if let Some(ch) = cached(channels(), id) {
        return Ok(ch);
    }
    let (ch, videos) = fetch(id)?;
    let ch = Arc::new(ch);
    lock(channels()).insert(id.to_string(), (Instant::now(), ch.clone()));
    if let Some(t) = videos {
        lock(tabs()).insert(format!("{id} videos"), (Instant::now(), t));
    }
    Ok(ch)
}

fn tab(ch: &Channel, kind: &str) -> Result<Tab, String> {
    let key = format!("{} {kind}", ch.id);
    if let Some(t) = cached(tabs(), &key) {
        return Ok(t);
    }
    let params = ch.kinds.iter().find(|(k, _)| k == kind).map(|(_, p)| p).ok_or("this channel has no such tab")?;
    let d = post("browse", json!({"browseId": ch.id, "params": params}))?;
    let t = read_tab(&d, kind, &ch.title, &ch.id).ok_or("YouTube didn't return that tab")?;
    lock(tabs()).insert(key, (Instant::now(), t.clone()));
    Ok(t)
}

/// At least `want` uploads of this kind in this order (fewer if there aren't
/// that many), and whether there are more.
pub fn videos(ch: &Channel, kind: &str, sort: &str, want: usize) -> Result<(Vec<Video>, bool), String> {
    let key = format!("{} {kind} {sort}", ch.id);
    let mut listing = match cached(listings(), &key) {
        Some(l) => l,
        None => {
            let t = tab(ch, kind)?;
            if sort == "latest" {
                t.latest
            } else {
                let tok = t.sorts.get(sort).ok_or("YouTube doesn't offer that order here")?;
                let d = post("browse", json!({"continuation": tok}))?;
                Listing { videos: uploads(&d, &ch.title, &ch.id), next: next_token(&d) }
            }
        }
    };
    while listing.videos.len() < want {
        let Some(tok) = listing.next.take() else { break };
        let d = post("browse", json!({"continuation": tok}))?;
        let more = uploads(&d, &ch.title, &ch.id);
        if more.is_empty() {
            break;
        }
        for v in more {
            if !listing.videos.iter().any(|o| o.id == v.id) {
                listing.videos.push(v);
            }
        }
        listing.next = next_token(&d);
    }
    let has_more = listing.next.is_some();
    lock(listings()).insert(key, (Instant::now(), listing.clone()));
    let mut out = listing.videos;
    out.truncate(want);
    Ok((out, has_more))
}

/// "videos · shorts · live" or "latest · popular · oldest", the current one bold.
fn switch(ch: &Channel, options: &[&str], current: &str, link: impl Fn(&str) -> String) -> String {
    let opts: Vec<String> = options.iter().map(|o| {
        if *o == current { format!("<b>{o}</b>") } else { format!(r#"<a href="/c/{}?{}">{o}</a>"#, ch.id, esc(&link(o))) }
    }).collect();
    opts.join(" · ")
}

pub fn channel_page(ch: &Channel, kind: &str, sort: &str, videos: &[Video], has_more: bool, count: usize,
                    positions: &HashMap<String, Pos>) -> String {
    let about = ch.about.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let about = if about.chars().count() > 220 { format!("{}...", about.chars().take(220).collect::<String>()) } else { about.to_string() };
    let avatar = if ch.avatar.is_empty() { String::new() } else { format!(r#"<img class="avatar" src="{}" alt="">"#, esc(&ch.avatar)) };
    let mut parts = vec![crate::pages::search_box("", false), format!(
        r#"<div class="chead">{avatar}<div><h1>{}</h1><div class="meta">{}</div><div class="note">{}</div></div></div>"#,
        esc(&ch.title), join(&[esc(&ch.handle), esc(&ch.facts)]), esc(&about))];

    let kinds: Vec<&str> = ch.kinds.iter().map(|(k, _)| k.as_str()).collect();
    let kind_links = switch(ch, &kinds, kind, |k| urlencode(&[("kind", k), ("sort", sort)]));
    let sort_links = switch(ch, &SORTS, sort, |s| urlencode(&[("kind", kind), ("sort", s)]));
    let mut left = vec![format!(r#"<h2 class="col">{kind_links} &nbsp;|&nbsp; {sort_links}</h2>"#)];
    let from = urlencode(&[("from", "channel"), ("q", ch.title.as_str())]);
    for v in videos {
        let href = format!("/v/{}?{from}", v.id);
        let meta = join(&[esc(&v.info), duration(v.duration), progress(positions.get(&v.id))]);
        left.push(row(&thumb(&v.id, &href, None), &href, &v.title, &meta, &dl_button(&v.id, "")));
    }
    if videos.is_empty() {
        left.push(format!(r#"<p class="note">No {kind} here.</p>"#));
    } else if has_more {
        let more = urlencode(&[("kind", kind), ("sort", sort), ("n", &(count + 30).to_string())]);
        left.push(format!(r#"<p><a href="/c/{}?{}">more</a></p>"#, ch.id, esc(&more)));
    }

    let mut right = vec![r#"<h2 class="col">Playlists</h2>"#.to_string()];
    if ch.playlists.is_empty() {
        right.push(r#"<p class="note">No playlists.</p>"#.into());
    }
    for p in &ch.playlists {
        let href = format!("/p/{}?{from}", p.id);
        let meta = esc(if p.count.is_empty() { "playlist" } else { &p.count });
        let th = if p.thumb.is_empty() { r#"<div class="thumb"></div>"#.into() } else { thumb(&p.id, &href, Some(&p.thumb)) };
        right.push(row(&th, &href, &p.title, &meta, ""));
    }
    parts.push(format!(r#"<div class="split"><div>{}</div><div>{}</div></div>"#, left.concat(), right.concat()));
    page(&format!("{} - chastitube", ch.title), &parts.concat(), "", "wide")
}
