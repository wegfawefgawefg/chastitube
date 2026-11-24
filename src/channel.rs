//! Channel pages: /c/<channel id>. A channel's own videos (its Videos tab,
//! so no Shorts), sorted the ways YouTube offers (latest, popular, oldest),
//! and its playlists. Nothing YouTube picks for you: no home tab, no
//! "featured", no related channels.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::pages::{dl_button, duration, join, page, progress, row, thumb};
use crate::store::Pos;
use crate::util::{esc, lock, urlencode};
use crate::youtube::{found, lockup_badge, next_token, playlist_hit, post, s, seconds, PlaylistHit, Video};

const VIDEOS_TAB: &str = "EgZ2aWRlb3PyBgQKAjoA";
const PLAYLISTS_TAB: &str = "EglwbGF5bGlzdHPyBgQKAkIA";
const CACHE: Duration = Duration::from_secs(600);
pub const SORTS: [&str; 3] = ["latest", "popular", "oldest"];

pub struct Channel {
    pub id: String,
    pub title: String,
    pub handle: String,
    pub avatar: String,
    /// "8.78M subscribers · 250 videos"
    pub facts: String,
    pub about: String,
    pub playlists: Vec<PlaylistHit>,
    /// YouTube's sort buttons: name ("latest"...) -> the token that asks for that order
    sorts: HashMap<String, String>,
}

/// Videos fetched so far in one order, and where the next page starts.
#[derive(Clone, Default)]
struct Listing {
    videos: Vec<Video>,
    next: Option<String>,
}

type Cache<T> = Mutex<HashMap<String, (Instant, T)>>;

fn channels() -> &'static Cache<Arc<Channel>> {
    static C: OnceLock<Cache<Arc<Channel>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn listings() -> &'static Cache<Listing> {
    static L: OnceLock<Cache<Listing>> = OnceLock::new();
    L.get_or_init(Default::default)
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

/// The videos in one page of a channel's Videos tab (first page or a continuation).
fn video_lockups(d: &Value, channel: &str, channel_id: &str) -> Vec<Video> {
    found(d, "lockupViewModel").into_iter().filter(|lk| lk["contentType"] == "LOCKUP_CONTENT_TYPE_VIDEO").map(|lk| {
        let md = &lk["metadata"]["lockupMetadataViewModel"];
        let info: Vec<String> = found(md, "metadataParts").iter()
            .flat_map(|parts| parts.as_array().into_iter().flatten())
            .map(|p| s(&p["text"]["content"])).filter(|t| !t.is_empty()).collect();
        let id = s(&lk["contentId"]);
        let title = s(&md["title"]["content"]);
        Video {
            title: if title.is_empty() { id.clone() } else { title },
            channel: channel.to_string(),
            channel_id: channel_id.to_string(),
            duration: seconds(&lockup_badge(lk)),
            info: info.join(" · "),
            id,
            ..Default::default()
        }
    }).collect()
}

/// The channel's header, its playlists, and the first page of its latest videos.
fn fetch(id: &str) -> Result<(Channel, Listing), String> {
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
    let mut sorts = HashMap::new();
    for chip in found(&d, "chipViewModel") {
        let name = s(&chip["text"]).to_lowercase();
        if let Some(tok) = found(chip, "token").first() {
            sorts.insert(name, s(tok));
        }
    }
    let playlists = playlists_tab.map(|p| found(&p, "lockupViewModel").into_iter().filter_map(playlist_hit).collect()).unwrap_or_default();
    let listing = Listing { videos: video_lockups(&d, &title, id), next: next_token(&d) };
    let ch = Channel { id: id.to_string(), title, handle, avatar, facts: facts.join(" · "), about: s(&md["description"]), playlists, sorts };
    Ok((ch, listing))
}

pub fn channel(id: &str) -> Result<Arc<Channel>, String> {
    if let Some((t, ch)) = lock(channels()).get(id) {
        if t.elapsed() < CACHE {
            return Ok(ch.clone());
        }
    }
    let (ch, latest) = fetch(id)?;
    let ch = Arc::new(ch);
    lock(channels()).insert(id.to_string(), (Instant::now(), ch.clone()));
    lock(listings()).insert(format!("{id} latest"), (Instant::now(), latest));
    Ok(ch)
}

