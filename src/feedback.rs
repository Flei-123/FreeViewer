//! Rückmeldung aus dem Programm heraus: Fehler melden oder Idee schicken.
//!
//! Geht als kleines JSON an den FleiTec-Server (derselbe Host, über den auch
//! die Updates kommen). Kein Konto, keine Anmeldung - und wenn der Server
//! gerade nicht erreichbar ist, sagt die Oberfläche das ehrlich.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

/// 0 = nichts läuft, 1 = wird gesendet, 2 = angekommen, 3 = schiefgegangen
pub static STATE: AtomicU8 = AtomicU8::new(0);
pub static MESSAGE: Mutex<String> = Mutex::new(String::new());

fn url() -> String {
    // aus wss://freeviewer.fleitec.com/fv/ws wird https://freeviewer.fleitec.com/fv-feedback
    let relay = std::env::var("FV_RELAY").unwrap_or_else(|_| crate::DEFAULT_RELAY.to_string());
    let host = relay
        .trim_start_matches("wss://")
        .trim_start_matches("ws://")
        .split('/')
        .next()
        .unwrap_or("freeviewer.fleitec.com")
        .to_string();
    let scheme = if relay.starts_with("ws://") { "http" } else { "https" };
    std::env::var("FV_FEEDBACK_URL").unwrap_or_else(|_| format!("{}://{}/fv-feedback", scheme, host))
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Baut den Rumpf der Meldung. Getrennt von send(), damit er prüfbar ist.
pub fn body(kind: &str, text: &str, contact: &str, id: &str, device: &str) -> String {
    format!(
        "{{\"kind\":\"{}\",\"text\":\"{}\",\"contact\":\"{}\",\"version\":\"{}\",\"id\":\"{}\",\"device\":\"{}\",\"os\":\"{}\"}}",
        json_escape(kind),
        json_escape(text.chars().take(4000).collect::<String>().trim()),
        json_escape(contact.chars().take(200).collect::<String>().trim()),
        json_escape(crate::update::VERSION),
        json_escape(id),
        json_escape(device),
        json_escape(if cfg!(windows) { "Windows" } else { "andere" }),
    )
}

/// Schickt die Meldung in einem eigenen Faden los.
pub fn send(kind: &str, text: &str, contact: &str, id: &str, device: &str) {
    if text.trim().is_empty() {
        STATE.store(3, Ordering::Relaxed);
        *MESSAGE.lock().unwrap() = "Bitte erst etwas hineinschreiben.".to_string();
        return;
    }
    let payload = body(kind, text, contact, id, device);
    let target = url();
    STATE.store(1, Ordering::Relaxed);
    *MESSAGE.lock().unwrap() = String::new();
    std::thread::spawn(move || {
        let res = ureq::post(&target)
            .header("Content-Type", "application/json")
            .send(payload.as_bytes());
        match res {
            Ok(r) if r.status() == 200 => {
                STATE.store(2, Ordering::Relaxed);
                *MESSAGE.lock().unwrap() = "Danke - ist angekommen.".to_string();
            }
            Ok(r) => {
                STATE.store(3, Ordering::Relaxed);
                *MESSAGE.lock().unwrap() = format!("Server sagt {}", r.status());
            }
            Err(e) => {
                STATE.store(3, Ordering::Relaxed);
                *MESSAGE.lock().unwrap() = format!("Nicht erreichbar: {}", e);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_is_valid_json_even_with_quotes_and_newlines() {
        let b = body("fehler", "Zeile1\nZei\"le2\\", "a@b.c", "123456789", "PC");
        let v: serde_json::Value = serde_json::from_str(&b).expect("kein JSON");
        assert_eq!(v["kind"], "fehler");
        assert_eq!(v["text"], "Zeile1\nZei\"le2\\");
        assert_eq!(v["id"], "123456789");
    }

    #[test]
    fn text_is_capped() {
        let long = "x".repeat(9000);
        let b = body("idee", &long, "", "", "");
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["text"].as_str().unwrap().len(), 4000);
    }

    #[test]
    fn url_follows_the_relay_host() {
        std::env::remove_var("FV_FEEDBACK_URL");
        std::env::set_var("FV_RELAY", "wss://beispiel.test/fv/ws");
        assert_eq!(url(), "https://beispiel.test/fv-feedback");
        std::env::set_var("FV_RELAY", "ws://192.168.1.60:7180/fv/ws");
        assert_eq!(url(), "http://192.168.1.60:7180/fv-feedback");
        std::env::remove_var("FV_RELAY");
    }
}
