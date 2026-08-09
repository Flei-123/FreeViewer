//! Freunde - Freundschaftsanfragen, Freundesliste, Ein-Klick-Zugriff.
//!
//! Der Unterschied zum Adressbuch (`partners.rs`): dort traegt jeder fuer sich
//! ein, wen er kennt - niemand muss zustimmen. Eine FREUNDSCHAFT entsteht nur,
//! wenn BEIDE Seiten zugestimmt haben. Erst dann sieht man einander in der
//! Liste, sieht wer online ist, und kann mit einem Klick ein Meeting starten
//! oder um Fernwartung bitten.
//!
//! Wo was liegt:
//!
//! * Der Relay fuehrt die Wahrheit: `/fv/freunde/*` (siehe relay.js). Er ist
//!   der einzige Weg, auf dem eine Anfrage den anderen ueberhaupt erreicht.
//! * Diese Datei haelt eine KOPIE davon neben den anderen Einstellungen
//!   (`freunde.json` im Ordner aus `ident::config_dir()`, genau wie
//!   `partners.json`). Damit steht die Liste sofort auf dem Schirm, auch wenn
//!   der Relay gerade nicht erreichbar ist.
//!
//! Ohne Konto haengt alles an der FreeViewer-ID dieses Rechners. Mit Konto
//! traegt der Relay zusaetzlich den Kontonamen ein - dann ist dieselbe
//! Freundesliste auf jedem angemeldeten Geraet zu sehen. Beides gleichzeitig
//! ist der Normalfall: die ID gilt immer, das Konto kommt obendrauf.
//!
//! Ausweisen tut sich der Client genau wie beim WebSocket-Anmelden
//! (`net::json_register`): mit dem Geheimnis aus `identity.txt`. Es steht
//! IMMER im Rumpf der Anfrage, nie in der Adresse.
//!
//! Nichts hier blockiert die Oberflaeche. Jeder Netzaufruf laeuft in einem
//! eigenen Faden; Ergebnisse kommen ueber `Dienst::meldung()` zurueck - dasselbe
//! Briefkasten-Muster wie in `account.rs` und `presence.rs`.

// Das Modul ist vollstaendig, die Oberflaeche wird getrennt angebunden.
// Bis dahin sind die oeffentlichen Bausteine aus Sicht des Binaerprogramms
// "unbenutzt" - ohne diese Zeile stuenden hier dutzende Warnungen.
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Wie oft im Hintergrund abgeglichen wird, wenn nichts los ist.
const ABGLEICH_SEKUNDEN: u64 = 20;

/// Obergrenze, damit eine kaputte Antwort die Datei nicht sprengt.
const MAX_FREUNDE: usize = 500;
const MAX_ANFRAGEN: usize = 200;

// ============================================================== Datentypen

/// Ein bestaetigter Freund. Beide haben zugestimmt.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Freund {
    /// Die FreeViewer-ID des anderen - damit geht Meeting und Fernwartung.
    pub fvid: String,
    /// Wie der andere sich nennt (leer = ID anzeigen).
    #[serde(default)]
    pub name: String,
    /// Seit wann befreundet, unix Sekunden.
    #[serde(default)]
    pub seit: u64,
    /// Ist er gerade erreichbar? Kommt vom Relay.
    #[serde(default)]
    pub online: bool,
    /// Zuletzt gesehen, unix MILLIsekunden (wie in `presence.rs`).
    #[serde(default)]
    pub gesehen: u64,
}

impl Freund {
    /// Was in der Liste steht: Name, sonst die huebsch gesetzte ID.
    pub fn anzeige(&self) -> String {
        if self.name.trim().is_empty() {
            crate::partners::pretty_id(&self.fvid)
        } else {
            self.name.clone()
        }
    }

    /// "gerade eben", "vor 3 Min." - gleiche Sprache wie das Adressbuch.
    pub fn zuletzt(&self) -> String {
        if self.online {
            "online".to_string()
        } else {
            crate::presence::ago_ms(self.gesehen)
        }
    }

    /// Kann man ihn jetzt sofort anklingeln?
    pub fn erreichbar(&self) -> bool {
        self.online
    }
}

/// Wer hat gefragt - ich oder der andere?
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Richtung {
    /// Jemand will mich als Freund. Ich muss annehmen oder ablehnen.
    Eingehend,
    /// Ich habe gefragt und warte auf die Antwort.
    Ausgehend,
}

impl Default for Richtung {
    fn default() -> Self {
        Richtung::Eingehend
    }
}

/// Eine offene Freundschaftsanfrage.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Anfrage {
    /// Die ANDERE Seite: bei `Eingehend` der Absender, bei `Ausgehend` der
    /// Empfaenger. (Der Name ist so gewaehlt, weil die Liste fast immer die
    /// eingehenden zeigt - dort ist es woertlich "von wem".)
    #[serde(rename = "fvid", alias = "von_fvid")]
    pub von_fvid: String,
    /// Wie der andere sich nennt.
    #[serde(default)]
    pub name: String,
    /// Wann gestellt, unix Sekunden.
    #[serde(default)]
    pub wann: u64,
    pub richtung: Richtung,
    /// Freier Gruss des Absenders (darf leer sein).
    #[serde(default)]
    pub nachricht: String,
    /// Ist der andere gerade online?
    #[serde(default)]
    pub online: bool,
}

impl Anfrage {
    /// Die ID der anderen Seite - unabhaengig von der Richtung.
    pub fn gegenueber(&self) -> &str {
        &self.von_fvid
    }

    pub fn anzeige(&self) -> String {
        if self.name.trim().is_empty() {
            crate::partners::pretty_id(&self.von_fvid)
        } else {
            self.name.clone()
        }
    }

    pub fn ist_eingehend(&self) -> bool {
        self.richtung == Richtung::Eingehend
    }
}

/// Ein Eintrag der Blockliste.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Blockiert {
    pub fvid: String,
    #[serde(default)]
    pub seit: u64,
}

/// Was schiefgehen kann. Alle Meldungen sind fertige deutsche Saetze und
/// koennen direkt angezeigt werden.
#[derive(Debug, Clone, PartialEq)]
pub enum Fehler {
    /// Die eigene ID kann nicht der eigene Freund sein.
    SelbstAnfrage,
    SchonBefreundet,
    SchonAngefragt,
    /// Annehmen/Ablehnen ohne dass eine Anfrage vorliegt.
    KeineAnfrage,
    KeinFreund,
    Blockiert,
    UngueltigeId,
    /// Der Relay hat abgelehnt oder war nicht erreichbar.
    Netz(String),
}

