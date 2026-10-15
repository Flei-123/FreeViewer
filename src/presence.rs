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
    machine_name()
}

/// What the machine itself is called - without the own renaming.
///
/// Windows is asked directly (GetComputerNameExW), because a service does not
/// always inherit environment variables, and in some environments HOSTNAME
/// contains an IP address. An IP is useless as a device name - "192.168.1.24"
/// tells nobody which PC it is -, so it is discarded.
pub fn machine_name() -> String {
    #[cfg(windows)]
    {
        let n = clean(&win_computer_name());
        if !n.is_empty() && !looks_like_ip(&n) {
            return n;
        }
    }
    for key in ["FV_NAME", "COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(key) {
            let v = clean(&v);
            if !v.is_empty() && !looks_like_ip(&v) {
                return v;
            }
        }
    }
    if let Ok(out) = std::process::Command::new("hostname").output() {
        let v = clean(String::from_utf8_lossy(&out.stdout).trim());
        if !v.is_empty() && !looks_like_ip(&v) {
            return v;
        }
    }
    String::new()
}

/// "192.168.1.24" or "fe80::1" - anything that looks
/// like an address rather than a name.
pub fn looks_like_ip(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    if t.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    // 192.168.1.24:7180 or with a remainder behind it
    let head = t.split([':', '/', ' ']).next().unwrap_or("");
    head.parse::<std::net::Ipv4Addr>().is_ok()
}

/// The name Windows itself uses (DNS short name of the machine).
#[cfg(windows)]
fn win_computer_name() -> String {
    use windows::Win32::System::SystemInformation::{
        GetComputerNameExW, ComputerNamePhysicalDnsHostname,
    };
    use windows::core::PWSTR;
    unsafe {
        // the first call only fetches the length (it fails on purpose)
        let mut len: u32 = 0;
        let _ = GetComputerNameExW(ComputerNamePhysicalDnsHostname, PWSTR::null(), &mut len);
        if len == 0 || len > 512 {
            len = 256;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let mut have = buf.len() as u32;
        if GetComputerNameExW(
            ComputerNamePhysicalDnsHostname,
            PWSTR(buf.as_mut_ptr()),
            &mut have,
        )
        .is_err()
        {
            return String::new();
        }
        let n = (have as usize).min(buf.len());
        String::from_utf16_lossy(&buf[..n])
            .trim_end_matches('\0')
            .to_string()
    }
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
///
/// Without `token` the relay only says online/offline. When a FleiTec account
/// is signed in, the token (in the `Authorization` header, never in the URL)
/// additionally unlocks name and last-seen time - for the IDs in that
/// account's own address book (r6). Older relays ignore the header.
pub fn query(relay_url: &str, ids: &[String], token: &str) -> Result<HashMap<String, Presence>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let list: Vec<&str> = ids.iter().map(|s| s.as_str()).take(200).collect();
    let url = format!("{}?ids={}", online_url(relay_url), list.join(","));
    let mut rq = ureq::get(&url);
    if !token.is_empty() {
        rq = rq.header("authorization", format!("Bearer {}", token).as_str());
    }
    let mut resp = rq.call().map_err(|e| anyhow!("{}: {}", url, e))?;
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
                let token = crate::account::load().map(|a| a.token).unwrap_or_default();
                if let Ok(map) = query(&me.relay, &ids, &token) {
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
            online_url("wss://freeviewer.fleitec.com/fv/ws"),
            "https://freeviewer.fleitec.com/fv/online"
        );
        assert_eq!(
            online_url("ws://192.168.1.60:7180/fv/ws"),
            "http://192.168.1.60:7180/fv/online"
        );
    }

    /// One-shot HTTP server: answers with `body`, returns the request head it got.
    fn serve_online(body: &'static str) -> (u16, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 2048];
            while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = c.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            c.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        (port, h)
    }

    #[test]
    fn query_without_account_sends_no_token_and_reads_a_minimal_reply() {
        // new relay, anonymous: only {online}; name/seen default to empty
        let (port, h) = serve_online(r#"{"ids":{"123456789":{"online":true}}}"#);
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        let m = query(&relay, &["123456789".to_string()], "").unwrap();
        let req = h.join().unwrap().to_lowercase();
        assert!(req.starts_with("get /fv/online?ids=123456789 http/1.1"), "{}", req);
        assert!(!req.contains("authorization"), "{}", req);
        assert!(!req.contains("token"), "{}", req);
        let p = &m["123456789"];
        assert!(p.online && p.name.is_empty() && p.seen == 0);
    }

    #[test]
    fn query_with_account_puts_the_token_in_the_header_only() {
        let (port, h) = serve_online(r#"{"ids":{"1":{"online":false,"name":"Flei-ONE","seen":5}}}"#);
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        let m = query(&relay, &["1".to_string()], "tok-0123456789").unwrap();
        let req = h.join().unwrap().to_lowercase();
        assert!(req.starts_with("get /fv/online?ids=1 http/1.1"), "token must not be in the address: {}", req);
        assert!(req.contains("authorization: bearer tok-0123456789\r\n"), "{}", req);
        assert_eq!(m["1"].name, "Flei-ONE");
        assert_eq!(m["1"].seen, 5);
    }

    #[test]
    fn eine_ip_ist_kein_geraetename() {
        assert!(looks_like_ip("192.168.1.24"));
        assert!(looks_like_ip("192.168.1.24:7180"));
        assert!(looks_like_ip("::1"));
        assert!(!looks_like_ip("FLEI-ONE"));
        assert!(!looks_like_ip("Buero-PC 2"));
        assert!(!looks_like_ip(""));
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
