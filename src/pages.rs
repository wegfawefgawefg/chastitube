//! HTML for search, history, searches, watch time and saved. The video page is in player.rs.
use std::collections::HashMap;

use crate::store::{Entry, Pos, Search, Watch};
use crate::tags::View;
use crate::util::{esc, local, urlencode};
use crate::youtube::{Details, PlaylistHit, Video};
use crate::yt::{id_in_name, SavedFile};

const CSS: &str = r#"
* { box-sizing: border-box; }
body { background: #141414; color: #ddd; font: 16px/1.45 system-ui, sans-serif;
       max-width: 820px; margin: 0 auto; padding: 24px 18px 80px; }
a { color: #9cc4ff; text-decoration: none; }
header { display: flex; gap: 18px; align-items: center; margin-bottom: 18px; }
header .logo { display: flex; align-items: center; gap: 10px; }
header .logo img { width: 36px; height: 36px; }
header .name { font-weight: 700; color: #eee; letter-spacing: .02em; }
form.search { display: flex; gap: 8px; margin-bottom: 22px; }
form.search input { flex: 1; font: inherit; padding: 9px 12px; background: #222; color: #eee;
                    border: 1px solid #444; border-radius: 6px; }
button { font: inherit; font-size: 14px; padding: 5px 12px; background: #2a2a2a; color: #ddd;
         border: 1px solid #484848; border-radius: 6px; cursor: pointer; white-space: nowrap; }
button:hover { background: #363636; }
button.big { font-size: 16px; padding: 8px 18px; }
.row { display: flex; gap: 12px; align-items: center; padding: 10px 0; border-bottom: 1px solid #262626; }
.row .text { flex: 1; min-width: 0; }
.row .thumb { flex: none; width: 168px; aspect-ratio: 16 / 9; border-radius: 4px; overflow: hidden; background: #222; }
.row .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
.row .title { color: #eee; cursor: pointer; }
.row .title:hover { text-decoration: underline; }
.meta { color: #888; font-size: 14px; }
.meta a { color: #8fb3e6; }
.note { color: #999; }
.err { color: #e88; }
h1 { font-size: 22px; font-weight: 600; margin: 14px 0 4px; }
body.wide { max-width: 1700px; }
.stage { max-width: calc((100vh - 250px) * 16 / 9); margin: 0 auto; }  /* player fits the window */
.split { display: grid; grid-template-columns: minmax(0, 3fr) minmax(0, 2fr); gap: 36px; }
@media (max-width: 1100px) {
.split { grid-template-columns: minmax(0, 1fr); } }
h2.col { font-size: 13px; font-weight: 600; color: #888; text-transform: uppercase; letter-spacing: .08em; margin: 0 0 2px; }
h2.day { font-size: 15px; font-weight: 600; color: #aaa; margin: 26px 0 4px; }
.player { position: relative; width: 100%; aspect-ratio: 16 / 9; background: #000; border-radius: 6px; overflow: hidden; }
.player iframe, .player video { position: absolute; inset: 0; width: 100%; height: 100%; border: 0; background: #000; }
.player .done { position: absolute; inset: 0; display: flex; flex-direction: column; gap: 14px;
                align-items: center; justify-content: center; color: #aaa; }
.done .buttons { display: flex; gap: 10px; flex-wrap: wrap; justify-content: center; }
.under { display: flex; gap: 14px; align-items: flex-start; justify-content: space-between; }
.bars .bar { display: flex; gap: 12px; align-items: center; padding: 5px 0; }
.bars .label { width: 260px; flex: none; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.bars .fill { height: 10px; background: #6d8fbf; border-radius: 3px; }
.bars .val { color: #999; font-size: 14px; white-space: nowrap; }
.totals { display: flex; gap: 40px; margin: 8px 0 10px; }
.totals .big { font-size: 26px; color: #eee; }
.tagbar { display: flex; flex-wrap: wrap; gap: 6px; margin: -8px 0 14px; }
.chip { display: inline-flex; align-items: center; gap: 4px; padding: 1px 9px; border-radius: 11px;
        background: #23272e; color: #b9c7da; font-size: 13px; border: 1px solid #333a44; }
a.chip:hover, .chip a:hover { color: #fff; }
.chip.on { background: #3b5578; color: #fff; border-color: #5577a3; }
.chip .n { color: #7d8896; }
.chip a { color: inherit; }
.chip .x { border: 0; background: none; padding: 0 0 0 2px; font-size: 14px; color: #778; line-height: 1; }
.chip .x:hover { color: #e88; background: none; }
.tags { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 4px; align-items: center; }
.tags select.add { font: inherit; font-size: 12px; background: #1c1c1c; color: #888; border: 1px solid #333;
                   border-radius: 11px; padding: 0 4px; }
"#;

const JS: &str = r#"
async function post(path, body) {
  const r = await fetch(path, {method: 'POST', body: body || ''});
  if (!r.ok) throw new Error(await r.text());
  return r.json().catch(() => ({}));
}
function showFile(name) { post('/api/showfile', name); }
async function dl(id, btn) {
  btn.disabled = true;
  await post('/api/download/' + id);
  watch(id, btn);
}
async function watch(id, btn) {
  const st = await (await fetch('/api/status/' + id)).json();
  if (st.state === 'queued') { btn.textContent = 'Queued'; btn.disabled = true;
                               setTimeout(() => watch(id, btn), 2000); }
  else if (st.state === 'downloading') { btn.textContent = 'Downloading ' + (st.pct || ''); btn.disabled = true;
                                    setTimeout(() => watch(id, btn), 1000); }
  else if (st.state === 'done') { btn.textContent = 'Saved'; btn.disabled = true; }
  else if (st.state === 'failed') { btn.textContent = 'Failed, retry'; btn.title = st.error || ''; btn.disabled = false; }
}
document.querySelectorAll('button[data-dl]').forEach(b => watch(b.dataset.dl, b));
// links from other apps land in this tab if it's the one on screen (see tabs.rs)
new EventSource('/api/events').onmessage = (m) => {
  if (document.visibilityState !== 'visible') return;
  const d = JSON.parse(m.data);
  post('/api/ack/' + d.n).then((r) => { if (r.ok) location.href = d.path; });
};
"#;

pub fn page(title: &str, body: &str, extra_js: &str, body_class: &str) -> String {
    format!(r#"<!doctype html><html><head><meta charset="utf-8">
<title>{}</title><link rel="icon" href="/icon/rust_64.png">
<link rel="preconnect" href="https://i.ytimg.com"><link rel="preconnect" href="https://www.youtube.com">
<style>{CSS}</style></head><body class="{body_class}">
<header><a class="name logo" href="/"><img src="/icon/rust_128.png" alt="">chastitube</a>
<a href="/history">history</a><a href="/searches">searches</a><a href="/time">time</a><a href="/saved">saved</a></header>
{body}
<script>{JS}</script>{extra_js}</body></html>"#, esc(title))
}

pub fn search_box(q: &str, autofocus: bool) -> String {
    format!(r#"<form class="search" action="/" method="get">
<input name="q" value="{}" placeholder="search, or paste a youtube link"{}>
<button class="big">Search</button></form>"#, esc(q), if autofocus { " autofocus" } else { "" })
}

fn join(parts: &[String]) -> String {
    parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(" · ")
}

pub fn duration(sec: Option<u64>) -> String {
    match sec {
        None | Some(0) => String::new(),
        Some(s) => {
            let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
            if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
        }
    }
}

/// "3 h 12 min", "12 min", "40 s"
pub fn spent(sec: f64) -> String {
    let s = sec as u64;
    match (s / 3600, s / 60 % 60) {
        (0, 0) => format!("{s} s"),
        (0, m) => format!("{m} min"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// 'stopped at 1:23:04 of 3:02:10', 'watched', or '' for unseen videos.
pub fn progress(p: Option<&Pos>) -> String {
    let Some(p) = p.filter(|p| p.dur > 0.0) else { return String::new() };
    if p.pos > p.dur - 30.0 {
        return "watched".into();
    }
    if p.pos < 30.0 {
        return String::new();
    }
    format!("stopped at {} of {}", duration(Some(p.pos as u64)), duration(Some(p.dur as u64)))
}

fn thumb(vid: &str, href: &str, src: Option<&str>) -> String {
    let src = src.map(String::from).unwrap_or_else(|| format!("https://i.ytimg.com/vi/{vid}/mqdefault.jpg"));
    format!(r#"<a class="thumb" href="{}"><img loading="lazy" alt="" src="{}"></a>"#, esc(href), esc(&src))
}

pub fn dl_button(vid: &str, class: &str) -> String {
    let class = if class.is_empty() { String::new() } else { format!(r#" class="{class}""#) };
    format!(r#"<button{class} data-dl="{vid}" onclick="dl('{vid}', this)">Download</button>"#)
}

fn row(thumb: &str, href: &str, title: &str, meta: &str, extra: &str) -> String {
    format!(r#"<div class="row">{thumb}<div class="text">
<a class="title" href="{}">{}</a>
<div class="meta">{meta}</div></div>{extra}</div>"#, esc(href), esc(title))
}

pub fn search_page(q: &str, results: Option<&[Video]>, error: Option<&str>, count: usize,
                   positions: &HashMap<String, Pos>, playlists: &[PlaylistHit], pl_error: Option<&str>) -> String {
    let mut parts = vec![search_box(q, true)];
    if let Some(e) = error {
        parts.push(format!(r#"<p class="err">{}</p>"#, esc(e)));
    }
    let Some(results) = results.filter(|_| error.is_none()) else {
        let title = if q.is_empty() { "chastitube".to_string() } else { format!("{q} - chastitube") };
        return page(&title, &parts.concat(), "", "");
    };
    let mut left = vec![r#"<h2 class="col">Videos</h2>"#.to_string()];
    for r in results {
        let href = format!("/v/{}?{}", r.id, urlencode(&[("from", "search"), ("q", q)]));
        let meta = join(&[esc(&r.channel), if r.live { "live".into() } else { duration(r.duration) },
                          progress(positions.get(&r.id))]);
        left.push(row(&thumb(&r.id, &href, None), &href, &r.title, &meta, &dl_button(&r.id, "")));
    }
    if results.is_empty() {
        left.push(r#"<p class="note">Nothing found.</p>"#.into());
    } else if count < 60 {
        let more = urlencode(&[("q", q), ("n", &(count + 20).to_string())]);
        left.push(format!(r#"<p><a href="/?{}">more results</a></p>"#, esc(&more)));
    }
    let mut right = vec![r#"<h2 class="col">Playlists</h2>"#.to_string()];
    if let Some(e) = pl_error {
        right.push(format!(r#"<p class="err">{}</p>"#, esc(e)));
    } else if playlists.is_empty() {
        right.push(r#"<p class="note">No playlists.</p>"#.into());
    }
    for p in playlists {
        let href = format!("/p/{}?{}", p.id, urlencode(&[("from", "search"), ("q", q)]));
        let meta = join(&[esc(&p.channel), esc(if p.count.is_empty() { "playlist" } else { &p.count })]);
        let th = if p.thumb.is_empty() { r#"<div class="thumb"></div>"#.into() } else { thumb(&p.id, &href, Some(&p.thumb)) };
        right.push(row(&th, &href, &p.title, &meta, ""));
    }
    parts.push(format!(r#"<div class="split"><div>{}</div><div>{}</div></div>"#, left.concat(), right.concat()));
    page(&format!("{q} - chastitube"), &parts.concat(), "", "wide")
}

pub fn saved_page(files: &[SavedFile], folder: &str) -> String {
    let mut parts = vec![search_box("", true),
        format!(r##"<p class="note">Saved in {} · <a href="#" onclick="showFile(''); return false">open folder</a></p>"##, esc(folder))];
    if files.is_empty() {
        parts.push(r#"<p class="note">Nothing saved yet.</p>"#.into());
    }
    for f in files {
        let (folder_name, base) = f.name.rsplit_once('/').unwrap_or(("", &f.name));
        let id = id_in_name(base);
        let title = match (id, base.rfind(" [")) {
            (Some(_), Some(i)) => &base[..i],
            _ => base,
        };
        let js_name = esc(&serde_json::to_string(&f.name).unwrap_or_default());
        let (th, title_html) = match id {
            Some(id) => (thumb(id, &format!("/v/{id}?from=saved"), None),
                         format!(r#"<a class="title" href="/v/{id}?from=saved">{}</a>"#, esc(title))),
            None => (String::new(), esc(title)),
        };
        let place = if folder_name.is_empty() { String::new() } else { format!("{} · ", esc(folder_name)) };
        parts.push(format!(r#"<div class="row">{th}<div class="text">{title_html}
<div class="meta">{place}{:.0} MB</div></div>
<button onclick="showFile({js_name})">Open folder</button></div>"#, f.size as f64 / 1e6));
    }
    page("saved - chastitube", &parts.concat(), "", "")
}

fn source_label(e: &Entry) -> String {
    let d = e.detail.as_str();
    match e.source.as_str() {
        "search" if !d.is_empty() => format!("searched: {d}"),
        "search" => "search".into(),
        "link" if !d.is_empty() => format!("link from {d}"),
        "link" => "link from another app".into(),
        "paste" => "pasted link".into(),
        "history" => "from history".into(),
        "saved" => "from saved".into(),
        "playlist" if !d.is_empty() => format!("playlist: {d}"),
        "playlist" => "from a playlist".into(),
        _ => "opened directly".into(),
    }
}

fn day_heading(t: f64, day: &mut String, parts: &mut Vec<String>) {
    let this = local(t).format("%A %Y-%m-%d").to_string();
    if this != *day {
        parts.push(format!(r#"<h2 class="day">{this}</h2>"#));
        *day = this;
    }
}

/// Does a history entry match every word of the search? Looks at the title,
/// channel, what it was found with, YouTube's category and keywords, and its tags.
fn matches(e: &Entry, d: Option<&Details>, tags: &View, words: &[String]) -> bool {
    let mut hay = format!("{} {} {}", e.title, e.channel, e.detail);
    if let Some(d) = d {
        hay.push(' ');
        hay.push_str(&d.category);
        hay.push(' ');
        hay.push_str(&d.keywords.join(" "));
    }
    if let Some(t) = tags.tags.get(&e.id) {
        hay.push(' ');
        hay.push_str(&t.join(" "));
    }
    if let Some(k) = tags.kinds.get(&e.id) {
        hay.push(' ');
        hay.push_str(k);
    }
    let hay = hay.to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

const TAG_JS: &str = r#"<script>
function tagFix(id, tag, on) {
  if (tag) post('/api/tag/' + id, JSON.stringify({tag: tag, on: on})).then(() => location.reload());
}
</script>"#;

/// Every tag in your history with how many videos have it; click to filter.
fn tag_bar(entries: &[Entry], tags: &View, current: &str) -> String {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    for e in entries.iter().filter(|e| seen.insert(e.id.as_str())) {
        for t in tags.tags.get(&e.id).into_iter().flatten() {
            *counts.entry(t).or_default() += 1;
        }
    }
    let mut counts: Vec<(&str, usize)> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let chips: Vec<String> = counts.iter().map(|(t, n)| {
        let on = if *t == current { " on" } else { "" };
        format!(r#"<a class="chip{on}" href="/history?{}">{} <span class="n">{n}</span></a>"#, esc(&urlencode(&[("tag", t)])), esc(t))
    }).collect();
    if chips.is_empty() { String::new() } else { format!(r#"<div class="tagbar">{}</div>"#, chips.concat()) }
}

/// A video's tags (click one to filter, x to remove) and a menu to add one.
fn tag_line(id: &str, tags: &View) -> String {
    if !tags.enabled && tags.tags.is_empty() {
        return String::new();
    }
    let mine: &[String] = tags.tags.get(id).map(Vec::as_slice).unwrap_or(&[]);
    let mut out = String::from(r#"<div class="tags">"#);
    for t in mine {
        let js = esc(&serde_json::to_string(t).unwrap_or_default());
        out.push_str(&format!(r#"<span class="chip"><a href="/history?{}">{}</a><button class="x" title="not about this" onclick="tagFix('{id}', {js}, false)">×</button></span>"#,
                              esc(&urlencode(&[("tag", t)])), esc(t)));
    }
    let addable: Vec<&String> = tags.topics.iter().filter(|t| !mine.contains(t)).collect();
    if !addable.is_empty() {
        out.push_str(&format!(r#"<select class="add" onchange="tagFix('{id}', this.value, true)"><option value="">+ tag</option>{}</select>"#,
            addable.iter().map(|t| format!(r#"<option value="{0}">{0}</option>"#, esc(t))).collect::<String>()));
    }
    out + "</div>"
}

pub fn history_page(entries: &[Entry], positions: &HashMap<String, Pos>, details: &HashMap<String, Details>,
                    tags: &View, q: &str, tag: &str) -> String {
    let words: Vec<String> = q.to_lowercase().split_whitespace().map(String::from).collect();
    let mut parts = vec![format!(r#"<form class="search" action="/history" method="get">
<input name="q" value="{}" placeholder="search your history: title, channel, topic" autofocus>
<button class="big">Find</button></form>"#, esc(q)), tag_bar(entries, tags, tag)];
    let shown: Vec<&Entry> = entries.iter()
        .filter(|e| words.is_empty() || matches(e, details.get(&e.id), tags, &words))
        .filter(|e| tag.is_empty() || tags.tags.get(&e.id).is_some_and(|t| t.iter().any(|t| t == tag)))
        .take(1000).collect();
    if !q.is_empty() || !tag.is_empty() {
        parts.push(format!(r#"<p class="note">{} found · <a href="/history">show all</a></p>"#, shown.len()));
    } else if shown.is_empty() {
        parts.push(r#"<p class="note">Nothing watched yet.</p>"#.into());
    }
    let waiting = if tags.enabled { entries.iter().filter(|e| !tags.kinds.contains_key(&e.id)).map(|e| &e.id).collect::<std::collections::HashSet<_>>().len() } else { 0 };
    if waiting > 0 {
        parts.push(format!(r#"<p class="note">Tagging {waiting} video{} in the background.</p>"#, if waiting == 1 { "" } else { "s" }));
    }
    let mut day = String::new();
    for e in shown {
        day_heading(e.time, &mut day, &mut parts);
        let href = format!("/v/{}?from=history", e.id);
        let category = details.get(&e.id).map(|d| esc(&d.category)).unwrap_or_default();
        let kind = tags.kinds.get(&e.id).map(|k| esc(k)).unwrap_or_default();
        let meta = join(&[local(e.time).format("%H:%M").to_string(), esc(&e.channel), kind, category,
                          esc(&source_label(e)), progress(positions.get(&e.id))]) + &tag_line(&e.id, tags);
        let title = if e.title.is_empty() { &e.id } else { &e.title };
        parts.push(row(&thumb(&e.id, &href, None), &href, title, &meta, ""));
    }
    page("history - chastitube", &parts.concat(), TAG_JS, "")
}

/// Every search, newest first, with the videos you opened from it.
pub fn searches_page(searches: &[Search], history: &[Entry]) -> String {
    let mut parts = vec![search_box("", true)];
    if searches.is_empty() {
        parts.push(r#"<p class="note">No searches yet.</p>"#.into());
    }
    let mut day = String::new();
    for (i, s) in searches.iter().enumerate() {
        day_heading(s.time, &mut day, &mut parts);
        // videos opened from this search, up to the next time you searched it
        let until = searches[..i].iter().rev().find(|later| later.q == s.q).map(|l| l.time).unwrap_or(f64::MAX);
        let mut opened: Vec<&Entry> = history.iter()
            .filter(|e| e.source == "search" && e.detail == s.q && e.time >= s.time - 1.0 && e.time < until).collect();
        opened.reverse();
        let mut meta = vec![local(s.time).format("%H:%M").to_string()];
        if !opened.is_empty() {
            let links: Vec<String> = opened.iter().take(6).map(|e| format!(r#"<a href="/v/{}?from=history">{}</a>"#,
                e.id, esc(if e.title.is_empty() { &e.id } else { &e.title }))).collect();
            let more = if opened.len() > 6 { format!(" and {} more", opened.len() - 6) } else { String::new() };
            meta.push(format!("opened {}{more}", links.join(", ")));
        }
        let href = format!("/?{}", urlencode(&[("q", &s.q)]));
        parts.push(format!(r#"<div class="row"><div class="text"><a class="title" href="{}">{}</a>
<div class="meta">{}</div></div></div>"#, esc(&href), esc(&s.q), meta.join(" · ")));
    }
    page("searches - chastitube", &parts.concat(), "", "")
}

fn bars(rows: &[(String, f64)], top: f64) -> String {
    let mut out = String::from(r#"<div class="bars">"#);
    for (label, secs) in rows {
        let w = if top > 0.0 { (secs / top * 100.0).max(1.0) } else { 0.0 };
        out.push_str(&format!(r#"<div class="bar"><div class="label">{label}</div><div style="flex: 1"><div class="fill" style="width: {w:.1}%"></div></div><div class="val">{}</div></div>"#, spent(*secs)));
    }
    out + "</div>"
}

/// Time actually spent playing: per day, per video and per channel.
pub fn time_page(watch: &Watch, titles: &HashMap<String, (String, String)>, tags: &View) -> String {
    let mut parts = vec![String::new()];
    if watch.is_empty() {
        parts.push(r#"<p class="note">Nothing yet. Time is counted while a video plays, from now on.</p>"#.into());
        return page("time - chastitube", &parts.concat(), "", "");
    }
    let today = crate::util::today();
    let week_start = (chrono::Local::now() - chrono::Duration::days(6)).format("%Y-%m-%d").to_string();
    let day_total = |d: &str| watch.get(d).map(|m| m.values().sum::<f64>()).unwrap_or(0.0);
    let week: f64 = watch.range(week_start.clone()..).map(|(_, m)| m.values().sum::<f64>()).sum();
    let all: f64 = watch.values().map(|m| m.values().sum::<f64>()).sum();
    parts.push(format!(r#"<div class="totals"><div><div class="big">{}</div><div class="meta">today</div></div>
<div><div class="big">{}</div><div class="meta">last 7 days</div></div>
<div><div class="big">{}</div><div class="meta">since {}</div></div></div>"#,
        spent(day_total(&today)), spent(week), spent(all), watch.keys().next().map(String::as_str).unwrap_or("")));

    let days: Vec<(String, f64)> = watch.iter().rev().take(14)
        .map(|(d, m)| (esc(&local_day_label(d)), m.values().sum())).collect();
    let top = days.iter().map(|d| d.1).fold(0.0, f64::max);
    parts.push(format!(r#"<h2 class="day">By day</h2>{}"#, bars(&days, top)));

    let (mut videos, mut channels): (HashMap<&str, f64>, HashMap<String, f64>) = Default::default();
    for m in watch.values() {
        for (id, s) in m {
            *videos.entry(id).or_default() += s;
            let ch = titles.get(id).map(|t| t.1.clone()).unwrap_or_default();
            *channels.entry(if ch.is_empty() { "unknown".into() } else { ch }).or_default() += s;
        }
    }
    let mut videos: Vec<(&str, f64)> = videos.into_iter().collect();
    videos.sort_by(|a, b| b.1.total_cmp(&a.1));
    let rows: Vec<(String, f64)> = videos.iter().take(25).map(|(id, s)| {
        let title = titles.get(*id).map(|t| t.0.as_str()).filter(|t| !t.is_empty()).unwrap_or(id);
        (format!(r#"<a href="/v/{id}?from=history" title="{0}">{0}</a>"#, esc(title)), *s)
    }).collect();
    parts.push(format!(r#"<h2 class="day">Videos</h2>{}"#, bars(&rows, videos.first().map(|v| v.1).unwrap_or(0.0))));

    if !tags.tags.is_empty() {
        parts.push(topic_bars(&videos, tags));
    }

    let mut channels: Vec<(String, f64)> = channels.into_iter().collect();
    channels.sort_by(|a, b| b.1.total_cmp(&a.1));
    let rows: Vec<(String, f64)> = channels.iter().take(15).map(|(c, s)| (esc(c), *s)).collect();
    parts.push(format!(r#"<h2 class="day">Channels</h2>{}"#, bars(&rows, channels.first().map(|c| c.1).unwrap_or(0.0))));
    page("time - chastitube", &parts.concat(), "", "")
}

/// Time per topic. A video with two topics counts toward both.
fn topic_bars(videos: &[(&str, f64)], tags: &View) -> String {
    let mut by: HashMap<String, f64> = HashMap::new();
    for (id, s) in videos {
        match tags.tags.get(*id).filter(|t| !t.is_empty()) {
            Some(t) => for tag in t { *by.entry(tag.clone()).or_default() += s; },
            None => *by.entry(if tags.kinds.contains_key(*id) { "no topic".into() } else { "not tagged yet".into() }).or_default() += s,
        }
    }
    let mut by: Vec<(String, f64)> = by.into_iter().collect();
    by.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top = by.first().map(|b| b.1).unwrap_or(0.0);
    let rows: Vec<(String, f64)> = by.iter().map(|(t, s)| {
        let label = if tags.topics.contains(t) { format!(r#"<a href="/history?{}">{}</a>"#, esc(&urlencode(&[("tag", t)])), esc(t)) } else { esc(t) };
        (label, *s)
    }).collect();
    format!(r#"<h2 class="day">Topics</h2>{}"#, bars(&rows, top))
}

fn local_day_label(day: &str) -> String {
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").map(|d| d.format("%a %b %-d").to_string()).unwrap_or_else(|_| day.into())
}

pub fn message_page(text: &str) -> String {
    page("chastitube", &format!(r#"{}<p class="note">{}</p>"#, search_box("", true), esc(text)), "", "")
}