impl std::fmt::Display for Fehler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Fehler::SelbstAnfrage => "Das ist die eigene FreeViewer-ID".to_string(),
            Fehler::SchonBefreundet => "Ihr seid schon befreundet".to_string(),
            Fehler::SchonAngefragt => "Die Anfrage laeuft schon".to_string(),
            Fehler::KeineAnfrage => "Von dieser ID liegt keine Anfrage vor".to_string(),
            Fehler::KeinFreund => "Diese ID steht nicht in der Freundesliste".to_string(),
            Fehler::Blockiert => "Diese ID steht auf der Blockliste".to_string(),
            Fehler::UngueltigeId => "Das ist keine gueltige FreeViewer-ID".to_string(),
            Fehler::Netz(m) => m.clone(),
        };
        write!(f, "{}", s)
    }
}

impl std::error::Error for Fehler {}

/// Was ein Hintergrundlauf der Oberflaeche zurueckmeldet.
#[derive(Debug, Clone, PartialEq)]
pub enum Meldung {
    /// Abgleich fertig: so viele Freunde, so viele offene Anfragen.
    Abgeglichen { freunde: usize, offen: usize },
    /// Anfrage ist beim Relay angekommen.
    Angefragt(String),
    /// Der andere hatte mich schon gefragt - wir sind jetzt sofort Freunde.
    SofortBefreundet(String),
    Angenommen(String),
    Abgelehnt(String),
    Entfernt(String),
    /// Der Relay hat abgelehnt (Text ist anzeigefertig).
    Fehlgeschlagen(String),
}

/// Nur Ziffern - der Nutzer tippt "497 628 420" oder "497-628-420".
pub fn normalisieren(fvid: &str) -> String {
    fvid.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// Sieht das nach einer FreeViewer-ID aus? (9 oder 10 Stellen)
pub fn ist_id(fvid: &str) -> bool {
    let d = normalisieren(fvid);
    (9..=10).contains(&d.len())
}

fn jetzt() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// =================================================================== Buch

/// Die lokale Kopie: Freunde, offene Anfragen, Blockliste.
///
/// Liegt als `freunde.json` im selben Ordner wie `partners.json` und
/// `identity.txt`. Ohne Konto ist das die einzige Fassung, die dieser Rechner
/// hat; mit Konto ist sie zusaetzlich beim Relay hinterlegt.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Buch {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub freunde: Vec<Freund>,
    #[serde(default)]
    pub anfragen: Vec<Anfrage>,
    #[serde(default)]
    pub blockiert: Vec<Blockiert>,
    /// Zu welcher FreeViewer-ID dieser Stand gehoert (Kontrolle beim Laden).
    #[serde(default)]
    pub fvid: String,
    /// Zu welchem Konto - leer heisst "ohne Konto".
    #[serde(default)]
    pub konto: String,
    /// Wann zuletzt erfolgreich abgeglichen, unix Sekunden.
    #[serde(default)]
    pub stand: u64,
}

impl Buch {
    pub fn path() -> std::path::PathBuf {
        crate::ident::config_dir().join("freunde.json")
    }

    fn backup_path() -> std::path::PathBuf {
        crate::ident::config_dir().join("freunde.bak.json")
    }

    /// Laedt die Liste.
    ///
    /// Anders als beim Adressbuch ist eine LEERE Liste hier voellig gueltig -
    /// wer seinen letzten Freund entfernt, hat eben keine mehr. Die
    /// Sicherungskopie kommt deshalb nur zum Zug, wenn die Datei fehlt oder
    /// kaputt ist; sonst wuerden entfernte Freunde wieder auftauchen.
    pub fn load() -> Self {
        if let Ok(s) = std::fs::read_to_string(Self::path()) {
            if let Ok(b) = serde_json::from_str::<Self>(&s) {
                return b;
            }
        }
        if let Ok(s) = std::fs::read_to_string(Self::backup_path()) {
            if let Ok(b) = serde_json::from_str::<Self>(&s) {
                return b;
            }
        }
        Self::default()
    }

    /// Schreibt die Liste - erst daneben, dann umbenennen, damit es nie eine
    /// halbe Datei gibt.
    pub fn save(&self) {
        let dir = crate::ident::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        if std::fs::metadata(Self::path()).is_ok() {
            let _ = std::fs::copy(Self::path(), Self::backup_path());
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let tmp = dir.join("freunde.json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, Self::path());
            }
        }
    }

    // ------------------------------------------------------------ Abfragen

    pub fn freund(&self, fvid: &str) -> Option<&Freund> {
        let id = normalisieren(fvid);
        self.freunde.iter().find(|f| f.fvid == id)
    }

    pub fn ist_freund(&self, fvid: &str) -> bool {
        self.freund(fvid).is_some()
    }

    pub fn ist_blockiert(&self, fvid: &str) -> bool {
        let id = normalisieren(fvid);
        self.blockiert.iter().any(|b| b.fvid == id)
    }

    /// Online zuerst, dann nach Namen - so liest sich eine Freundesliste.
    pub fn sortiert(&self) -> Vec<Freund> {
        let mut v = self.freunde.clone();
        v.sort_by(|a, b| {
            b.online
                .cmp(&a.online)
                .then_with(|| a.anzeige().to_lowercase().cmp(&b.anzeige().to_lowercase()))
        });
        v
    }

    pub fn eingehende(&self) -> Vec<Anfrage> {
        self.anfragen
            .iter()
            .filter(|a| a.richtung == Richtung::Eingehend)
            .cloned()
            .collect()
    }

    pub fn ausgehende(&self) -> Vec<Anfrage> {
        self.anfragen
            .iter()
            .filter(|a| a.richtung == Richtung::Ausgehend)
            .cloned()
            .collect()
    }

    fn anfrage_von(&self, fvid: &str, richtung: Richtung) -> Option<usize> {
        let id = normalisieren(fvid);
        self.anfragen
            .iter()
            .position(|a| a.von_fvid == id && a.richtung == richtung)
    }

    // ------------------------------------------------------- Zustandswechsel
    //
    // Diese Schritte laufen OHNE Netz und sind die Stellen, an denen der
    // Zustand sich aendern darf. Der Relay macht dieselben Pruefungen noch
    // einmal - hier stehen sie, damit die Oberflaeche sofort antworten kann
    // und nicht erst auf die Leitung wartet.

