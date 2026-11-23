//! YouTube's own web API (the one its pages call): search, playlists and one
//! video's details. One kept-alive connection pool, so after the first call
//! there's no new TLS handshake per request.
//!
//! Only search, playlist pages and a single video's details are fetched.
//! Nothing here asks for recommendations, the home feed or related videos.
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Duration;

use crate::util::valid_id;

const API: &str = "https://www.youtube.com/youtubei/v1/";
const PLAYLISTS_FILTER: &str = "EgIQAw=="; // YouTube's "Type: Playlist" search filter

fn client() -> Value {
    json!({"clientName": "WEB", "clientVersion": "2.20251120.01.00", "hl": "en", "gl": "US"})
}

pub fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).max_idle_connections_per_host(4).build())
}

fn post(endpoint: &str, mut body: Value) -> Result<Value, String> {
    body["context"] = json!({"client": client()});
    let resp = agent().post(&format!("{API}{endpoint}?prettyPrint=false"))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Every value stored under `key`, anywhere in the response, in order.
pub fn find<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == key { out.push(x) } else { find(x, key, out) }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| find(x, key, out)),
        _ => {}
    }
}

fn found<'a>(v: &'a Value, key: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    find(v, key, &mut out);
    out
}

fn text(t: &Value) -> String {
    if let Some(s) = t.get("simpleText").and_then(Value::as_str) {
        return s.to_string();
    }
    t.get("runs").and_then(Value::as_array)
        .map(|runs| runs.iter().filter_map(|r| r["text"].as_str()).collect())
        .unwrap_or_default()
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

/// "1:02:03" -> 3723
pub fn seconds(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut total = 0u64;
    for p in s.split(':') {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        total = total * 60 + p.parse::<u64>().ok()?;
    }
    Some(total)
}

/// The token for the next page of results. In search responses it's the
/// last continuationItemRenderer (earlier tokens belong to the filter chips);
/// in playlists it's the first continuationItemViewModel.
fn next_token(d: &Value) -> Option<String> {
    let token = |item: &&Value| found(item, "token").first().and_then(|t| t.as_str()).map(String::from);
    found(d, "continuationItemRenderer").last().and_then(token)
        .or_else(|| found(d, "continuationItemViewModel").first().and_then(token))
}

#[derive(Clone)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration: Option<u64>,
    pub live: bool,
}

#[derive(Clone)]
pub struct PlaylistHit {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub thumb: String,
    pub count: String,
}

#[derive(Clone)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub videos: Vec<Video>,
}

/// One results page per 20 wanted (a page holds about 20).
fn search_pages(query: &str, params: Option<&str>, count: usize) -> Result<Vec<Value>, String> {
    let mut body = json!({"query": query});
    if let Some(p) = params {
        body["params"] = json!(p);
    }
    let mut pages = Vec::new();
    for _ in 0..count.div_ceil(20) {
        let d = post("search", body)?;
        let token = next_token(&d);
        pages.push(d);
        match token {
            Some(t) => body = json!({"continuation": t}),
            None => break,
        }
    }
    Ok(pages)
}

pub fn videos(query: &str, count: usize) -> Result<Vec<Video>, String> {
    let pages = search_pages(query, None, count)?;
    let mut out: Vec<Video> = Vec::new();
    for d in &pages {
        for v in found(d, "videoRenderer") {
            let id = s(&v["videoId"]);
            let url = v["navigationEndpoint"]["commandMetadata"]["webCommandMetadata"]["url"].as_str().unwrap_or("");
            if !valid_id(&id) || url.starts_with("/shorts/") || out.iter().any(|o| o.id == id) {
                continue;
            }
            let badges = format!("{}{}", v["badges"], v["thumbnailOverlays"]);
            let length = text(&v["lengthText"]);
            let mut channel = text(&v["ownerText"]);
            if channel.is_empty() {
                channel = text(&v["longBylineText"]);
            }
            let title = text(&v["title"]);
            out.push(Video {
                title: if title.is_empty() { id.clone() } else { title },
                channel,
                duration: seconds(&length),
                live: badges.contains("LIVE") && length.is_empty(),
                id,
            });
        }
    }
    if out.is_empty() {
        return Err("search API gave no videos".into());
    }
    out.truncate(count);
    Ok(out)
}

fn lockup_channel(lk: &Value) -> String {
    let parts = found(&lk["metadata"]["lockupMetadataViewModel"]["metadata"], "metadataParts");
    parts.first().and_then(|p| p.get(0)).map(|p| s(&p["text"]["content"])).unwrap_or_default()
}

