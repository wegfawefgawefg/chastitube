//! Settings, small helpers and the ID rules shared by everything else.
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

pub fn port() -> u16 {
    std::env::var("CHASTITUBE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8790)
}

/// chastitube's own loopback address. `./install.sh --name` maps the name
/// "chastitube" to it in /etc/hosts and lets the server use port 80 there, so
/// the page is http://chastitube/ and can't clash with anything on localhost.
pub const NAME_IP: [u8; 4] = [127, 0, 0, 73];
pub const NAME_URL: &str = "http://chastitube";

/// The name is set up: "chastitube" resolves to NAME_IP and something answers on port 80 there.
/// Only for the real server (default port), not test servers on other ports.
pub fn name_works() -> bool {
    if port() != 8790 {
        return false;
    }
    let resolves = ("chastitube", 80).to_socket_addrs()
        .map(|mut a| a.any(|a| a.ip() == IpAddr::from(NAME_IP))).unwrap_or(false);
    resolves && TcpStream::connect_timeout(&SocketAddr::from((NAME_IP, 80)), Duration::from_millis(300)).is_ok()
}

pub fn local_base() -> String {
    format!("http://localhost:{}", port())
}

/// Where chastitube's pages live: http://chastitube if that's set up, else http://localhost:8790.
pub fn base() -> String {
    static BASE: OnceLock<String> = OnceLock::new();
    BASE.get_or_init(|| if name_works() { NAME_URL.to_string() } else { local_base() }).clone()
}

/// Every address our own pages can be opened under.
pub fn own_origins() -> [String; 3] {
    [local_base(), format!("http://127.0.0.1:{}", port()), NAME_URL.to_string()]
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
}

/// The chastitube folder (the one holding art/ and bin/), found from where the binary is.
pub fn app_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) {
        let mut dir = exe.parent().map(PathBuf::from);
        while let Some(d) = dir {
            if d.join("art").is_dir() && d.join("Cargo.toml").is_file() {
                return d;
            }
            dir = d.parent().map(PathBuf::from);
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("CHASTITUBE_DATA_DIR").map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share/chastitube"))
}

pub fn save_dir() -> PathBuf {
    std::env::var_os("CHASTITUBE_SAVE_DIR").map(PathBuf::from)
        .unwrap_or_else(|| home().join("Videos/chastitube"))
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// A lock that keeps working after a panic in another thread.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn id_chars(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn valid_id(vid: &str) -> bool {
    vid.len() == 11 && id_chars(vid)
}

/// A playlist someone made. "RD..." lists are YouTube's own Mixes (its
/// recommender picking videos for you), so those don't count.
pub fn valid_list(list: &str) -> bool {
    (12..=64).contains(&list.len()) && id_chars(list) && !list.starts_with("RD")
}

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn urlencode(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

/// Query string -> pairs. Empty values are dropped, like Python's parse_qs.
pub fn parse_query(q: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(q.as_bytes()).into_owned().filter(|(_, v)| !v.is_empty()).collect()
}

pub fn qget<'a>(q: &'a [(String, String)], key: &str) -> Option<&'a str> {
    q.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// Percent-encode like Python's urllib.parse.quote (keeps / - _ . ~).
pub fn quote_path(s: &str) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    const SET: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC.remove(b'/').remove(b'-').remove(b'_').remove(b'.').remove(b'~');
    utf8_percent_encode(s, SET).to_string()
}

pub fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// Local time from a unix timestamp.
pub fn local(t: f64) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(t as i64, 0).single().unwrap_or_else(chrono::Local::now)
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