    /// Ich stelle eine Anfrage.
    pub fn anfrage_stellen(&mut self, fvid: &str, name: &str) -> Result<Anfrage, Fehler> {
        let id = normalisieren(fvid);
        if !ist_id(&id) {
            return Err(Fehler::UngueltigeId);
        }
        if !self.fvid.is_empty() && id == self.fvid {
            return Err(Fehler::SelbstAnfrage);
        }
        if self.ist_freund(&id) {
            return Err(Fehler::SchonBefreundet);
        }
        if self.ist_blockiert(&id) {
            return Err(Fehler::Blockiert);
        }
        if self.anfrage_von(&id, Richtung::Ausgehend).is_some() {
            return Err(Fehler::SchonAngefragt);
        }
        // Er hat mich schon gefragt: dann sind sich beide einig.
        if let Some(i) = self.anfrage_von(&id, Richtung::Eingehend) {
            let a = self.anfragen[i].clone();
            self.annehmen(&a.von_fvid)?;
            return Ok(a);
        }
        let a = Anfrage {
            von_fvid: id,
            name: crate::presence::clean(name),
            wann: jetzt(),
            richtung: Richtung::Ausgehend,
            nachricht: String::new(),
            online: false,
        };
        self.anfragen.push(a.clone());
        self.kuerzen();
        Ok(a)
    }

    /// Eine Anfrage kam vom Relay herein. `true`, wenn sie neu war.
    pub fn anfrage_eintragen(&mut self, mut a: Anfrage) -> bool {
        a.von_fvid = normalisieren(&a.von_fvid);
        if a.von_fvid.is_empty() || self.ist_freund(&a.von_fvid) || self.ist_blockiert(&a.von_fvid) {
            return false;
        }
        if let Some(i) = self.anfrage_von(&a.von_fvid, a.richtung) {
            self.anfragen[i] = a;
            return false;
        }
        self.anfragen.push(a);
        self.kuerzen();
        true
    }

    /// Ich nehme eine EINGEHENDE Anfrage an - erst damit entsteht die
    /// Freundschaft. Ohne vorliegende Anfrage geht das nicht; ein Fremder
    /// wird so niemals still zum Freund.
    pub fn annehmen(&mut self, fvid: &str) -> Result<Freund, Fehler> {
        let id = normalisieren(fvid);
        if let Some(f) = self.freund(&id) {
            return Ok(f.clone()); // zweiter Klick - kein Fehler
        }
        let i = self
            .anfrage_von(&id, Richtung::Eingehend)
            .ok_or(Fehler::KeineAnfrage)?;
        let a = self.anfragen.remove(i);
        // eine eigene Anfrage in die Gegenrichtung faellt mit weg
        if let Some(j) = self.anfrage_von(&id, Richtung::Ausgehend) {
            self.anfragen.remove(j);
        }
        let f = Freund {
            fvid: id,
            name: a.name,
            seit: jetzt(),
            online: a.online,
            gesehen: 0,
        };
        self.freunde.push(f.clone());
        self.kuerzen();
        Ok(f)
    }

    /// Ablehnen (eingehend) bzw. die eigene Anfrage zuruecknehmen
    /// (ausgehend). Beides wirft nur die Anfrage weg - es entsteht KEINE
    /// Freundschaft.
    pub fn ablehnen(&mut self, fvid: &str) -> Result<(), Fehler> {
        let id = normalisieren(fvid);
        let vorher = self.anfragen.len();
        self.anfragen.retain(|a| a.von_fvid != id);
        if self.anfragen.len() == vorher {
            return Err(Fehler::KeineAnfrage);
        }
        Ok(())
    }

    /// Freundschaft aufloesen. Gilt fuer beide Seiten - der Relay raeumt sie
    /// auch beim anderen weg.
    pub fn entfernen(&mut self, fvid: &str) -> Result<(), Fehler> {
        let id = normalisieren(fvid);
        let vorher = self.freunde.len();
        self.freunde.retain(|f| f.fvid != id);
        if self.freunde.len() == vorher {
            return Err(Fehler::KeinFreund);
        }
        Ok(())
    }

    /// Auf die Blockliste setzen (oder wieder herunternehmen). Wer blockiert
    /// ist, kann nicht mehr anfragen; offene Anfragen fallen weg.
    pub fn blockieren(&mut self, fvid: &str, an: bool) -> Result<(), Fehler> {
        let id = normalisieren(fvid);
        if !ist_id(&id) {
            return Err(Fehler::UngueltigeId);
        }
        if !self.fvid.is_empty() && id == self.fvid {
            return Err(Fehler::SelbstAnfrage);
        }
        if an {
            if !self.ist_blockiert(&id) {
                self.blockiert.push(Blockiert {
                    fvid: id.clone(),
                    seit: jetzt(),
                });
            }
            self.anfragen.retain(|a| a.von_fvid != id);
        } else {
            self.blockiert.retain(|b| b.fvid != id);
        }
        Ok(())
    }

    /// Der Relay hat gesprochen. Seine Fassung gilt - sie ist die einzige, die
    /// beide Seiten kennen. Gibt zurueck, ob sich etwas geaendert hat.
    pub fn uebernehmen(&mut self, stand: &Stand) -> bool {
        let mut neu = Buch {
            version: 1,
            freunde: stand.freunde.clone(),
            anfragen: stand
                .eingehend
                .iter()
                .cloned()
                .map(|mut a| {
                    a.richtung = Richtung::Eingehend;
                    a
                })
                .chain(stand.ausgehend.iter().cloned().map(|mut a| {
                    a.richtung = Richtung::Ausgehend;
                    a
                }))
                .collect(),
            blockiert: stand.blockiert.clone(),
            fvid: normalisieren(&stand.fvid),
            konto: stand.konto.clone(),
            stand: jetzt(),
        };
        for f in &mut neu.freunde {
            f.fvid = normalisieren(&f.fvid);
        }
        for a in &mut neu.anfragen {
            a.von_fvid = normalisieren(&a.von_fvid);
        }
        neu.freunde.retain(|f| !f.fvid.is_empty());
        neu.anfragen.retain(|a| !a.von_fvid.is_empty());
        neu.kuerzen();
        let anders = neu.freunde != self.freunde
            || neu.anfragen != self.anfragen
            || neu.blockiert != self.blockiert
            || neu.konto != self.konto;
        *self = neu;
        anders
    }

    fn kuerzen(&mut self) {
        if self.freunde.len() > MAX_FREUNDE {
            self.freunde.truncate(MAX_FREUNDE);
        }
        if self.anfragen.len() > MAX_ANFRAGEN {
            self.anfragen.truncate(MAX_ANFRAGEN);
        }
        self.version = 1;
    }
}

// ================================================================ Netzteil

/// Die Antwort des Relays auf `/fv/freunde/stand` (und `/liste`).
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Stand {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub fvid: String,
    #[serde(default)]
    pub konto: String,
    #[serde(default)]
    pub freunde: Vec<Freund>,
    #[serde(default)]
    pub eingehend: Vec<Anfrage>,
    #[serde(default)]
    pub ausgehend: Vec<Anfrage>,
    #[serde(default)]
    pub blockiert: Vec<Blockiert>,
    #[serde(default)]
    pub error: String,
}

