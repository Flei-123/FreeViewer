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
    format!("{}/account/{}", base_url(relay_url), what)
}

/// `wss://host/fv/ws` -> `https://host/fv/<was>` - OHNE /account/ dazwischen.
/// Die Einrichtungs-Endpunkte (/fv/setup/*) haengen direkt an der Basis.
pub fn plain_url(relay_url: &str, what: &str) -> String {
    format!("{}/{}", base_url(relay_url), what)
}

fn base_url(relay_url: &str) -> String {
    let http = if let Some(rest) = relay_url.strip_prefix("wss://") {
        format!("https://{}", rest)
    } else if let Some(rest) = relay_url.strip_prefix("ws://") {
        format!("http://{}", rest)
    } else {
        relay_url.to_string()
    };
    match http.rfind("/ws") {
        Some(i) if i + 3 == http.len() => http[..i].to_string(),
        _ => http.trim_end_matches('/').to_string(),
    }
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


// --------------------------------------------------------- Einrichtungs-Links
//
// Der Installation-Tab erzeugt am Relay einen Einmal-Code; ein frisches
// Geraet loest ihn genau einmal ein und bekommt das gewaehlte Passwort.
// Danach liegt das Geraet (ohne Passwort) im Konto-Adressbuch, das Passwort
// holt sich der Besitzer einmalig ueber die Inbox.

#[derive(Serialize)]
struct SetupCreateReq<'a> {
    password: &'a str,
    name_hint: &'a str,
    max_uses: u32,
}

#[derive(Deserialize, Default)]
struct SetupCreateReply {
    #[serde(default)]
    code: String,
    #[serde(default)]
    link: String,
    #[serde(default)]
    web: String,
    #[serde(default)]
    error: String,
}

/// Ein noch nicht eingeloester Code (Anzeige im Tab).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SetupPending {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub name_hint: String,
    #[serde(default)]
    pub at: u64,
    /// Wie viele Geraete der Link insgesamt einrichten darf.
    #[serde(default = "one")]
    pub max_uses: u32,
    /// Wie viele schon eingerichtet sind.
    #[serde(default)]
    pub uses: u32,
}

fn one() -> u32 {
    1
}

/// Die Web-Adresse eines Einrichtungs-Codes - dieselbe, die das Relay beim
/// Erzeugen zurueckgibt. Damit steht der Link auch bei jedem wartenden
/// Eintrag in der Liste, nicht nur direkt nach dem Erzeugen.
pub fn setup_web_link(code: &str) -> String {
    format!("{}/setup/{}", crate::brand::WEB, code)
}

/// Ein frisch eingerichtetes Geraet - inklusive Passwort (nur einmal sichtbar).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SetupClaimed {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub at: u64,
}

#[derive(Deserialize, Default)]
struct SetupInboxReply {
    #[serde(default)]
    pending: Vec<SetupPending>,
    #[serde(default)]
    claimed: Vec<SetupClaimed>,
    #[serde(default)]
    error: String,
}

#[derive(Serialize)]
struct SetupClaimReq<'a> {
    code: &'a str,
    id: &'a str,
    name: &'a str,
}

/// Was das frische Geraet beim Einloesen des Codes zurueckbekommt.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SetupClaimReply {
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub name_hint: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub error: String,
}

