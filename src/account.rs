//! Das FleiTec-Konto - freiwillig.
//!
//! FreeViewer laeuft ohne Anmeldung vollstaendig: ID, Passwort, Adressbuch,
//! alles liegt auf dem Rechner. Wer sich anmeldet, bekommt genau eine Sache
//! dazu - seine Geraeteliste liegt danach auch beim Konto und ist auf jedem
//! Rechner dieselbe.
//!
//! Was NICHT hochgeht: gespeicherte Passwoerter. Die sind mit der Kennung
//! dieses Rechners verschluesselt (siehe partners.rs) und waeren woanders
//! nicht zu gebrauchen; ausserdem soll der Server so wenig wie moeglich
//! wissen.
//!
//! Angemeldet wird nicht bei uns, sondern beim JARVIS-Konto - derselbe Name,
//! dasselbe Passwort wie auf der Webseite. Der Relay reicht die Anmeldung nur
//! durch und fragt spaeter nach, zu wem ein Zeichen (Token) gehoert.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::partners::SyncDevice;

/// Was auf der Platte liegt, wenn jemand angemeldet ist.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub user: String,
    pub token: String,
    /// Wann zuletzt erfolgreich abgeglichen wurde (unix Sekunden).
    #[serde(default)]
    pub synced: u64,
}

/// Bewusst NICHT im gemeinsamen Ordner (ProgramData): dort duerfen alle
/// Windows-Konten dieses Rechners hinein, und das Zeichen (Token) gilt fuer
/// das ganze FleiTec-Konto. Die Anmeldung gehoert deshalb ins Profil des
/// Menschen, der sich angemeldet hat.
pub fn file() -> PathBuf {
    crate::ident::user_config_dir().join("account.json")
}

pub fn load() -> Option<Session> {
    let raw = std::fs::read_to_string(file()).ok()?;
    let s: Session = serde_json::from_str(&raw).ok()?;
    if s.token.is_empty() {
        None
    } else {
        Some(s)
    }
}

pub fn save(s: &Session) -> std::io::Result<()> {
    let _ = std::fs::create_dir_all(crate::ident::user_config_dir());
    let json = serde_json::to_string_pretty(s).unwrap_or_default();
    std::fs::write(file(), json)
}

/// Abmelden - der Zettel wird weggeworfen, das Adressbuch bleibt liegen.
pub fn forget() {
    let _ = std::fs::remove_file(file());
}

/// `wss://host/fv/ws` -> `https://host/fv/account/<was>`
pub fn url(relay_url: &str, what: &str) -> String {
    let http = if let Some(rest) = relay_url.strip_prefix("wss://") {
        format!("https://{}", rest)
    } else if let Some(rest) = relay_url.strip_prefix("ws://") {
        format!("http://{}", rest)
    } else {
        relay_url.to_string()
    };
    let base = match http.rfind("/ws") {
        Some(i) if i + 3 == http.len() => http[..i].to_string(),
        _ => http.trim_end_matches('/').to_string(),
    };
    format!("{}/account/{}", base, what)
}

#[derive(Serialize)]
struct LoginReq<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Deserialize)]
struct LoginReply {
    #[serde(default)]
    token: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    error: String,
}

#[derive(Serialize)]
struct SyncReq {
    devices: Vec<SyncDevice>,
}

#[derive(Deserialize)]
pub struct SyncReply {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub rev: u64,
    #[serde(default)]
    pub devices: Vec<SyncDevice>,
    #[serde(default)]
    pub error: String,
}

/// Ein POST mit JSON. ureq meldet 4xx/5xx als Fehler - hier wird daraus
/// wieder ein Zahlenwert, damit "falsches Passwort" anders klingen kann als
/// "Server nicht erreichbar".
fn post_json(target: &str, body: &str) -> Result<(u16, String)> {
    match ureq::post(target)
        .header("content-type", "application/json")
        .send(body)
    {
        Ok(mut r) => {
            let code = r.status().as_u16();
            let text = r
                .body_mut()
                .read_to_string()
                .map_err(|e| anyhow!("Antwort unlesbar: {}", e))?;
            Ok((code, text))
        }
        Err(ureq::Error::StatusCode(code)) => Ok((code, String::new())),
        Err(e) => Err(anyhow!("nicht erreichbar: {}", e)),
    }
}