/// `wss://host/fv/ws` -> `https://host/fv/freunde/<was>`
pub fn url(relay_url: &str, was: &str) -> String {
    let http = if let Some(rest) = relay_url.strip_prefix("wss://") {
        format!("https://{}", rest)
    } else if let Some(rest) = relay_url.strip_prefix("ws://") {
        format!("http://{}", rest)
    } else {
        relay_url.to_string()
    };
    let basis = match http.rfind("/ws") {
        Some(i) if i + 3 == http.len() => http[..i].to_string(),
        _ => http.trim_end_matches('/').to_string(),
    };
    format!("{}/freunde/{}", basis, was)
}

/// Ein POST an den Relay. Fehlercodes werden zu lesbaren deutschen Saetzen.
fn ruf(relay_url: &str, was: &str, rumpf: serde_json::Value) -> Result<serde_json::Value, Fehler> {
    let ziel = url(relay_url, was);
    let body = rumpf.to_string();
    let (status, text) = match ureq::post(&ziel)
        .header("content-type", "application/json")
        .send(body.as_str())
    {
        Ok(mut r) => {
            let code = r.status().as_u16();
            let text = r
                .body_mut()
                .read_to_string()
                .map_err(|e| Fehler::Netz(format!("Antwort unlesbar: {}", e)))?;
            (code, text)
        }
        Err(ureq::Error::StatusCode(code)) => (code, String::new()),
        Err(e) => return Err(Fehler::Netz(format!("Relay nicht erreichbar: {}", e))),
    };
    let wert: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));
    if status == 200 {
        return Ok(wert);
    }
    let meldung = wert
        .get("error")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| match status {
            401 => "Dieses Geraet kennt der Relay noch nicht".to_string(),
            403 => "Anfrage nicht moeglich".to_string(),
            404 => "Nicht gefunden".to_string(),
            409 => "Geht nicht - das laeuft schon".to_string(),
            429 => "Zu viele Anfragen - bitte kurz warten".to_string(),
            _ => format!("Relay meldet Fehler {}", status),
        });
    Err(Fehler::Netz(meldung))
}

/// Der Ausweis, der in jedem Rumpf steht.
fn ausweis(geheimnis: &str, token: &str, name: &str) -> serde_json::Value {
    let mut v = serde_json::json!({ "secret": geheimnis, "name": name });
    if !token.is_empty() {
        v["token"] = serde_json::Value::String(token.to_string());
    }
    v
}

fn mit(mut v: serde_json::Value, schluessel: &str, wert: &str) -> serde_json::Value {
    v[schluessel] = serde_json::Value::String(wert.to_string());
    v
}

/// Kompletten Stand holen (ein Rundlauf fuer Freunde + Anfragen + Blockliste).
pub fn hole_stand(relay: &str, geheimnis: &str, token: &str, name: &str) -> Result<Stand, Fehler> {
    let v = ruf(relay, "stand", ausweis(geheimnis, token, name))?;
    serde_json::from_value(v).map_err(|e| Fehler::Netz(format!("Antwort unverstaendlich: {}", e)))
}

pub fn sende_anfrage(
    relay: &str,
    geheimnis: &str,
    token: &str,
    name: &str,
    an: &str,
    nachricht: &str,
) -> Result<bool, Fehler> {
    let mut v = mit(ausweis(geheimnis, token, name), "an", &normalisieren(an));
    v["nachricht"] = serde_json::Value::String(nachricht.to_string());
    let a = ruf(relay, "anfragen", v)?;
    // true = der andere hatte schon gefragt, wir sind sofort befreundet
    Ok(a.get("angenommen").and_then(|x| x.as_bool()).unwrap_or(false))
}

pub fn sende_annehmen(relay: &str, geheimnis: &str, token: &str, name: &str, von: &str) -> Result<(), Fehler> {
    ruf(relay, "annehmen", mit(ausweis(geheimnis, token, name), "von", &normalisieren(von))).map(|_| ())
}

pub fn sende_ablehnen(relay: &str, geheimnis: &str, token: &str, name: &str, wen: &str) -> Result<(), Fehler> {
    ruf(relay, "ablehnen", mit(ausweis(geheimnis, token, name), "von", &normalisieren(wen))).map(|_| ())
}

pub fn sende_entfernen(relay: &str, geheimnis: &str, token: &str, name: &str, wen: &str) -> Result<(), Fehler> {
    ruf(relay, "entfernen", mit(ausweis(geheimnis, token, name), "fvid", &normalisieren(wen))).map(|_| ())
}

pub fn sende_blockieren(
    relay: &str,
    geheimnis: &str,
    token: &str,
    name: &str,
    wen: &str,
    an: bool,
) -> Result<(), Fehler> {
    let mut v = mit(ausweis(geheimnis, token, name), "fvid", &normalisieren(wen));
    v["an"] = serde_json::Value::Bool(an);
    ruf(relay, "blockieren", v).map(|_| ())
}

// ================================================================= Dienst

/// Der Freunde-Dienst: haelt die Liste, gleicht im Hintergrund ab und laesst
/// die Oberflaeche NIE warten.
///
/// Muster wie `presence::Watch` (Hintergrundfaden + Zwischenspeicher) und
/// `account::sync_async` (Briefkasten fuer das Ergebnis).
///
/// ```ignore
/// let freunde = freunde::Dienst::vom_geraet(shared.relay_url.clone());
/// freunde.starten();
/// // in jedem Bild der Oberflaeche:
/// if let Some(m) = freunde.meldung() { /* kurz anzeigen */ }
/// for f in freunde.liste() { /* Zeile zeichnen, Knopf "Meeting" */ }
/// ```
pub struct Dienst {
    relay: String,
    geheimnis: String,
    name: Mutex<String>,
    token: Mutex<String>,
    buch: Mutex<Buch>,
    /// Briefkasten fuer die Oberflaeche. Als Arc, damit ein Hintergrundfaden
    /// hineinschreiben kann, ohne den ganzen Dienst festzuhalten.
    postfach: Arc<Mutex<Vec<Meldung>>>,
    gestartet: AtomicBool,
    beschaeftigt: AtomicBool,
    /// "Bitte gleich abgleichen" - setzt jeder Schritt, liest der Faden.
    sofort: Arc<AtomicBool>,
    letzter: AtomicU64,
    /// Letzter Abgleich hat geklappt?
    verbunden: AtomicBool,
}

