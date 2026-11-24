//! The video page. Saved videos play from disk in a plain <video>; anything
//! else plays in YouTube's embedded player, kept to that one video. Both save
//! your place every few seconds, on pause and when you leave, and count the
//! seconds actually played for the time page.
//!
//! Opened from a playlist, the page also lists the playlist beside the
//! player, offers the next video when one ends, and can go on to it by
//! itself (the "autoplay next" box, remembered in this browser).
use std::collections::{HashMap, HashSet};

use crate::pages::{dl_button, duration, page, progress, search_box};
use crate::store::Pos;
use crate::util::{esc, quote_path, urlencode};
use crate::youtube::Playlist;

const PLAYLIST_CSS: &str = r#"
<style>
.watch { display: grid; grid-template-columns: minmax(0, calc((100vh - 250px) * 16 / 9)) 420px; gap: 20px;
         align-items: start; justify-content: center; }
@media (max-width: 1000px) { .watch { grid-template-columns: minmax(0, 1fr); } }
.plist { background: #1b1b1b; border: 1px solid #2a2a2a; border-radius: 6px;
         display: flex; flex-direction: column; height: calc(100vh - 170px); min-height: 360px; }
.plist .head { padding: 12px 14px; border-bottom: 1px solid #2a2a2a; }
.plist .head .name { color: #eee; font-weight: 600; }
.plist .head .tools { display: flex; gap: 12px; align-items: center; margin-top: 8px; font-size: 14px; color: #aaa; }
.plist .items { flex: 1; min-height: 0; overflow-y: auto; scrollbar-color: #444 #1b1b1b; }
.plist .item { display: flex; gap: 10px; padding: 7px 10px; align-items: center; color: #ccc; }
.plist .item:hover { background: #242424; }
.plist .item.now { background: #2b2b2b; }
.plist .item .n { width: 26px; text-align: right; color: #777; font-size: 13px; flex: none; }
.plist .item img { width: 112px; aspect-ratio: 16 / 9; object-fit: cover; border-radius: 3px; flex: none; background: #222; }
.plist .item .t { font-size: 14px; line-height: 1.3; }
.plist .item .meta { font-size: 12px; }
</style>"#;

// `played` is wall-clock seconds of playback since the last report.
const SAVE_POS_JS: &str = r#"
let playStart = null;
function played() {
  if (playStart === null) return 0;
  const t = performance.now(), s = (t - playStart) / 1000;
  playStart = t; return s;
}
function postPos(pos, dur) {
  const p = played();
  if (dur > 0 && isFinite(dur)) fetch('/api/pos/' + VIDEO_ID, {method: 'POST', keepalive: true,
                                       body: JSON.stringify({pos: pos, dur: dur, played: p})});
}
"#;

// What the player box shows once a video is over: Watch again, and Next
// when there is one. With "autoplay next" on, it goes on after 3 seconds.
const FINISH_JS: &str = r#"
function button(text, href) {
  const a = document.createElement('a'); a.href = href;
  const b = document.createElement('button'); b.className = 'big'; b.textContent = text;
  a.appendChild(b); return a;
}
function finish(text, ended) {
  const box = document.getElementById('player-box');
  const done = document.createElement('div'); done.className = 'done';
  const msg = document.createElement('div'); msg.textContent = text; done.appendChild(msg);
  const row = document.createElement('div'); row.className = 'buttons';
  row.appendChild(button('Watch again', AGAIN_URL));
  if (NEXT) row.appendChild(button('Next: ' + NEXT.title, NEXT.url));
  done.appendChild(row);
  box.replaceChildren(done);
  if (ended && NEXT && localStorage.getItem('autoplay') === '1') {
    const note = document.createElement('div'); done.appendChild(note);
    let left = 3;
    const tick = () => {
      if (left === 0) { location.href = NEXT.url; return; }
      note.textContent = 'Next video in ' + left + '... ';
      const stop = document.createElement('a'); stop.href = '#'; stop.textContent = 'stop';
      stop.onclick = (e) => { e.preventDefault(); clearTimeout(timer); note.textContent = ''; };
      note.appendChild(stop);
      left -= 1; timer = setTimeout(tick, 1000);
    };
    let timer; tick();
  }
}
"#;

// YouTube's player: when it ends, or if anything inside it tries to switch
// to a different video, the player is removed.
const EMBED_JS: &str = r#"
let player, timer;
function savePos(atEnd) {
  if (!player || !player.getCurrentTime || !player.getVideoData) return;
  if (player.getVideoData().video_id !== VIDEO_ID) return;
  const dur = player.getDuration();
  postPos(atEnd ? dur : player.getCurrentTime(), dur);
}
window.addEventListener('pagehide', () => savePos(false));
function onYouTubeIframeAPIReady() {
  player = new YT.Player('player', {events: {onStateChange: (e) => {
    const data = player.getVideoData ? player.getVideoData() : null;
    if (data && data.video_id && data.video_id !== VIDEO_ID) { playStart = null; return finish('That was a different video.', false); }
    clearInterval(timer);
    if (e.data === YT.PlayerState.PLAYING) {
      playStart = performance.now();
      timer = setInterval(() => savePos(false), 5000);
    } else {
      if (e.data === YT.PlayerState.ENDED) { savePos(true); finish('Finished.', true); }
      else if (playStart !== null) savePos(false);
      playStart = null;
    }
  }}});
}
"#;

const LOCAL_JS: &str = r#"
const v = document.getElementById('local');
let timer;
v.addEventListener('loadedmetadata', () => {
  if (START) v.currentTime = START;
  v.play().catch(() => {});
});
v.addEventListener('play', () => {
  playStart = performance.now();
  clearInterval(timer);
  timer = setInterval(() => postPos(v.currentTime, v.duration), 5000);
});
v.addEventListener('pause', () => { clearInterval(timer); postPos(v.currentTime, v.duration); playStart = null; });
v.addEventListener('ended', () => {
  clearInterval(timer); postPos(v.duration, v.duration); playStart = null;
  if (NEXT) finish('Finished.', true);
});
window.addEventListener('pagehide', () => postPos(v.currentTime, v.duration));
"#;

const PLAYLIST_JS: &str = r#"
const auto = document.getElementById('autoplay');
if (auto) {
  auto.checked = localStorage.getItem('autoplay') === '1';
  auto.onchange = () => localStorage.setItem('autoplay', auto.checked ? '1' : '0');
}
const now = document.querySelector('.plist .item.now');
if (now) {  // scroll the list (not the page) so the current video is in the middle
  const box = now.parentElement;
  const off = now.getBoundingClientRect().top - box.getBoundingClientRect().top;
  box.scrollTop = Math.max(0, off - box.clientHeight / 2 + now.clientHeight / 2);
}
async function dlList(btn, count) {
  if (!confirm('Download all ' + count + ' videos into a folder of their own?')) return;
  btn.disabled = true;
  await post('/api/download-list/' + LIST_ID);
  watchList(btn);
}
async function watchList(btn) {
  const st = await (await fetch('/api/list-status/' + LIST_ID)).json();
  if (st.busy) { btn.disabled = true; btn.textContent = 'Saving ' + st.saved + ' of ' + st.total;
                 setTimeout(() => watchList(btn), 3000); }
  else if (st.saved === st.total) { btn.disabled = true; btn.textContent = 'All saved'; }
  else { btn.disabled = false; btn.textContent = 'Download all'; }
}
const dlAll = document.getElementById('dl-all');
if (dlAll) watchList(dlAll);
"#;

pub fn embed_url(vid: &str, start: u64, origin: &str) -> String {
    let start = start.to_string();
    let mut params = vec![("autoplay", "1"), ("rel", "0"), ("iv_load_policy", "3"), ("playsinline", "1"),
                          ("enablejsapi", "1"), ("origin", origin)];
    if start != "0" {
        params.push(("start", &start));
    }
    format!("https://www.youtube.com/embed/{vid}?{}", urlencode(&params))
}

fn in_list_url(vid: &str, pl: Option<&Playlist>, restart: bool) -> String {
    let mut params: Vec<(&str, &str)> = Vec::new();
    if restart {
        params.push(("restart", "1"));
    }
    if let Some(pl) = pl {
        params.extend([("list", pl.id.as_str()), ("from", "playlist"), ("pl", pl.title.as_str())]);
    }
    if params.is_empty() { format!("/v/{vid}") } else { format!("/v/{vid}?{}", urlencode(&params)) }
}

fn next_video(vid: &str, pl: Option<&Playlist>) -> serde_json::Value {
    let Some(pl) = pl else { return serde_json::Value::Null };
    let i = pl.videos.iter().position(|v| v.id == vid).map(|i| i + 1).unwrap_or(usize::MAX);
    match pl.videos.get(i) {
        Some(n) => serde_json::json!({"title": n.title, "url": in_list_url(&n.id, Some(pl), false)}),
        None => serde_json::Value::Null,
    }
}

fn playlist_panel(vid: &str, pl: &Playlist, positions: &HashMap<String, Pos>, saved: &HashSet<String>) -> String {
    let mut items = String::new();
    for (i, v) in pl.videos.iter().enumerate() {
        let meta: Vec<String> = [duration(v.duration), progress(positions.get(&v.id)),
                                 if saved.contains(&v.id) { "saved".into() } else { String::new() }]
            .into_iter().filter(|s| !s.is_empty()).collect();
        let cls = if v.id == vid { "item now" } else { "item" };
        items.push_str(&format!(r#"<a class="{cls}" href="{}"><span class="n">{}</span><img loading="lazy" alt="" src="https://i.ytimg.com/vi/{}/mqdefault.jpg"><span><div class="t">{}</div><div class="meta">{}</div></span></a>"#,
            esc(&in_list_url(&v.id, Some(pl), false)), i + 1, v.id, esc(&v.title), meta.join(" · ")));
    }
    let count = pl.videos.len();
    format!(r#"<div class="plist"><div class="head">
<div class="name">{}</div>
<div class="meta">{} · {count} videos</div>
<div class="tools"><label><input type="checkbox" id="autoplay"> autoplay next</label>
<button id="dl-all" onclick="dlList(this, {count})">Download all</button></div></div>
<div class="items">{items}</div></div>"#, esc(&pl.title), esc(&pl.channel))
}

pub struct VideoPage<'a> {
    pub vid: &'a str,
    pub title: &'a str,
    pub channel: &'a str,
    /// "UC..." if known: the channel name links to its page
    pub channel_id: &'a str,
    pub start: u64,
    pub origin: &'a str,
    pub resumed: u64,
    pub saved_name: Option<&'a str>,
    pub pl: Option<&'a Playlist>,
    pub positions: &'a HashMap<String, Pos>,
    pub saved: &'a HashSet<String>,
}

pub fn video_page(p: &VideoPage) -> String {
    let mut notes: Vec<String> = Vec::new();
    if !p.channel.is_empty() {
        notes.push(crate::pages::channel_link(p.channel, p.channel_id));
    }
    if p.saved_name.is_some() {
        notes.push("your saved copy".into());
    }
    if p.resumed > 0 {
        notes.push(format!(r#"picked up where you left off ({}) · <a href="{}">start from the beginning</a>"#,
                           duration(Some(p.resumed)), esc(&in_list_url(p.vid, p.pl, true))));
    }
    let (player, button, play_js, api) = match p.saved_name {
        Some(name) => (
            format!(r#"<video id="local" controls preload="metadata" src="/file/{}"></video>"#, esc(&quote_path(name))),
            format!(r#"<button class="big" onclick="showFile({})">Open folder</button>"#, esc(&serde_json::to_string(name).unwrap_or_default())),
            format!("const START = {};{LOCAL_JS}", p.start),
            "",
        ),
        None => (
            format!(r#"<iframe id="player" src="{}" allow="autoplay; fullscreen; encrypted-media; picture-in-picture" allowfullscreen referrerpolicy="strict-origin-when-cross-origin"></iframe>"#,
                    esc(&embed_url(p.vid, p.start, p.origin))),
            dl_button(p.vid, "big"),
            EMBED_JS.to_string(),
            r#"<script src="https://www.youtube.com/iframe_api"></script>"#,
        ),
    };
    let consts = format!("const VIDEO_ID = {};const LIST_ID = {};const NEXT = {};const AGAIN_URL = {};",
        serde_json::json!(p.vid), serde_json::json!(p.pl.map(|pl| &pl.id)), next_video(p.vid, p.pl),
        serde_json::json!(in_list_url(p.vid, p.pl, true)));
    let consts = consts.replace("</", "<\\/"); // a title can't end the script early
    let scripts = format!("<script>{consts}{SAVE_POS_JS}{FINISH_JS}{play_js}{}</script>{api}",
                          if p.pl.is_some() { PLAYLIST_JS } else { "" });
    let main = format!(r#"<div class="player" id="player-box">{player}</div>
<div class="under"><div><h1>{}</h1><p class="meta">{}</p></div>
<p>{button}</p></div>"#, esc(p.title), notes.join(" · "));
    let search = search_box("", false); // keys belong to the player here
    let body = match p.pl {
        Some(pl) => format!(r#"{search}{PLAYLIST_CSS}<div class="watch"><div>{main}</div>{}</div>"#,
                            playlist_panel(p.vid, pl, p.positions, p.saved)),
        None => format!(r#"{search}<div class="stage">{main}</div>"#),
    };
    page(&format!("{} - chastitube", p.title), &body, &scripts, "wide")
}
