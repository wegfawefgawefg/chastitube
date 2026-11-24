//! Work out what a link someone sent you points at.
use crate::util::{valid_id, valid_list};

pub enum Link {
    Video { id: String, start: Option<u64>, list: Option<String> },
    Playlist(String),
    Search(String),
    /// "@handle", "UC..." (a channel id), "c/name" or "user/name"
    Channel(String),
    /// A YouTube page we don't open (home, a Mix, Shorts feed...).
    YouTube,
}

/// '90', '90s', '1m30s', '1h2m3s' -> seconds.
pub fn parse_time(t: Option<&str>) -> Option<u64> {
    let t = t?.trim();
    if t.is_empty() {
        return None;
    }
    if t.bytes().all(|b| b.is_ascii_digit()) {
        return t.parse().ok();
    }
    let (mut total, mut num, mut last_unit, mut any) = (0u64, String::new(), 0, false);
    for c in t.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let (unit, mult) = match c {
            'h' => (1, 3600),
            'm' => (2, 60),
            's' => (3, 1),
            _ => return None,
        };
        if num.is_empty() || unit <= last_unit {
            return None;
        }
        total += num.parse::<u64>().ok()? * mult;
        num.clear();
        last_unit = unit;
        any = true;
    }
    if !num.is_empty() || !any {
        return None;
    }
    Some(total)
}

const YT_HOSTS: &[&str] = &["youtube.com", "www.youtube.com", "m.youtube.com",
                            "youtube-nocookie.com", "www.youtube-nocookie.com"];

/// None means not YouTube (YouTube Music counts as not YouTube).
pub fn classify(text: &str) -> Option<Link> {
    let u = url::Url::parse(text.trim()).ok()?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return None;
    }
    let host = u.host_str().unwrap_or("").to_lowercase();
    let q: Vec<(String, String)> = u.query_pairs().into_owned().filter(|(_, v)| !v.is_empty()).collect();
    let get = |k: &str| q.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    let start = parse_time(get("t").or(get("start")));
    let list = get("list").filter(|l| valid_list(l)).map(String::from);
    let parts: Vec<&str> = u.path().split('/').filter(|p| !p.is_empty()).collect();

    if host == "youtu.be" {
        return Some(match parts.first() {
            Some(id) if valid_id(id) => Link::Video { id: id.to_string(), start, list },
            _ => Link::YouTube,
        });
    }
    if !YT_HOSTS.contains(&host.as_str()) {
        return None;
    }
    let vid = if parts.first() == Some(&"watch") {
        get("v")
    } else if parts.len() >= 2 && ["shorts", "live", "embed", "v"].contains(&parts[0]) {
        Some(parts[1])
    } else {
        None
    };
    if let Some(id) = vid.filter(|v| valid_id(v)) {
        return Some(Link::Video { id: id.to_string(), start, list });
    }
    if parts.first() == Some(&"playlist") {
        if let Some(l) = list {
            return Some(Link::Playlist(l));
        }
    }
    if let Some(first) = parts.first() {
        if first.starts_with('@') && first.len() > 1 {
            return Some(Link::Channel(first.to_string()));
        }
        if parts.len() >= 2 && *first == "channel" && parts[1].starts_with("UC") {
            return Some(Link::Channel(parts[1].to_string()));
        }
        if parts.len() >= 2 && (*first == "c" || *first == "user") {
            return Some(Link::Channel(format!("{first}/{}", parts[1])));
        }
    }
    if parts.first() == Some(&"results") {
        if let Some(sq) = get("search_query") {
            return Some(Link::Search(sq.to_string()));
        }
    }
    Some(Link::YouTube)
}
