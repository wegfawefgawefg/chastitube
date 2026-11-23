//! Open chastitube tabs. A link from another app goes to a chastitube tab
//! you can see instead of opening a new one.
//!
//! Every page keeps an EventSource open on /api/events. To open a link, the
//! server offers it to every tab at once; a tab that's on screen answers on
//! /api/ack/<n>, and the first to answer navigates. Tabs in the background ignore it, and if
//! nobody answers, the link handler opens a new tab as before.
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::util::lock;

const ACK_WAIT: Duration = Duration::from_millis(800); // to wait for each tab to answer

struct Tabs {
    next_tab: u64,
    tabs: Vec<(u64, Sender<String>)>, // newest last
    acks: HashMap<u64, Sender<()>>,
}

fn tabs() -> &'static Mutex<Tabs> {
    static T: OnceLock<Mutex<Tabs>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(Tabs { next_tab: 0, tabs: Vec::new(), acks: HashMap::new() }))
}

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Runs for as long as a tab is open, writing the event stream itself.
pub fn stream(mut out: Box<dyn Write + Send>) {
    let (tx, rx): (Sender<String>, Receiver<String>) = channel();
    let me = {
        let mut t = lock(tabs());
        t.next_tab += 1;
        let me = t.next_tab;
        t.tabs.push((me, tx));
        me
    };
    let mut write = |data: &[u8]| out.write_all(data).and_then(|_| out.flush());
    let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nretry: 1000\n\n";
    let mut ok = write(head).is_ok();
    while ok {
        ok = match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(msg) => write(format!("data: {msg}\n\n").as_bytes()).is_ok(),
            Err(RecvTimeoutError::Timeout) => write(b": ping\n\n").is_ok(),
            Err(RecvTimeoutError::Disconnected) => false,
        };
    }
    lock(tabs()).tabs.retain(|(id, _)| *id != me);
}

/// Ask the tab that's on screen to open path. Every tab is asked at once;
/// the first to answer gets it (the others are told no and stay put).
/// True if one did.
pub fn send(path: &str) -> bool {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = channel();
    let msg = serde_json::json!({"n": n, "path": path}).to_string();
    {
        let mut t = lock(tabs());
        t.acks.insert(n, tx);
        for (_, tab) in t.tabs.iter().rev() {
            let _ = tab.send(msg.clone());
        }
    }
    let answered = rx.recv_timeout(ACK_WAIT).is_ok();
    lock(tabs()).acks.remove(&n);
    answered
}

/// A tab offering to open message n. Only the first one gets a yes.
pub fn ack(n: u64) -> bool {
    match lock(tabs()).acks.remove(&n) {
        Some(tx) => tx.send(()).is_ok(),
        None => false,
    }
}