/// Anmelden. Gibt die Sitzung zurueck, die dann gespeichert werden kann.
pub fn login(relay_url: &str, user: &str, pass: &str) -> Result<Session> {
    let body = serde_json::to_string(&LoginReq {
        username: user.trim(),
        password: pass,
    })
    .map_err(|e| anyhow!("{}", e))?;
    let (status, text) = post_json(&url(relay_url, "login"), &body)?;
    let reply: LoginReply = serde_json::from_str(&text).unwrap_or(LoginReply {
        token: String::new(),
        username: String::new(),
        error: String::new(),
    });
    if status != 200 || reply.token.is_empty() {
        let msg = if reply.error.is_empty() {
            match status {
                401 => "Benutzername oder Passwort stimmt nicht".to_string(),
                429 => "Zu viele Versuche - bitte kurz warten".to_string(),
                _ => format!("Anmeldung fehlgeschlagen ({})", status),
            }
        } else {
            reply.error
        };
        return Err(anyhow!(msg));
    }
    Ok(Session {
        user: if reply.username.is_empty() {
            user.trim().to_string()
        } else {
            reply.username
        },
        token: reply.token,
        synced: 0,
    })
}

/// Abgleich in einem Rutsch: eigene Liste hoch, zusammengefuehrte zurueck.
pub fn sync(relay_url: &str, token: &str, devices: Vec<SyncDevice>) -> Result<SyncReply> {
    let target = format!("{}?token={}", url(relay_url, "data"), urlenc(token));
    let body = serde_json::to_string(&SyncReq { devices }).map_err(|e| anyhow!("{}", e))?;
    let (status, text) = post_json(&target, &body)?;
    if status == 401 {
        return Err(anyhow!("nicht angemeldet"));
    }
    if status != 200 {
        return Err(anyhow!("Konto meldet Fehler {}", status));
    }
    let reply: SyncReply =
        serde_json::from_str(&text).map_err(|e| anyhow!("Antwort unverstaendlich: {}", e))?;
    if !reply.error.is_empty() {
        return Err(anyhow!(reply.error));
    }
    Ok(reply)
}

/// Nur das Noetigste - Zeichen (Token) bestehen aus URL-sicheren Zeichen,
/// aber sicher ist sicher.
fn urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Was ein Abgleich im Hintergrund zurueckmeldet.
#[derive(Debug, Clone)]
pub enum SyncOut {
    /// Fertig: so viele Geraete kamen zurueck, so viele sind es jetzt.
    Ok { devices: Vec<SyncDevice>, at: u64 },
    /// Anmeldung hat geklappt (Name des Kontos).
    LoggedIn(String),
    /// Zeichen abgelaufen - der Nutzer muss sich neu anmelden.
    LoggedOut,
    Failed(String),
}

/// Abgleich im Hintergrund. Der Aufrufer bekommt das Ergebnis ueber den
/// Briefkasten (Mutex), damit die Oberflaeche nie wartet.
pub fn sync_async(
    relay_url: String,
    token: String,
    devices: Vec<SyncDevice>,
    out: std::sync::Arc<std::sync::Mutex<Option<SyncOut>>>,
    busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;
    if busy.swap(true, Ordering::SeqCst) {
        return; // laeuft schon
    }
    std::thread::spawn(move || {
        let res = match sync(&relay_url, &token, devices) {
            Ok(r) => SyncOut::Ok {
                devices: r.devices,
                at: now(),
            },
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("nicht angemeldet") {
                    SyncOut::LoggedOut
                } else {
                    SyncOut::Failed(msg)
                }
            }
        };
        *out.lock().unwrap() = Some(res);
        busy.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn die_adresse_kommt_vom_relay() {
        assert_eq!(
            url("wss://freeviewer.fleitec.com/fv/ws", "login"),
            "https://freeviewer.fleitec.com/fv/account/login"
        );
        assert_eq!(
            url("ws://192.168.1.60:7180/fv/ws", "data"),
            "http://192.168.1.60:7180/fv/account/data"
        );
    }

    #[test]
    fn zeichen_werden_sauber_verpackt() {
        assert_eq!(urlenc("abc-1_2.3~x"), "abc-1_2.3~x");
        assert_eq!(urlenc("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn ohne_zeichen_gilt_niemand_als_angemeldet() {
        let s = Session {
            user: "Justin".into(),
            token: String::new(),
            synced: 0,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&json).unwrap();
        assert!(back.token.is_empty());
    }
}