/// Ein GET mit JSON-Antwort (die Inbox ist ein GET).
fn get_json(target: &str) -> Result<(u16, String)> {
    match ureq::get(target).call() {
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

/// Einmal-Code erzeugen (braucht die Konto-Anmeldung).
pub fn setup_create(
    relay_url: &str,
    token: &str,
    password: &str,
    name_hint: &str,
    max_uses: u32,
) -> Result<(String, String, String)> {
    let target = format!("{}?token={}", plain_url(relay_url, "setup/create"), urlenc(token));
    let body = serde_json::to_string(&SetupCreateReq {
        password,
        name_hint,
        max_uses,
    })
    .map_err(|e| anyhow!("{}", e))?;
    let (status, text) = post_json(&target, &body)?;
    let r: SetupCreateReply = serde_json::from_str(&text).unwrap_or_default();
    if status == 401 {
        return Err(anyhow!("nicht angemeldet"));
    }
    if status != 200 || r.code.is_empty() {
        let msg = if r.error.is_empty() {
            format!("Relay meldet Fehler {}", status)
        } else {
            r.error
        };
        return Err(anyhow!(msg));
    }
    Ok((r.code, r.link, r.web))
}

/// Posteingang des Besitzers: wartende Codes + frisch eingerichtete Geraete.
pub fn setup_inbox(relay_url: &str, token: &str) -> Result<(Vec<SetupPending>, Vec<SetupClaimed>)> {
    let target = format!("{}?token={}", plain_url(relay_url, "setup/inbox"), urlenc(token));
    let (status, text) = get_json(&target)?;
    if status == 401 {
        return Err(anyhow!("nicht angemeldet"));
    }
    if status != 200 {
        return Err(anyhow!("Relay meldet Fehler {}", status));
    }
    let r: SetupInboxReply =
        serde_json::from_str(&text).map_err(|e| anyhow!("Antwort unverstaendlich: {}", e))?;
    if !r.error.is_empty() {
        return Err(anyhow!(r.error));
    }
    Ok((r.pending, r.claimed))
}

/// Die andere Seite: das frische Geraet loest den Code ein (ohne Anmeldung).
pub fn setup_claim(relay_url: &str, code: &str, id: &str, name: &str) -> Result<SetupClaimReply> {
    let body = serde_json::to_string(&SetupClaimReq { code, id, name })
        .map_err(|e| anyhow!("{}", e))?;
    let (status, text) = post_json(&plain_url(relay_url, "setup/claim"), &body)?;
    let r: SetupClaimReply = serde_json::from_str(&text).unwrap_or_default();
    if status != 200 {
        let msg = if r.error.is_empty() {
            match status {
                404 => "Code unbekannt oder abgelaufen".to_string(),
                410 => "Code wurde schon verwendet".to_string(),
                _ => format!("Relay meldet Fehler {}", status),
            }
        } else {
            r.error
        };
        return Err(anyhow!(msg));
    }
    Ok(r)
}

#[derive(Serialize)]
struct SetupRevokeReq<'a> {
    code: &'a str,
}

/// Einen wartenden Link widerrufen - schon eingerichtete Geraete behalten
/// ihre Einrichtung, der Rest des Links wird sofort unbrauchbar.
pub fn setup_revoke(relay_url: &str, token: &str, code: &str) -> Result<()> {
    let target = format!("{}?token={}", plain_url(relay_url, "setup/revoke"), urlenc(token));
    let body = serde_json::to_string(&SetupRevokeReq { code }).map_err(|e| anyhow!("{}", e))?;
    let (status, _text) = post_json(&target, &body)?;
    match status {
        200 => Ok(()),
        401 => Err(anyhow!("nicht angemeldet")),
        404 => Err(anyhow!("Code unbekannt")),
        _ => Err(anyhow!("Relay meldet Fehler {}", status)),
    }
}

/// Widerrufen im Hintergrund (das Ergebnis holt die naechste Inbox-Runde).
pub fn setup_revoke_async(relay_url: String, token: String, code: String) {
    std::thread::spawn(move || {
        let _ = setup_revoke(&relay_url, &token, &code);
    });
}

/// Was ein Einrichtungslauf im Hintergrund zurueckmeldet.
#[derive(Debug, Clone)]
pub enum SetupOut {
    /// Code erzeugt: (code, freeviewer://-Link, Web-Link).
    Created { code: String, link: String, web: String },
    /// Posteingang: wartende Codes, frisch eingerichtete Geraete.
    Inbox { pending: Vec<SetupPending>, claimed: Vec<SetupClaimed> },
    Failed(String),
}

/// Code erzeugen im Hintergrund (Oberflaeche wartet nie).
pub fn setup_create_async(
    relay_url: String,
    token: String,
    password: String,
    name_hint: String,
    max_uses: u32,
    out: std::sync::Arc<std::sync::Mutex<Option<SetupOut>>>,
    busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;
    if busy.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let res = match setup_create(&relay_url, &token, &password, &name_hint, max_uses) {
            Ok((code, link, web)) => SetupOut::Created { code, link, web },
            Err(e) => SetupOut::Failed(e.to_string()),
        };
        *out.lock().unwrap() = Some(res);
        busy.store(false, Ordering::SeqCst);
    });
}

/// Posteingang holen im Hintergrund.
pub fn setup_inbox_async(
    relay_url: String,
    token: String,
    out: std::sync::Arc<std::sync::Mutex<Option<SetupOut>>>,
    busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;
    if busy.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let res = match setup_inbox(&relay_url, &token) {
            Ok((pending, claimed)) => SetupOut::Inbox { pending, claimed },
            Err(e) => SetupOut::Failed(e.to_string()),
        };
        *out.lock().unwrap() = Some(res);
        busy.store(false, Ordering::SeqCst);
    });
}

/// URLs: Konto-Endpunkte unter /account/, Einrichtung direkt an der Basis.
#[cfg(test)]
mod url_tests {
    #[test]
    fn setup_urls_liegen_nicht_unter_account() {
        assert_eq!(
            super::plain_url("wss://freeviewer.fleitec.com/fv/ws", "setup/create"),
            "https://freeviewer.fleitec.com/fv/setup/create"
        );
        assert_eq!(
            super::url("wss://freeviewer.fleitec.com/fv/ws", "login"),
            "https://freeviewer.fleitec.com/fv/account/login"
        );
    }
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
