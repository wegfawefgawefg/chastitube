//! `sudo chastitube firefox-policy [file]`: adds what chastitube needs to Firefox's
//! policy file (default /etc/firefox/policies/policies.json), keeping
//! everything else in it:
//!   - typing `chastitube` in the address bar opens http://chastitube/ instead of
//!     searching for the word (browser.fixup.domainwhitelist.chastitube)
//!   - chastitube's pages may autoplay with sound
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

use crate::util::{local_base, NAME_URL};

const DEFAULT_PATH: &str = "/etc/firefox/policies/policies.json";

/// The object at `key` inside `v`, created if missing (or replaced if it isn't an object).
fn object<'a>(v: &'a mut Value, key: &str) -> &'a mut Map<String, Value> {
    if !v[key].is_object() {
        v[key] = json!({});
    }
    v[key].as_object_mut().expect("just made it an object")
}

pub fn firefox_policy(path: Option<&str>) {
    let path = Path::new(path.unwrap_or(DEFAULT_PATH));
    let mut doc: Value = fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}));
    if !doc.is_object() {
        doc = json!({});
    }
    let policies = object(&mut doc, "policies");
    let mut p = Value::Object(std::mem::take(policies));

    object(&mut p, "Preferences").insert("browser.fixup.domainwhitelist.chastitube".into(),
                                         json!({"Value": true, "Status": "default"}));
    let perms = object(&mut p, "Permissions");
    let mut ap = perms.remove("Autoplay").filter(Value::is_object).unwrap_or_else(|| json!({}));
    if !ap["Allow"].is_array() {
        ap["Allow"] = json!([]);
    }
    let allow = ap["Allow"].as_array_mut().expect("just made it an array");
    for site in [NAME_URL.to_string(), local_base()] {
        if !allow.iter().any(|a| a.as_str() == Some(&site)) {
            allow.push(json!(site));
        }
    }
    perms.insert("Autoplay".into(), ap);

    doc["policies"] = p;
    let text = serde_json::to_string_pretty(&doc).expect("json") + "\n";
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.chastitube-tmp");
    match fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, path)) {
        Ok(()) => println!("Updated {} (restart Firefox to use it)", path.display()),
        Err(e) => {
            eprintln!("Couldn't write {}: {e} (run it with sudo)", path.display());
            std::process::exit(1);
        }
    }
}
