//! Who is online, and what is that machine called?
//!
//! The relay keeps a tiny directory next to the ID table: the name a host
//! reports about itself and when it was last seen. That is everything a
//! partner list needs to look like TeamViewer's - and it is deliberately
//! *all* the relay learns. Screens, input and passwords stay end-to-end
//! encrypted, the directory holds no secrets.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use serde::Deserialize;

/// What the relay knows about one ID.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Presence {
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub name: String,
    /// last time the host was connected, unix milliseconds
    #[serde(default)]
    pub seen: u64,
}

#[derive(Debug, Deserialize)]
struct OnlineReply {
    ids: HashMap<String, Presence>,
}

/// Where a name chosen by the user is kept.
pub fn name_file() -> std::path::PathBuf {
    crate::ident::config_dir().join("name.txt")
}

/// Stores the name this machine reports about itself.
pub fn save_device_name(name: &str) -> std::io::Result<()> {
    let clean = clean(name);
    if clean.is_empty() {
        match std::fs::remove_file(name_file()) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    } else {
        let _ = std::fs::create_dir_all(crate::ident::config_dir());
        std::fs::write(name_file(), clean)
    }
}

/// Name of this machine, as shown in somebody else's partner list.
pub fn device_name() -> String {
    if let Ok(s) = std::fs::read_to_string(name_file()) {
        let s = clean(&s);
        if !s.is_empty() {
            return s;
        }
    }
    for key in ["FV_NAME", "COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(key) {
            let v = clean(&v);
            if !v.is_empty() {
                return v;
            }
        }
    }
    #[cfg(windows)]
    {
        // services do not inherit COMPUTERNAME in every case
        if let Ok(out) = std::process::Command::new("hostname").output() {
            let v = clean(String::from_utf8_lossy(&out.stdout).trim());
            if !v.is_empty() {
                return v;
            }
        }
    }
    String::new()
}

/// Keeps the name harmless for JSON and for the relay's own filter.
pub fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
        .take(40)
        .collect::<String>()
        .trim()
        .to_string()
}

/// `wss://host/fv/ws` -> `https://host/fv/online`
pub fn online_url(relay_url: &str) -> String {
    let http = if let Some(rest) = relay_url.strip_prefix("wss://") {
        format!("https://{}", rest)
    } else if let Some(rest) = relay_url.strip_prefix("ws://") {
        format!("http://{}", rest)
    } else {
        relay_url.to_string()
    };
    match http.rfind("/ws") {
        Some(i) if i + 3 == http.len() => format!("{}/online", &http[..i]),
        _ => format!("{}/online", http.trim_end_matches('/')),
    }
}

/// Asks the relay about a bunch of IDs at once.
pub fn query(relay_url: &str, ids: &[String]) -> Result<HashMap<String, Presence>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let list: Vec<&str> = ids.iter().map(|s| s.as_str()).take(200).collect();
    let url = format!("{}?ids={}", online_url(relay_url), list.join(","));
    let mut resp = ureq::get(&url)
        .call()
        .map_err(|e| anyhow!("{}: {}", url, e))?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow!("Antwort: {}", e))?;
    let reply: OnlineReply = serde_json::from_str(&body)?;
    Ok(reply.ids)
}

/// Cache in front of the relay so the GUI can ask every frame.
pub struct Watch {
    state: Mutex<HashMap<String, Presence>>,
    wanted: Mutex<Vec<String>>,
    running: AtomicBool,
    relay: String,
}

impl Watch {
    pub fn new(relay: String) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(HashMap::new()),
            wanted: Mutex::new(Vec::new()),
            running: AtomicBool::new(false),
            relay,
        })
    }

    /// Which IDs the list currently shows. Cheap, called from the GUI.
    pub fn watch(&self, ids: Vec<String>) {
        *self.wanted.lock().unwrap() = ids;
    }

    pub fn get(&self, id: &str) -> Option<Presence> {
        self.state.lock().unwrap().get(id).cloned()
    }

    pub fn online(&self, id: &str) -> bool {
        self.get(id).map(|p| p.online).unwrap_or(false)
    }

    /// Refreshes every few seconds in the background.
    pub fn start(self: &Arc<Self>) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        std::thread::spawn(move || loop {
            let ids = me.wanted.lock().unwrap().clone();
            if !ids.is_empty() {
                if let Ok(map) = query(&me.relay, &ids) {
                    let mut st = me.state.lock().unwrap();
                    for (k, v) in map {
                        st.insert(k, v);
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(5));
        });
    }
}

/// "vor 3 Min." for a unix millisecond stamp - same wording as the address
/// book so the two lists read alike.
pub fn ago_ms(ms: u64) -> String {
    if ms == 0 {
        return "noch nie".to_string();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let d = now.saturating_sub(ms) / 1000;
    match d {
        0..=59 => "gerade eben".to_string(),
        60..=3599 => format!("vor {} Min.", d / 60),
        3600..=86399 => format!("vor {} Std.", d / 3600),
        86400..=172_799 => "gestern".to_string(),
        _ => format!("vor {} Tagen", d / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_url_is_derived_from_the_relay_url() {
        assert_eq!(
            online_url("wss://jarvis.fleitec.com/fv/ws"),
            "https://jarvis.fleitec.com/fv/online"
        );
        assert_eq!(
            online_url("ws://192.168.1.60:7180/fv/ws"),
            "http://192.168.1.60:7180/fv/online"
        );
    }

    #[test]
    fn names_stay_harmless() {
        assert_eq!(clean("FLEI-ONE"), "FLEI-ONE");
        assert_eq!(clean("  Pa\"tis {Laptop}  "), "Patis Laptop");
        assert_eq!(clean(&"x".repeat(80)).len(), 40);
    }

    #[test]
    fn ago_reads_like_the_address_book() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert_eq!(ago_ms(0), "noch nie");
        assert_eq!(ago_ms(now - 5_000), "gerade eben");
        assert_eq!(ago_ms(now - 300_000), "vor 5 Min.");
        assert_eq!(ago_ms(now - 7_200_000), "vor 2 Std.");
    }
}