impl Dienst {
    /// Mit ausdruecklichem Geheimnis und Namen (fuer Tests und Sonderfaelle).
    pub fn neu(relay_url: String, geheimnis: String, name: String) -> Arc<Self> {
        Arc::new(Self {
            relay: relay_url,
            geheimnis,
            name: Mutex::new(name),
            token: Mutex::new(String::new()),
            buch: Mutex::new(Buch::load()),
            postfach: Arc::new(Mutex::new(Vec::new())),
            gestartet: AtomicBool::new(false),
            beschaeftigt: AtomicBool::new(false),
            sofort: Arc::new(AtomicBool::new(false)),
            letzter: AtomicU64::new(0),
            verbunden: AtomicBool::new(false),
        })
    }

    /// Der Normalfall: Geheimnis und Name kommen von diesem Rechner.
    pub fn vom_geraet(relay_url: String) -> Arc<Self> {
        let g = crate::ident::load_or_create_secret();
        let n = crate::presence::device_name();
        Self::neu(relay_url, g, n)
    }

    /// Konto-Zeichen setzen (`None` = abgemeldet weiterarbeiten).
    pub fn konto_setzen(&self, token: Option<&str>) {
        let neu = token.unwrap_or("").trim().to_string();
        let mut t = self.token.lock().unwrap();
        if *t != neu {
            *t = neu;
            self.sofort.store(true, Ordering::SeqCst);
        }
    }

    pub fn namen_setzen(&self, name: &str) {
        *self.name.lock().unwrap() = crate::presence::clean(name);
    }

    pub fn angemeldet(&self) -> bool {
        !self.token.lock().unwrap().is_empty()
    }

    /// Hat der letzte Abgleich geklappt?
    pub fn verbunden(&self) -> bool {
        self.verbunden.load(Ordering::Relaxed)
    }

    /// Der Hintergrundfaden. Darf mehrfach aufgerufen werden.
    pub fn starten(self: &Arc<Self>) {
        if self.gestartet.swap(true, Ordering::SeqCst) {
            return;
        }
        let ich = self.clone();
        std::thread::spawn(move || loop {
            let faellig = jetzt().saturating_sub(ich.letzter.load(Ordering::Relaxed))
                >= ABGLEICH_SEKUNDEN;
            if ich.sofort.swap(false, Ordering::SeqCst) || faellig {
                ich.abgleich();
            }
            std::thread::sleep(Duration::from_millis(500));
        });
    }

    // ------------------------------------------------------- Was die GUI holt

    /// Die Freundesliste - online zuerst. Kommt aus dem Zwischenspeicher,
    /// wartet also nie auf das Netz.
    pub fn liste(&self) -> Vec<Freund> {
        self.buch.lock().unwrap().sortiert()
    }

    /// Alle offenen Anfragen, beide Richtungen.
    pub fn offene_anfragen(&self) -> Vec<Anfrage> {
        self.buch.lock().unwrap().anfragen.clone()
    }

    /// Nur die, auf die ich antworten muss.
    pub fn eingehende(&self) -> Vec<Anfrage> {
        self.buch.lock().unwrap().eingehende()
    }

    pub fn ausgehende(&self) -> Vec<Anfrage> {
        self.buch.lock().unwrap().ausgehende()
    }

    pub fn blockierte(&self) -> Vec<Blockiert> {
        self.buch.lock().unwrap().blockiert.clone()
    }

    pub fn ist_freund(&self, fvid: &str) -> bool {
        self.buch.lock().unwrap().ist_freund(fvid)
    }

    /// Erreichbarer Freund? (Ein-Klick-Meeting/Fernwartung nur dann anbieten.)
    pub fn erreichbar(&self, fvid: &str) -> bool {
        self.buch
            .lock()
            .unwrap()
            .freund(fvid)
            .map(|f| f.online)
            .unwrap_or(false)
    }

    /// Eine Abschrift des ganzen Buches.
    pub fn buch(&self) -> Buch {
        self.buch.lock().unwrap().clone()
    }

    /// Naechste Meldung aus dem Briefkasten (und aus ihm heraus).
    pub fn meldung(&self) -> Option<Meldung> {
        let mut p = self.postfach.lock().unwrap();
        if p.is_empty() {
            None
        } else {
            Some(p.remove(0))
        }
    }

    fn melde(&self, m: Meldung) {
        let mut p = self.postfach.lock().unwrap();
        if p.len() > 20 {
            p.remove(0);
        }
        p.push(m);
    }

    // --------------------------------------------------------- Was die GUI tut
    //
    // Alle vier melden SOFORT zurueck, ob der Schritt ueberhaupt zulaessig ist
    // (Selbst-Anfrage, doppelt, kein Freund ...). Das Netz laeuft danach in
    // einem eigenen Faden; sein Ergebnis kommt ueber meldung().

    /// Freundschaftsanfrage an eine FreeViewer-ID.
    pub fn anfragen(&self, fvid: &str) -> Result<(), Fehler> {
        self.anfragen_mit(fvid, "")
    }