/// At least `want` of the channel's videos in this order (fewer if it hasn't that many).
pub fn videos(ch: &Channel, sort: &str, want: usize) -> Result<(Vec<Video>, bool), String> {
    let key = format!("{} {sort}", ch.id);
    let cached = lock(listings()).get(&key).filter(|(t, _)| t.elapsed() < CACHE).map(|(_, l)| l.clone());
    let mut listing = match cached {
        Some(l) => l,
        None => {
            let tok = ch.sorts.get(sort).ok_or("YouTube doesn't offer that order for this channel")?;
            let d = post("browse", json!({"continuation": tok}))?;
            Listing { videos: video_lockups(&d, &ch.title, &ch.id), next: next_token(&d) }
        }
    };
    while listing.videos.len() < want {
        let Some(tok) = listing.next.take() else { break };
        let d = post("browse", json!({"continuation": tok}))?;
        let more = video_lockups(&d, &ch.title, &ch.id);
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

pub fn channel_page(ch: &Channel, sort: &str, videos: &[Video], has_more: bool, count: usize,
                    positions: &HashMap<String, Pos>) -> String {
    let about = ch.about.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let about = if about.chars().count() > 220 { format!("{}...", about.chars().take(220).collect::<String>()) } else { about.to_string() };
    let avatar = if ch.avatar.is_empty() { String::new() } else { format!(r#"<img class="avatar" src="{}" alt="">"#, esc(&ch.avatar)) };
    let mut parts = vec![crate::pages::search_box("", false), format!(
        r#"<div class="chead">{avatar}<div><h1>{}</h1><div class="meta">{}</div><div class="note">{}</div></div></div>"#,
        esc(&ch.title), join(&[esc(&ch.handle), esc(&ch.facts)]), esc(&about))];

    let sorts: Vec<String> = SORTS.iter().filter(|s| ch.sorts.contains_key(**s) || **s == "latest").map(|s| {
        if *s == sort { format!(r#"<b>{s}</b>"#) } else { format!(r#"<a href="/c/{}?sort={s}">{s}</a>"#, ch.id) }
    }).collect();
    let mut left = vec![format!(r#"<h2 class="col">Videos · {}</h2>"#, sorts.join(" · "))];
    let from = |extra: &[(&str, &str)]| {
        let mut p = vec![("from", "channel"), ("q", ch.title.as_str())];
        p.extend_from_slice(extra);
        urlencode(&p)
    };
    for v in videos {
        let href = format!("/v/{}?{}", v.id, from(&[]));
        let meta = join(&[esc(&v.info), duration(v.duration), progress(positions.get(&v.id))]);
        left.push(row(&thumb(&v.id, &href, None), &href, &v.title, &meta, &dl_button(&v.id, "")));
    }
    if videos.is_empty() {
        left.push(r#"<p class="note">No videos (Shorts and streams aren't shown).</p>"#.into());
    } else if has_more {
        left.push(format!(r#"<p><a href="/c/{}?sort={sort}&n={}">more videos</a></p>"#, ch.id, count + 30));
    }

    let mut right = vec![r#"<h2 class="col">Playlists</h2>"#.to_string()];
    if ch.playlists.is_empty() {
        right.push(r#"<p class="note">No playlists.</p>"#.into());
    }
    for p in &ch.playlists {
        let href = format!("/p/{}?{}", p.id, from(&[]));
        let meta = esc(if p.count.is_empty() { "playlist" } else { &p.count });
        let th = if p.thumb.is_empty() { r#"<div class="thumb"></div>"#.into() } else { thumb(&p.id, &href, Some(&p.thumb)) };
        right.push(row(&th, &href, &p.title, &meta, ""));
    }
    parts.push(format!(r#"<div class="split"><div>{}</div><div>{}</div></div>"#, left.concat(), right.concat()));
    page(&format!("{} - chastitube", ch.title), &parts.concat(), "", "wide")
}