fn lockup_badge(lk: &Value) -> String {
    found(&lk["contentImage"], "thumbnailBadgeViewModel").first().map(|b| s(&b["text"])).unwrap_or_default()
}

pub fn playlists(query: &str, count: usize) -> Result<Vec<PlaylistHit>, String> {
    let pages = search_pages(query, Some(PLAYLISTS_FILTER), count)?;
    let mut out = Vec::new();
    for d in &pages {
        for lk in found(d, "lockupViewModel") {
            if lk["contentType"] != "LOCKUP_CONTENT_TYPE_PLAYLIST" {
                continue;
            }
            let id = s(&lk["contentId"]);
            let title = s(&lk["metadata"]["lockupMetadataViewModel"]["title"]["content"]);
            let thumb = found(&lk["contentImage"], "sources").iter()
                .find_map(|src| src.get(0).and_then(|x| x["url"].as_str()).map(String::from))
                .unwrap_or_default();
            out.push(PlaylistHit {
                title: if title.is_empty() { id.clone() } else { title },
                channel: lockup_channel(lk),
                count: lockup_badge(lk),
                thumb,
                id,
            });
        }
    }
    if out.is_empty() {
        return Err("search API gave no playlists".into());
    }
    out.truncate(count);
    Ok(out)
}

/// A playlist's title, channel and videos in order (100 per page).
/// Private and deleted videos are left out, and so are repeats.
pub fn playlist(list_id: &str) -> Result<Playlist, String> {
    let mut body = json!({"browseId": format!("VL{list_id}")});
    let (mut title, mut channel, mut videos) = (String::new(), String::new(), Vec::<Video>::new());
    for page in 0..50 {
        let d = post("browse", body)?;
        if page == 0 {
            title = s(&d["metadata"]["playlistMetadataRenderer"]["title"]);
            channel = found(&d["header"], "content").iter().filter_map(|c| c.as_str())
                .find_map(|c| c.strip_prefix("by ")).unwrap_or("").to_string();
            if title.is_empty() {
                return Err("YouTube didn't return that playlist".into());
            }
        }
        for lk in found(&d, "lockupViewModel") {
            if lk["contentType"] != "LOCKUP_CONTENT_TYPE_VIDEO" {
                continue;
            }
            let id = s(&lk["contentId"]);
            let t = s(&lk["metadata"]["lockupMetadataViewModel"]["title"]["content"]);
            if !valid_id(&id) || t == "[Private video]" || t == "[Deleted video]" || videos.iter().any(|v| v.id == id) {
                continue;
            }
            videos.push(Video {
                title: if t.is_empty() { id.clone() } else { t },
                channel: lockup_channel(lk),
                duration: seconds(&lockup_badge(lk)),
                live: false,
                id,
            });
        }
        match next_token(&d) {
            Some(t) => body = json!({"continuation": t}),
            None => break,
        }
    }
    if channel.is_empty() {
        channel = videos.first().map(|v| v.channel.clone()).unwrap_or_default();
    }
    Ok(Playlist { id: list_id.to_string(), title, channel, videos })
}

/// What YouTube says about one video: its own category, the uploader's
/// keywords and the description. Kept for search over history and tags.
#[derive(Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct Details {
    pub id: String,
    pub title: String,
    pub channel: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub published: String,
    #[serde(default)]
    pub fetched: f64,
}

pub fn details(vid: &str) -> Result<Details, String> {
    let d = post("player", json!({"videoId": vid}))?;
    let vd = &d["videoDetails"];
    let mf = &d["microformat"]["playerMicroformatRenderer"];
    let title = s(&vd["title"]);
    if title.is_empty() {
        return Err(format!("no details for {vid}"));
    }
    Ok(Details {
        id: vid.to_string(),
        title,
        channel: s(&vd["author"]),
        channel_id: s(&vd["channelId"]),
        duration: s(&vd["lengthSeconds"]).parse().unwrap_or(0),
        category: s(&mf["category"]),
        keywords: vd["keywords"].as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default(),
        description: s(&vd["shortDescription"]),
        published: s(&mf["publishDate"]),
        fetched: crate::util::now(),
    })
}

/// Title and channel only, through oEmbed: the fallback if the player API fails.
pub fn oembed(vid: &str) -> Result<(String, String), String> {
    let url = format!("https://www.youtube.com/oembed?format=json&url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3D{vid}");
    let d: Value = serde_json::from_str(&agent().get(&url).call().map_err(|e| e.to_string())?
        .into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok((s(&d["title"]), s(&d["author_name"])))
}