    /// Wie `anfragen`, aber mit einem kurzen Gruss.
    pub fn anfragen_mit(&self, fvid: &str, nachricht: &str) -> Result<(), Fehler> {
        let id = normalisieren(fvid);
        {
            let mut b = self.buch.lock().unwrap();
            let name = self.name.lock().unwrap().clone();
            b.anfrage_stellen(&id, &name)?;
            b.save();
        }
        let (relay, geheimnis, token, name) = self.zugang();
        let nachricht = nachricht.to_string();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            match sende_anfrage(&relay, &geheimnis, &token, &name, &id, &nachricht) {
                Ok(true) => ich.melde(Meldung::SofortBefreundet(id.clone())),
                Ok(false) => ich.melde(Meldung::Angefragt(id.clone())),
                Err(e) => ich.melde(Meldung::Fehlgeschlagen(e.to_string())),
            }
            ich.sofort.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Eine eingehende Anfrage annehmen - erst damit sind beide Freunde.
    pub fn annehmen(&self, fvid: &str) -> Result<(), Fehler> {
        let id = normalisieren(fvid);
        {
            let mut b = self.buch.lock().unwrap();
            b.annehmen(&id)?;
            b.save();
        }
        let (relay, geheimnis, token, name) = self.zugang();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            match sende_annehmen(&relay, &geheimnis, &token, &name, &id) {
                Ok(()) => ich.melde(Meldung::Angenommen(id.clone())),
                Err(e) => ich.melde(Meldung::Fehlgeschlagen(e.to_string())),
            }
            ich.sofort.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Ablehnen bzw. die eigene Anfrage zuruecknehmen.
    pub fn ablehnen(&self, fvid: &str) -> Result<(), Fehler> {
        let id = normalisieren(fvid);
        {
            let mut b = self.buch.lock().unwrap();
            b.ablehnen(&id)?;
            b.save();
        }
        let (relay, geheimnis, token, name) = self.zugang();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            match sende_ablehnen(&relay, &geheimnis, &token, &name, &id) {
                Ok(()) => ich.melde(Meldung::Abgelehnt(id.clone())),
                Err(e) => ich.melde(Meldung::Fehlgeschlagen(e.to_string())),
            }
            ich.sofort.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Freundschaft aufloesen (bei beiden).
    pub fn entfernen(&self, fvid: &str) -> Result<(), Fehler> {
        let id = normalisieren(fvid);
        {
            let mut b = self.buch.lock().unwrap();
            b.entfernen(&id)?;
            b.save();
        }
        let (relay, geheimnis, token, name) = self.zugang();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            match sende_entfernen(&relay, &geheimnis, &token, &name, &id) {
                Ok(()) => ich.melde(Meldung::Entfernt(id.clone())),
                Err(e) => ich.melde(Meldung::Fehlgeschlagen(e.to_string())),
            }
            ich.sofort.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Blockieren (`an = true`) oder wieder freigeben.
    pub fn blockieren(&self, fvid: &str, an: bool) -> Result<(), Fehler> {
        let id = normalisieren(fvid);
        {
            let mut b = self.buch.lock().unwrap();
            b.blockieren(&id, an)?;
            b.save();
        }
        let (relay, geheimnis, token, name) = self.zugang();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            if let Err(e) = sende_blockieren(&relay, &geheimnis, &token, &name, &id, an) {
                ich.melde(Meldung::Fehlgeschlagen(e.to_string()));
            }
            ich.sofort.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Sofortigen Abgleich anstossen (kehrt sofort zurueck).
    pub fn synchronisieren(&self) {
        self.sofort.store(true, Ordering::SeqCst);
    }

    /// Abgleich JETZT und hier - blockiert. Nur fuer den Hintergrundfaden und
    /// den kopflosen Betrieb; die Oberflaeche nimmt `synchronisieren()`.
    pub fn abgleich_blockierend(&self) -> Result<Stand, Fehler> {
        let (relay, geheimnis, token, name) = self.zugang();
        let stand = hole_stand(&relay, &geheimnis, &token, &name)?;
        let mut b = self.buch.lock().unwrap();
        if b.uebernehmen(&stand) {
            b.save();
        } else {
            b.stand = jetzt();
        }
        Ok(stand)
    }

    fn abgleich(&self) {
        if self.beschaeftigt.swap(true, Ordering::SeqCst) {
            return;
        }
        let ergebnis = self.abgleich_blockierend();
        self.letzter.store(jetzt(), Ordering::Relaxed);
        match ergebnis {
            Ok(_) => {
                self.verbunden.store(true, Ordering::Relaxed);
                let b = self.buch.lock().unwrap();
                self.melde(Meldung::Abgeglichen {
                    freunde: b.freunde.len(),
                    offen: b.anfragen.len(),
                });
            }
            Err(e) => {
                self.verbunden.store(false, Ordering::Relaxed);
                self.melde(Meldung::Fehlgeschlagen(e.to_string()));
            }
        }
        self.beschaeftigt.store(false, Ordering::SeqCst);
    }

    fn zugang(&self) -> (String, String, String, String) {
        (
            self.relay.clone(),
            self.geheimnis.clone(),
            self.token.lock().unwrap().clone(),
            self.name.lock().unwrap().clone(),
        )
    }

    /// Ein kleiner Griff, mit dem ein Hintergrundfaden melden und den
    /// naechsten Abgleich anstossen kann - mehr braucht er nicht.
    fn klon_briefkasten(&self) -> Briefkasten {
        Briefkasten {
            postfach: Arc::clone(&self.postfach),
            sofort: Arc::clone(&self.sofort),
        }
    }
}

/// Ein kleiner Griff fuer Hintergrundfaeden.
struct Briefkasten {
    postfach: Arc<Mutex<Vec<Meldung>>>,
    sofort: Arc<AtomicBool>,
}

impl Briefkasten {
    fn melde(&self, m: Meldung) {
        let mut p = self.postfach.lock().unwrap();
        if p.len() > 20 {
            p.remove(0);
        }
        p.push(m);
    }
}

// =================================================================== Tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Eigener Wegwerf-Ordner - ein Test fasst NIE die echte Konfiguration an
    /// (der Datenverlust vom 31.07.2026 kam genau daher).
    fn eigener_ordner(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("fv-freunde-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        crate::ident::set_test_config_dir(d.clone());
        d
    }

    fn buch_mit_mir(meine: &str) -> Buch {
        Buch {
            version: 1,
            fvid: meine.to_string(),
            ..Default::default()
        }
    }

    fn eingehend(von: &str, name: &str) -> Anfrage {
        Anfrage {
            von_fvid: von.to_string(),
            name: name.to_string(),
            wann: 1_700_000_000,
            richtung: Richtung::Eingehend,
            nachricht: String::new(),
            online: true,
        }
    }

    #[test]
    fn ein_test_schreibt_nie_in_die_echte_konfiguration() {
        eigener_ordner("isoliert");
        let b = buch_mit_mir("111111111");
        b.save();
        assert!(Buch::path().starts_with(std::env::temp_dir()));
        if let Some(m) = crate::ident::machine_config_dir() {
            assert!(!Buch::path().starts_with(m));
        }
        assert!(!Buch::path().starts_with(crate::ident::user_config_dir()));
    }

    #[test]
    fn speichern_und_laden_behaelt_alles() {
        eigener_ordner("speichern");
        let mut b = buch_mit_mir("111111111");
        b.anfrage_eintragen(eingehend("222222222", "Laptop"));
        b.annehmen("222222222").unwrap();
        b.anfrage_eintragen(eingehend("333333333", "Handy"));
        b.blockieren("444444444", true).unwrap();
        b.save();

        let wieder = Buch::load();
        assert_eq!(wieder.freunde.len(), 1);
        assert_eq!(wieder.freunde[0].fvid, "222222222");
        assert_eq!(wieder.freunde[0].name, "Laptop");
        assert_eq!(wieder.eingehende().len(), 1);
        assert_eq!(wieder.eingehende()[0].von_fvid, "333333333");
        assert!(wieder.ist_blockiert("444444444"));
        assert_eq!(wieder.fvid, "111111111");
    }

    #[test]
    fn eine_leere_liste_ist_gueltig_und_holt_nichts_zurueck() {
        eigener_ordner("leer");
        let mut b = buch_mit_mir("111111111");
        b.anfrage_eintragen(eingehend("222222222", "Weg"));
        b.annehmen("222222222").unwrap();
        b.save();
        // zweiter Schreibvorgang legt die Sicherungskopie an
        b.entfernen("222222222").unwrap();
        b.save();
        let wieder = Buch::load();
        assert_eq!(wieder.freunde.len(), 0, "entfernte Freunde duerfen nicht zurueckkommen");
    }

    #[test]
    fn eine_kaputte_datei_faellt_auf_die_sicherung_zurueck() {
        eigener_ordner("kaputt");
        let mut b = buch_mit_mir("111111111");
        b.anfrage_eintragen(eingehend("222222222", "Laptop"));
        b.annehmen("222222222").unwrap();
        b.save();
        b.save(); // jetzt existiert die Sicherungskopie
        std::fs::write(Buch::path(), b"{ kaputt").unwrap();
        let wieder = Buch::load();
        assert_eq!(wieder.freunde.len(), 1);
        assert_eq!(wieder.freunde[0].fvid, "222222222");
    }

    #[test]
    fn eine_anfrage_an_sich_selbst_wird_abgewiesen() {
        let mut b = buch_mit_mir("111111111");
        assert_eq!(b.anfrage_stellen("111111111", "Ich"), Err(Fehler::SelbstAnfrage));
        // auch mit Leerzeichen getippt
        assert_eq!(b.anfrage_stellen("111 111 111", "Ich"), Err(Fehler::SelbstAnfrage));
        assert!(b.anfragen.is_empty());
    }

    #[test]
    fn eine_unsinnige_id_wird_abgewiesen() {
        let mut b = buch_mit_mir("111111111");
        assert_eq!(b.anfrage_stellen("123", "Kurz"), Err(Fehler::UngueltigeId));
        assert_eq!(b.anfrage_stellen("", "Nichts"), Err(Fehler::UngueltigeId));
        assert_eq!(b.anfrage_stellen("abc", "Buchstaben"), Err(Fehler::UngueltigeId));
    }

    #[test]
    fn dieselbe_anfrage_zweimal_geht_nicht() {
        let mut b = buch_mit_mir("111111111");
        b.anfrage_stellen("222222222", "Ich").unwrap();
        assert_eq!(b.anfrage_stellen("222222222", "Ich"), Err(Fehler::SchonAngefragt));
        // andere Schreibweise, gleiche ID
        assert_eq!(b.anfrage_stellen("222 222 222", "Ich"), Err(Fehler::SchonAngefragt));
        assert_eq!(b.ausgehende().len(), 1);
    }

    #[test]
    fn eine_anfrage_an_einen_freund_geht_nicht() {
        let mut b = buch_mit_mir("111111111");
        b.anfrage_eintragen(eingehend("222222222", "Laptop"));
        b.annehmen("222222222").unwrap();
        assert_eq!(b.anfrage_stellen("222222222", "Ich"), Err(Fehler::SchonBefreundet));
    }

    #[test]
    fn ohne_anfrage_wird_niemand_still_zum_freund() {
        let mut b = buch_mit_mir("111111111");
        // Anfrage von einem voellig Unbekannten liegt nicht vor
        assert_eq!(b.annehmen("999999999"), Err(Fehler::KeineAnfrage));
        assert!(b.freunde.is_empty());
        // eine AUSGEHENDE Anfrage macht mich nicht zum Freund - der andere
        // muss zustimmen
        b.anfrage_stellen("222222222", "Ich").unwrap();
        assert_eq!(b.annehmen("222222222"), Err(Fehler::KeineAnfrage));
        assert!(b.freunde.is_empty());
    }

    #[test]
    fn annehmen_macht_aus_der_anfrage_einen_freund() {
        let mut b = buch_mit_mir("111111111");
        b.anfrage_eintragen(eingehend("222222222", "Laptop"));
        assert_eq!(b.eingehende().len(), 1);
        let f = b.annehmen("222222222").unwrap();
        assert_eq!(f.fvid, "222222222");
        assert_eq!(f.name, "Laptop");
        assert!(f.seit > 0);
        assert!(b.ist_freund("222222222"));
        assert!(b.anfragen.is_empty(), "die Anfrage muss aus der Liste raus");
        // zweiter Klick ist kein Fehler
        assert!(b.annehmen("222222222").is_ok());
        assert_eq!(b.freunde.len(), 1);
    }

    #[test]
    fn beide_fragen_sich_gegenseitig_und_sind_sofort_freunde() {
        let mut b = buch_mit_mir("111111111");
        b.anfrage_eintragen(eingehend("222222222", "Laptop"));
        b.anfrage_stellen("222222222", "Ich").unwrap();
        assert!(b.ist_freund("222222222"));
        assert!(b.anfragen.is_empty());
    }

    #[test]
    fn ablehnen_wirft_die_anfrage_weg_ohne_freundschaft() {
        let mut b = buch_mit_mir("111111111");
        b.anfrage_eintragen(eingehend("222222222", "Laptop"));
        b.ablehnen("222222222").unwrap();
        assert!(b.anfragen.is_empty());
        assert!(!b.ist_freund("222222222"));
        assert_eq!(b.ablehnen("222222222"), Err(Fehler::KeineAnfrage));
    }

    #[test]
    fn eine_eigene_anfrage_laesst_sich_zuruecknehmen() {
        let mut b = buch_mit_mir("111111111");
        b.anfrage_stellen("222222222", "Ich").unwrap();
        assert_eq!(b.ausgehende().len(), 1);
        b.ablehnen("222222222").unwrap();
        assert!(b.anfragen.is_empty());
        assert!(!b.ist_freund("222222222"));
    }

    #[test]
    fn entfernen_nimmt_den_freund_aus_der_liste() {
        let mut b = buch_mit_mir("111111111");
        b.anfrage_eintragen(eingehend("222222222", "Laptop"));
        b.annehmen("222222222").unwrap();
        b.entfernen("222222222").unwrap();
        assert!(!b.ist_freund("222222222"));
        assert_eq!(b.entfernen("222222222"), Err(Fehler::KeinFreund));
    }

    #[test]
    fn blockieren_wirft_offene_anfragen_weg_und_sperrt_neue() {
        let mut b = buch_mit_mir("111111111");
        b.anfrage_eintragen(eingehend("222222222", "Nervt"));
        b.blockieren("222222222", true).unwrap();
        assert!(b.anfragen.is_empty());
        assert!(b.ist_blockiert("222222222"));
        assert_eq!(b.anfrage_stellen("222222222", "Ich"), Err(Fehler::Blockiert));
        // eine hereinkommende Anfrage prallt ab
        assert!(!b.anfrage_eintragen(eingehend("222222222", "Nervt")));
        assert!(b.anfragen.is_empty());
        // freigeben
        b.blockieren("222222222", false).unwrap();
        assert!(!b.ist_blockiert("222222222"));
        assert!(b.anfrage_eintragen(eingehend("222222222", "Nervt")));
    }

    #[test]
    fn eingehend_und_ausgehend_bleiben_getrennt() {
        let mut b = buch_mit_mir("111111111");
        b.anfrage_eintragen(eingehend("222222222", "A"));
        b.anfrage_stellen("333333333", "Ich").unwrap();
        assert_eq!(b.eingehende().len(), 1);
        assert_eq!(b.ausgehende().len(), 1);
        assert_eq!(b.eingehende()[0].gegenueber(), "222222222");
        assert!(b.eingehende()[0].ist_eingehend());
        assert_eq!(b.ausgehende()[0].gegenueber(), "333333333");
        assert!(!b.ausgehende()[0].ist_eingehend());
    }

    #[test]
    fn der_stand_vom_relay_gewinnt() {
        let mut b = buch_mit_mir("111111111");
        b.anfrage_stellen("999999999", "Ich").unwrap(); // nur lokal geraten
        let stand = Stand {
            ok: true,
            fvid: "111111111".into(),
            konto: "justin".into(),
            freunde: vec![Freund {
                fvid: "222222222".into(),
                name: "Laptop".into(),
                seit: 1_700_000_000,
                online: true,
                gesehen: 1_700_000_000_000,
            }],
            eingehend: vec![eingehend("333333333", "Handy")],
            ausgehend: vec![],
            blockiert: vec![Blockiert { fvid: "444444444".into(), seit: 1 }],
            error: String::new(),
        };
        assert!(b.uebernehmen(&stand));
        assert_eq!(b.freunde.len(), 1);
        assert_eq!(b.freunde[0].fvid, "222222222");
        assert_eq!(b.eingehende().len(), 1);
        assert_eq!(b.ausgehende().len(), 0, "die lokale Vermutung faellt weg");
        assert_eq!(b.konto, "justin");
        assert!(b.ist_blockiert("444444444"));
        // derselbe Stand nochmal aendert nichts
        assert!(!b.uebernehmen(&stand));
    }

    #[test]
    fn der_relay_darf_richtungen_setzen() {
        let mut b = buch_mit_mir("111111111");
        let stand = Stand {
            ok: true,
            fvid: "111111111".into(),
            // der Relay schickt in beiden Listen "richtung", hier absichtlich
            // falsch herum - die Zuordnung nach Liste muss gewinnen
            eingehend: vec![Anfrage {
                von_fvid: "222222222".into(),
                richtung: Richtung::Ausgehend,
                ..Default::default()
            }],
            ausgehend: vec![Anfrage {
                von_fvid: "333333333".into(),
                richtung: Richtung::Eingehend,
                ..Default::default()
            }],
            ..Default::default()
        };
        b.uebernehmen(&stand);
        assert_eq!(b.eingehende()[0].von_fvid, "222222222");
        assert_eq!(b.ausgehende()[0].von_fvid, "333333333");
    }

    #[test]
    fn die_liste_stellt_online_nach_vorne() {
        let mut b = buch_mit_mir("111111111");
        b.freunde = vec![
            Freund { fvid: "222222222".into(), name: "Zebra".into(), online: false, ..Default::default() },
            Freund { fvid: "333333333".into(), name: "Anton".into(), online: false, ..Default::default() },
            Freund { fvid: "444444444".into(), name: "Willi".into(), online: true, ..Default::default() },
        ];
        let s = b.sortiert();
        assert_eq!(s[0].name, "Willi");
        assert_eq!(s[1].name, "Anton");
        assert_eq!(s[2].name, "Zebra");
    }

    #[test]
    fn die_adresse_kommt_vom_relay() {
        assert_eq!(
            url("wss://freeviewer.fleitec.com/fv/ws", "stand"),
            "https://freeviewer.fleitec.com/fv/freunde/stand"
        );
        assert_eq!(
            url("ws://192.168.1.60:7180/fv/ws", "anfragen"),
            "http://192.168.1.60:7180/fv/freunde/anfragen"
        );
    }

    #[test]
    fn ids_werden_auf_ziffern_gekuerzt() {
        assert_eq!(normalisieren("497 628 420"), "497628420");
        assert_eq!(normalisieren("4-976-284-201"), "4976284201");
        assert_eq!(normalisieren("abc"), "");
        assert!(ist_id("497 628 420"));
        assert!(ist_id("1234567890"));
        assert!(!ist_id("12345"));
        assert!(!ist_id("12345678901"));
    }

    #[test]
    fn die_richtung_wird_wie_beim_relay_geschrieben() {
        let a = eingehend("222222222", "A");
        let j = serde_json::to_string(&a).unwrap();
        assert!(j.contains("\"richtung\":\"eingehend\""), "{}", j);
        assert!(j.contains("\"fvid\":\"222222222\""), "{}", j);
        // und wieder zurueck - genau so schickt es der Relay
        let roh = r#"{"fvid":"333333333","name":"Handy","wann":5,"richtung":"ausgehend","nachricht":"hi","online":true}"#;
        let b: Anfrage = serde_json::from_str(roh).unwrap();
        assert_eq!(b.von_fvid, "333333333");
        assert_eq!(b.richtung, Richtung::Ausgehend);
        assert_eq!(b.nachricht, "hi");
    }

    #[test]
    fn die_antwort_des_relays_laesst_sich_lesen() {
        // woertlich so, wie relay.js /fv/freunde/stand antwortet
        let roh = r#"{"ok":true,"fvid":"7902410486","konto":"","freunde":[
            {"fvid":"8358538558","name":"FV-Test-B","seit":1786250165,"online":true,"gesehen":1786250165580}],
            "eingehend":[{"fvid":"2130270636","name":"FV-Test-C","wann":1786250165,"richtung":"eingehend","nachricht":"","online":true}],
            "ausgehend":[],"blockiert":[]}"#;
        let s: Stand = serde_json::from_str(roh).unwrap();
        assert!(s.ok);
        assert_eq!(s.freunde.len(), 1);
        assert_eq!(s.freunde[0].name, "FV-Test-B");
        assert!(s.freunde[0].online);
        assert_eq!(s.eingehend.len(), 1);
        assert_eq!(s.eingehend[0].von_fvid, "2130270636");
    }

    #[test]
    fn anzeige_faellt_auf_die_id_zurueck() {
        let f = Freund { fvid: "497628420".into(), ..Default::default() };
        assert_eq!(f.anzeige(), "497 628 420");
        let g = Freund { fvid: "497628420".into(), name: "Buero-PC".into(), ..Default::default() };
        assert_eq!(g.anzeige(), "Buero-PC");
        assert_eq!(
            Freund { online: true, ..Default::default() }.zuletzt(),
            "online"
        );
    }

    #[test]
    fn namen_bleiben_harmlos() {
        let mut b = buch_mit_mir("111111111");
        let a = b.anfrage_stellen("222222222", "  Pa\"tis {Laptop}  ").unwrap();
        assert_eq!(a.name, "Patis Laptop");
    }
}
