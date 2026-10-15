//! Friends - friend requests, friend list, one-click access.
//!
//! The difference from the address book (`partners.rs`): there everyone enters
//! on their own whom they know - nobody has to agree. A FRIENDSHIP only arises
//! when BOTH sides have agreed. Only then do you see each other in the
//! list, see who is online, and can start a meeting with one click
//! or ask for remote support.
//!
//! Where things live:
//!
//! * The relay holds the truth: `/fv/friends/*` (see relay.js). It is
//!   the only way a request reaches the other side at all.
//! * This file keeps a COPY of it next to the other settings
//!   (`friends.json` in the folder from `ident::config_dir()`, just like
//!   `partners.json`). This way the list is on screen immediately, even when
//!   the relay is currently not reachable.
//!
//! Without an account everything hangs on the FreeViewer ID of this machine. With an account
//! the relay additionally records the account name - then the same
//! friend list is visible on every logged-in device. Both at the same time
//! is the normal case: the ID always applies, the account comes on top.
//!
//! SINCE r7 (24.09.2026) - ONE ADDRESS BOOK: With an account the relay reads the
//! friends from the Fleitec address book (fleikontakte), the same one as FirnChat
//! and the contacts page. A friendship is then one between PERSONS:
//! a friend can have several PCs (`geraete`), or none at all - then
//! `fvid` is empty and they are addressed via `person` (account ID) or
//! `@handle`. Without an account everything stays on the FreeViewer ID as before.
//! The key of an entry is therefore `schluessel()`: the number,
//! otherwise "k:<person>", otherwise "@handle" - the relay understands exactly that too.
//!
//! The client identifies itself exactly as with the WebSocket login
//! (`net::json_register`): with the secret from `identity.txt`. It is
//! ALWAYS in the request body, never in the URL.
//!
//! Nothing here blocks the UI. Every network call runs in its
//! own thread; results come back via `Dienst::notice()` - the same
//! mailbox pattern as in `account.rs` and `presence.rs`.

// The module is complete, the UI is attached separately.
// Until then the public building blocks are "unused" from the point of view of the
// binary program - without this line dozens of warnings would appear here.
#![allow(dead_code)]
use crate::lockutil::LockExt;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How often to sync in the background when nothing is going on.
const ABGLEICH_SEKUNDEN: u64 = 20;

/// If the relay does not know invitations, it is not asked for them for this long.
const EINLADUNG_PAUSE_SEKUNDEN: u64 = 600;

/// Upper limit, so that a broken answer cannot blow up the file.
const MAX_FRIENDS: usize = 500;
const MAX_ANFRAGEN: usize = 200;

// ============================================================== Datentypen

/// Ein bestaetigter Friend. Beide haben zugestimmt.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Friend {
    /// The FreeViewer ID of the other - used for meeting and remote support.
    pub fvid: String,
    /// What the other calls themselves (empty = show ID).
    #[serde(default)]
    pub name: String,
    /// Seit wann befreundet, unix Sekunden.
    #[serde(default)]
    pub seit: u64,
    /// Is the friend reachable right now? Comes from the relay.
    #[serde(default)]
    pub online: bool,
    /// Zuletzt gesehen, unix MILLIsekunden (wie in `presence.rs`).
    #[serde(default)]
    pub gesehen: u64,
    /// Account ID of the person in the address book (empty = friend only via ID).
    #[serde(default)]
    pub person: String,
    /// Their @username (without @), if they have an account.
    #[serde(default)]
    pub handle: String,
    /// All FreeViewer PCs of this person, online first. `fvid` is the first.
    #[serde(default)]
    pub geraete: Vec<FriendDevice>,
    /// "buch" (address book) or "fv" (old list at the relay, without account).
    #[serde(default)]
    pub quelle: String,
}

/// Ein PC eines Freundes.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct FriendDevice {
    pub fvid: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub gesehen: u64,
}

/// Number, otherwise "k:<person>", otherwise "@handle" - the key the
/// relay understands for this entry.
fn key_from(fvid: &str, person: &str, handle: &str) -> String {
    if !fvid.is_empty() {
        fvid.to_string()
    } else if !person.is_empty() {
        format!("k:{}", person)
    } else if !handle.is_empty() {
        format!("@{}", handle)
    } else {
        String::new()
    }
}

/// Does key `k` (already cleaned with `ziel_norm`) match this
/// entry? Number, account or @name - all point to the same person.
fn passt(k: &str, fvid: &str, person: &str, handle: &str) -> bool {
    if k.is_empty() {
        return false;
    }
    if let Some(p) = k.strip_prefix("k:") {
        return !person.is_empty() && p == person;
    }
    if let Some(h) = k.strip_prefix('@') {
        return !handle.is_empty() && h.eq_ignore_ascii_case(handle);
    }
    !fvid.is_empty() && k == fvid
}

impl Friend {
    /// What shows in the list: name, otherwise @name, otherwise the nicely formatted ID.
    pub fn display_name(&self) -> String {
        if !self.name.trim().is_empty() {
            self.name.clone()
        } else if !self.handle.is_empty() {
            format!("@{}", self.handle)
        } else {
            crate::partners::pretty_id(&self.fvid)
        }
    }

    pub fn schluessel(&self) -> String {
        key_from(&self.fvid, &self.person, &self.handle)
    }

    /// Is `k` this friend - via any of their PCs, their account or
    /// their @name?
    pub fn ist(&self, k: &str) -> bool {
        passt(k, &self.fvid, &self.person, &self.handle)
            || (!k.is_empty() && self.geraete.iter().any(|g| g.fvid == k))
    }

    /// Does the friend have a PC one can connect to?
    pub fn hat_pc(&self) -> bool {
        !self.fvid.is_empty()
    }

    /// "just now", "3 min ago" - same wording as the address book.
    pub fn zuletzt(&self) -> String {
        if self.online {
            "online".to_string()
        } else {
            crate::presence::ago_ms(self.gesehen)
        }
    }

    /// Can the friend be rung up right now? (Only with a PC.)
    pub fn erreichbar(&self) -> bool {
        self.online && self.hat_pc()
    }
}

/// Who asked - me or the other?
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Richtung {
    /// Someone wants me as a friend. I have to accept or decline.
    Eingehend,
    /// I asked and am waiting for the answer.
    Ausgehend,
}

impl Default for Richtung {
    fn default() -> Self {
        Richtung::Eingehend
    }
}

/// Eine offene Freundschaftsanfrage.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Request {
    /// The OTHER side: for `Eingehend` the sender, for `Ausgehend` the
    /// recipient. (The name is chosen this way because the list almost always shows
    /// the incoming ones - there it literally means "from whom".)
    #[serde(rename = "fvid", alias = "von_fvid")]
    pub von_fvid: String,
    /// What the other calls themselves.
    #[serde(default)]
    pub name: String,
    /// Wann gestellt, unix Sekunden.
    #[serde(default)]
    pub wann: u64,
    pub richtung: Richtung,
    /// Freier Gruss des Absenders (darf leer sein).
    #[serde(default)]
    #[serde(rename = "nachricht")]
    pub message: String,
    /// Is the other currently online?
    #[serde(default)]
    pub online: bool,
    /// Account ID of the other side (request from the address book).
    #[serde(default)]
    pub person: String,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub quelle: String,
}

impl Request {
    /// The key of the other side - independent of the direction:
    /// their number, otherwise "k:<person>", otherwise "@handle".
    pub fn gegenueber(&self) -> String {
        key_from(&self.von_fvid, &self.person, &self.handle)
    }

    pub fn ist(&self, k: &str) -> bool {
        passt(k, &self.von_fvid, &self.person, &self.handle)
    }

    pub fn display_name(&self) -> String {
        if !self.name.trim().is_empty() {
            self.name.clone()
        } else if !self.handle.is_empty() {
            format!("@{}", self.handle)
        } else {
            crate::partners::pretty_id(&self.von_fvid)
        }
    }

    pub fn ist_eingehend(&self) -> bool {
        self.richtung == Richtung::Eingehend
    }
}

/// An entry of the block list.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Blockiert {
    #[serde(default)]
    pub fvid: String,
    #[serde(default)]
    pub seit: u64,
    #[serde(default)]
    pub person: String,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub name: String,
}

impl Blockiert {
    pub fn schluessel(&self) -> String {
        key_from(&self.fvid, &self.person, &self.handle)
    }

    pub fn ist(&self, k: &str) -> bool {
        passt(k, &self.fvid, &self.person, &self.handle)
    }
}

/// What can go wrong. All messages are ready-made German sentences and
/// can be displayed directly.
#[derive(Debug, Clone, PartialEq)]
pub enum FriendError {
    /// The own ID cannot be the own friend.
    SelfRequest,
    SchonBefreundet,
    SchonAngefragt,
    /// Accept/decline without a request being present.
    NoRequest,
    NotFriend,
    Blockiert,
    UngueltigeId,
    /// An @name only works with an account (the address book hangs on the account).
    AccountRequired,
    /// The relay declined or was not reachable.
    Netz(String),
    /// The relay does not know meeting invitations yet (older version).
    KeinKanal,
    /// The friend has no (yet) FreeViewer PC an invitation could go to.
    KeinGeraet,
    /// A link with an end-to-end key must never go through the relay.
    LinkWithKey,
}

impl std::fmt::Display for FriendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            FriendError::SelfRequest => "Das ist die eigene FreeViewer-ID".to_string(),
            FriendError::SchonBefreundet => "Ihr seid schon befreundet".to_string(),
            FriendError::SchonAngefragt => "Die Anfrage laeuft schon".to_string(),
            FriendError::NoRequest => "Von dieser ID liegt keine Anfrage vor".to_string(),
            FriendError::NotFriend => "Diese ID steht nicht in der Freundesliste".to_string(),
            FriendError::Blockiert => "Diese ID steht auf der Blockliste".to_string(),
            FriendError::UngueltigeId => "Das ist keine gueltige FreeViewer-ID oder kein @Name".to_string(),
            FriendError::AccountRequired => "Fuer @Namen bitte zuerst mit dem Fleitec-Konto anmelden".to_string(),
            FriendError::Netz(m) => m.clone(),
            FriendError::KeinKanal => "Der Relay kann noch keine Einladungen zustellen".to_string(),
            FriendError::KeinGeraet => "Dieser Freund hat noch keinen FreeViewer-PC".to_string(),
            FriendError::LinkWithKey => {
                "Ein Link mit Schluessel geht nicht ueber den Relay".to_string()
            }
        };
        write!(f, "{}", s)
    }
}

impl std::error::Error for FriendError {}

/// What a background run reports back to the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// Abgleich fertig: so viele Friends, so viele offene Anfragen.
    Abgeglichen { friends: usize, offen: usize },
    /// Request has arrived at the relay.
    Angefragt(String),
    /// The other had already asked me - we are friends immediately now.
    SofortBefreundet(String),
    Angenommen(String),
    Abgelehnt(String),
    Entfernt(String),
    /// Meeting invitation has arrived at the relay (and thus at the friend).
    EinladungGesendet(String),
    /// The relay cannot deliver invitations: the link has to go to the friend by hand.
    EinladungNurLink(String),
    /// A friend invited me to a meeting (name, title).
    EinladungErhalten(String, String),
    /// The relay declined (text is ready to display).
    Fehlgeschlagen(String),
}

/// Digits only - the user types "497 628 420" or "497-628-420".
pub fn normalisieren(fvid: &str) -> String {
    fvid.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// Does this look like a FreeViewer ID? (9 or 10 digits)
pub fn ist_id(fvid: &str) -> bool {
    let d = normalisieren(fvid);
    (9..=10).contains(&d.len())
}

/// What the user types, cleaned: "@Anna " -> "@anna",
/// "k:<account>" stays, everything else -> digits only.
pub fn ziel_norm(s: &str) -> String {
    let t = s.trim();
    if let Some(h) = t.strip_prefix('@') {
        let h: String = h
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.' || *c == '-')
            .collect();
        return if h.is_empty() { String::new() } else { format!("@{}", h.to_ascii_lowercase()) };
    }
    if let Some(k) = t.strip_prefix("k:") {
        let k: String = k
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        return if k.is_empty() { String::new() } else { format!("k:{}", k) };
    }
    normalisieren(t)
}

/// Is this usable as a target? A FreeViewer ID, an @name or "k:<account>".
pub fn ist_ziel(s: &str) -> bool {
    let z = ziel_norm(s);
    z.starts_with('@') || z.starts_with("k:") || ist_id(&z)
}

/// For messages: "497 628 420", "@anna" or "dieser Kontakt".
pub fn target_label(s: &str) -> String {
    let z = ziel_norm(s);
    if z.starts_with('@') {
        z
    } else if z.starts_with("k:") {
        "dieser Kontakt".to_string()
    } else {
        crate::partners::pretty_id(&z)
    }
}

fn jetzt() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// =================================================================== Buch

/// The local copy: friends, open requests, block list.
///
/// Lives as `friends.json` in the same folder as `partners.json` and
/// `identity.txt`. Without an account this is the only version this machine
/// has; with an account it is additionally stored at the relay.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Buch {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    #[serde(rename = "freunde")]
    pub friends: Vec<Friend>,
    #[serde(default)]
    pub anfragen: Vec<Request>,
    #[serde(default)]
    pub blockiert: Vec<Blockiert>,
    /// Which FreeViewer ID this state belongs to (check on load).
    #[serde(default)]
    pub fvid: String,
    /// Which account - empty means "without account".
    #[serde(default)]
    #[serde(rename = "konto")]
    pub account: String,
    /// Wann zuletzt erfolgreich abgeglichen, unix Sekunden.
    #[serde(default)]
    pub stand: u64,
}

/// Same display name (ignoring case) as a friend in the address book?
/// A hint for the "someone is knocking" dialog - not a proof of identity.
pub fn has_friend_named(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return false;
    }
    Buch::load()
        .friends
        .iter()
        .any(|f| f.display_name().trim().to_lowercase() == name)
}

impl Buch {
    pub fn path() -> std::path::PathBuf {
        crate::ident::config_dir().join("freunde.json")
    }

    fn backup_path() -> std::path::PathBuf {
        crate::ident::config_dir().join("freunde.bak.json")
    }

    /// Loads the list.
    ///
    /// Unlike the address book, an EMPTY list is perfectly valid here -
    /// whoever removes their last friend simply has none left. The
    /// backup copy therefore only comes into play when the file is missing or
    /// broken; otherwise removed friends would reappear.
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

    /// Writes the list - first next to it, then rename, so that there is never
    /// a half file.
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

    pub fn friend(&self, fvid: &str) -> Option<&Friend> {
        let id = ziel_norm(fvid);
        self.friends.iter().find(|f| f.ist(&id))
    }

    pub fn is_friend(&self, fvid: &str) -> bool {
        self.friend(fvid).is_some()
    }

    pub fn ist_blockiert(&self, fvid: &str) -> bool {
        let id = ziel_norm(fvid);
        self.blockiert.iter().any(|b| b.ist(&id))
    }

    /// What is the entry behind this key called? For messages.
    pub fn label_for(&self, k: &str) -> String {
        let id = ziel_norm(k);
        if let Some(f) = self.friends.iter().find(|f| f.ist(&id)) {
            return f.display_name();
        }
        if let Some(a) = self.anfragen.iter().find(|a| a.ist(&id)) {
            return a.display_name();
        }
        target_label(&id)
    }

    /// Online first, then by name - that is how a friend list reads well.
    pub fn sortiert(&self) -> Vec<Friend> {
        let mut v = self.friends.clone();
        v.sort_by(|a, b| {
            b.online
                .cmp(&a.online)
                .then_with(|| a.display_name().to_lowercase().cmp(&b.display_name().to_lowercase()))
        });
        v
    }

    pub fn eingehende(&self) -> Vec<Request> {
        self.anfragen
            .iter()
            .filter(|a| a.richtung == Richtung::Eingehend)
            .cloned()
            .collect()
    }

    pub fn ausgehende(&self) -> Vec<Request> {
        self.anfragen
            .iter()
            .filter(|a| a.richtung == Richtung::Ausgehend)
            .cloned()
            .collect()
    }

    fn request_from(&self, fvid: &str, richtung: Richtung) -> Option<usize> {
        let id = ziel_norm(fvid);
        self.anfragen
            .iter()
            .position(|a| a.ist(&id) && a.richtung == richtung)
    }

    // ------------------------------------------------------- state changes
    //
    // These steps run WITHOUT network and are the places where the
    // state may change. The relay makes the same checks once
    // more - they are here so that the UI can answer immediately
    // and does not first wait on the wire.

    /// Ich stelle eine Request.
    pub fn submit_request(&mut self, fvid: &str, name: &str) -> Result<Request, FriendError> {
        let id = ziel_norm(fvid);
        if !ist_ziel(&id) {
            return Err(FriendError::UngueltigeId);
        }
        if !self.fvid.is_empty() && id == self.fvid {
            return Err(FriendError::SelfRequest);
        }
        if self.is_friend(&id) {
            return Err(FriendError::SchonBefreundet);
        }
        if self.ist_blockiert(&id) {
            return Err(FriendError::Blockiert);
        }
        if self.request_from(&id, Richtung::Ausgehend).is_some() {
            return Err(FriendError::SchonAngefragt);
        }
        // They already asked me: then both agree.
        if let Some(i) = self.request_from(&id, Richtung::Eingehend) {
            let a = self.anfragen[i].clone();
            self.annehmen(&a.gegenueber())?;
            return Ok(a);
        }
        let (von_fvid, person, handle) = if let Some(h) = id.strip_prefix('@') {
            (String::new(), String::new(), h.to_string())
        } else if let Some(k) = id.strip_prefix("k:") {
            (String::new(), k.to_string(), String::new())
        } else {
            (id, String::new(), String::new())
        };
        let a = Request {
            von_fvid,
            name: crate::presence::clean(name),
            wann: jetzt(),
            richtung: Richtung::Ausgehend,
            message: String::new(),
            online: false,
            person,
            handle,
            quelle: String::new(),
        };
        self.anfragen.push(a.clone());
        self.kuerzen();
        Ok(a)
    }

    /// A request came in from the relay. `true` if it was new.
    pub fn record_request(&mut self, mut a: Request) -> bool {
        a.von_fvid = normalisieren(&a.von_fvid);
        let k = a.gegenueber();
        if k.is_empty() || self.is_friend(&k) || self.ist_blockiert(&k) {
            return false;
        }
        if let Some(i) = self.request_from(&k, a.richtung) {
            self.anfragen[i] = a;
            return false;
        }
        self.anfragen.push(a);
        self.kuerzen();
        true
    }

    /// I accept an INCOMING request - only with this does the
    /// friendship arise. Without a pending request this is not possible; a stranger
    /// can thus never silently become a friend.
    pub fn annehmen(&mut self, fvid: &str) -> Result<Friend, FriendError> {
        let id = ziel_norm(fvid);
        if let Some(f) = self.friend(&id) {
            return Ok(f.clone()); // zweiter Klick - kein FriendError
        }
        let i = self
            .request_from(&id, Richtung::Eingehend)
            .ok_or(FriendError::NoRequest)?;
        let a = self.anfragen.remove(i);
        // an own request in the opposite direction is dropped too
        if let Some(j) = self.request_from(&id, Richtung::Ausgehend) {
            self.anfragen.remove(j);
        }
        let f = Friend {
            fvid: a.von_fvid,
            name: a.name,
            seit: jetzt(),
            online: a.online,
            gesehen: 0,
            person: a.person,
            handle: a.handle,
            geraete: Vec::new(),
            quelle: a.quelle,
        };
        self.friends.push(f.clone());
        self.kuerzen();
        Ok(f)
    }

    /// Decline (incoming) or withdraw the own request
    /// (outgoing). Both only throw the request away - NO
    /// friendship arises.
    pub fn ablehnen(&mut self, fvid: &str) -> Result<(), FriendError> {
        let id = ziel_norm(fvid);
        let vorher = self.anfragen.len();
        self.anfragen.retain(|a| !a.ist(&id));
        if self.anfragen.len() == vorher {
            return Err(FriendError::NoRequest);
        }
        Ok(())
    }

    /// Dissolve a friendship. Applies to both sides - the relay cleans it up
    /// at the other end too.
    pub fn entfernen(&mut self, fvid: &str) -> Result<(), FriendError> {
        let id = ziel_norm(fvid);
        let vorher = self.friends.len();
        self.friends.retain(|f| !f.ist(&id));
        if self.friends.len() == vorher {
            return Err(FriendError::NotFriend);
        }
        Ok(())
    }

    /// Put on the block list (or take off again). Whoever is blocked
    /// can no longer ask; open requests are dropped.
    pub fn blockieren(&mut self, fvid: &str, an: bool) -> Result<(), FriendError> {
        let id = ziel_norm(fvid);
        if !ist_ziel(&id) {
            return Err(FriendError::UngueltigeId);
        }
        if !self.fvid.is_empty() && id == self.fvid {
            return Err(FriendError::SelfRequest);
        }
        if an {
            if !self.ist_blockiert(&id) {
                let name = self.label_for(&id);
                let mut b = Blockiert {
                    seit: jetzt(),
                    name,
                    ..Default::default()
                };
                if let Some(h) = id.strip_prefix('@') {
                    b.handle = h.to_string();
                } else if let Some(k) = id.strip_prefix("k:") {
                    b.person = k.to_string();
                } else {
                    b.fvid = id.clone();
                }
                self.blockiert.push(b);
            }
            self.anfragen.retain(|a| !a.ist(&id));
        } else {
            self.blockiert.retain(|b| !b.ist(&id));
        }
        Ok(())
    }

    /// The relay has spoken. Its version applies - it is the only one both
    /// sides know. Returns whether something changed.
    pub fn uebernehmen(&mut self, stand: &Stand) -> bool {
        let mut neu = Buch {
            version: 1,
            friends: stand.friends.clone(),
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
            account: stand.account.clone(),
            stand: jetzt(),
        };
        for f in &mut neu.friends {
            f.fvid = normalisieren(&f.fvid);
            for g in &mut f.geraete {
                g.fvid = normalisieren(&g.fvid);
            }
            f.geraete.retain(|g| !g.fvid.is_empty());
        }
        for a in &mut neu.anfragen {
            a.von_fvid = normalisieren(&a.von_fvid);
        }
        for b in &mut neu.blockiert {
            b.fvid = normalisieren(&b.fvid);
        }
        // A friend from the address book may also stand there WITHOUT a PC - then
        // they are carried by their account (person) or their @name.
        neu.friends.retain(|f| !f.schluessel().is_empty());
        neu.anfragen.retain(|a| !a.gegenueber().is_empty());
        neu.blockiert.retain(|b| !b.schluessel().is_empty());
        neu.kuerzen();
        let anders = neu.friends != self.friends
            || neu.anfragen != self.anfragen
            || neu.blockiert != self.blockiert
            || neu.account != self.account;
        *self = neu;
        anders
    }

    fn kuerzen(&mut self) {
        if self.friends.len() > MAX_FRIENDS {
            self.friends.truncate(MAX_FRIENDS);
        }
        if self.anfragen.len() > MAX_ANFRAGEN {
            self.anfragen.truncate(MAX_ANFRAGEN);
        }
        self.version = 1;
    }
}

// ================================================================ Netzteil

/// The answer of the relay to `/fv/friends/stand` (and `/liste`).
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Stand {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub fvid: String,
    #[serde(default)]
    #[serde(rename = "konto")]
    pub account: String,
    #[serde(default)]
    pub friends: Vec<Friend>,
    #[serde(default)]
    pub eingehend: Vec<Request>,
    #[serde(default)]
    pub ausgehend: Vec<Request>,
    #[serde(default)]
    pub blockiert: Vec<Blockiert>,
    #[serde(default)]
    pub error: String,
}

/// `wss://host/fv/ws` -> `https://host/fv/friends/<was>`
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

/// A POST to the relay. Error codes become readable German sentences.
fn ruf(relay_url: &str, was: &str, rumpf: serde_json::Value) -> Result<serde_json::Value, FriendError> {
    let ziel = url(relay_url, was);
    let body = rumpf.to_string();
    // Also read the body on 4xx/5xx: only then do the German sentences of the
    // relay arrive (and an old relay without /einladen is recognisable as 404 "unbekannt").
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let (status, text) = match agent
        .post(&ziel)
        .header("content-type", "application/json")
        .send(body.as_str())
    {
        Ok(mut r) => {
            let code = r.status().as_u16();
            let text = r
                .body_mut()
                .read_to_string()
                .map_err(|e| FriendError::Netz(format!("Antwort unlesbar: {}", e)))?;
            (code, text)
        }
        Err(ureq::Error::StatusCode(code)) => (code, String::new()),
        Err(e) => return Err(FriendError::Netz(format!("Relay nicht erreichbar: {}", e))),
    };
    let wert: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));
    if status == 200 {
        return Ok(wert);
    }
    // A relay before patch 0005 does not know /einladen and answers 404 "unbekannt"
    // (a known path with an unknown ID says something different).
    if status == 404 && wert.get("error").and_then(|e| e.as_str()) == Some("unbekannt") {
        return Err(FriendError::KeinKanal);
    }
    let notice = wert
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
    Err(FriendError::Netz(notice))
}

/// The credentials that are in every body.
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

/// Fetch the complete state (one round trip for friends + requests + block list).
pub fn hole_stand(relay: &str, geheimnis: &str, token: &str, name: &str) -> Result<Stand, FriendError> {
    let v = ruf(relay, "stand", ausweis(geheimnis, token, name))?;
    serde_json::from_value(v).map_err(|e| FriendError::Netz(format!("Antwort unverstaendlich: {}", e)))
}

pub fn send_request(
    relay: &str,
    geheimnis: &str,
    token: &str,
    name: &str,
    an: &str,
    message: &str,
) -> Result<bool, FriendError> {
    let mut v = mit(ausweis(geheimnis, token, name), "an", &ziel_norm(an));
    v["nachricht"] = serde_json::Value::String(message.to_string());
    let a = ruf(relay, "anfragen", v)?;
    // true = the other had already asked, we are friends immediately
    Ok(a.get("angenommen").and_then(|x| x.as_bool()).unwrap_or(false))
}

pub fn sende_annehmen(relay: &str, geheimnis: &str, token: &str, name: &str, von: &str) -> Result<(), FriendError> {
    ruf(relay, "annehmen", mit(ausweis(geheimnis, token, name), "von", &ziel_norm(von))).map(|_| ())
}

pub fn sende_ablehnen(relay: &str, geheimnis: &str, token: &str, name: &str, wen: &str) -> Result<(), FriendError> {
    ruf(relay, "ablehnen", mit(ausweis(geheimnis, token, name), "von", &ziel_norm(wen))).map(|_| ())
}

pub fn sende_entfernen(relay: &str, geheimnis: &str, token: &str, name: &str, wen: &str) -> Result<(), FriendError> {
    ruf(relay, "entfernen", mit(ausweis(geheimnis, token, name), "fvid", &ziel_norm(wen))).map(|_| ())
}

pub fn sende_blockieren(
    relay: &str,
    geheimnis: &str,
    token: &str,
    name: &str,
    wen: &str,
    an: bool,
) -> Result<(), FriendError> {
    let mut v = mit(ausweis(geheimnis, token, name), "fvid", &ziel_norm(wen));
    v["an"] = serde_json::Value::Bool(an);
    ruf(relay, "blockieren", v).map(|_| ())
}

// ============================================ Meeting invitation to friends (r32)
//
// Flow: button "Meeting" -> create meeting -> invitation (only the https address)
// goes via the relay into the friend's mailbox (`/einladen`), the friend
// fetches it on the next sync (`/einladungen`) and clears it with
// `/einladung-weg`. Everything is additive: a relay without these paths
// answers 404 "unbekannt" -> `FriendError::KeinKanal`, then the old way
// stays (link in the clipboard). The end-to-end key is in the
// fragment (#k=) and NEVER goes through the relay - such links are rejected here already
// before sending (and by the relay once more).

/// A meeting invitation from a friend that is in the mailbox.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Einladung {
    /// Unique number at the relay (for clearing).
    #[serde(default)]
    pub id: String,
    /// FreeViewer-ID des Absenders.
    #[serde(default)]
    pub fvid: String,
    /// Name des Absenders (leer = ID anzeigen).
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub titel: String,
    /// https-Adresse des Meetings.
    #[serde(default)]
    pub link: String,
    /// Unix Sekunden.
    #[serde(default)]
    pub wann: u64,
    #[serde(default)]
    pub online: bool,
}

impl Einladung {
    /// Wer eingeladen hat, als Text.
    pub fn von(&self) -> String {
        if self.name.trim().is_empty() {
            crate::partners::pretty_id(&self.fvid)
        } else {
            self.name.clone()
        }
    }
}

#[derive(Deserialize, Default)]
struct EinladungenAntwort {
    #[serde(default)]
    einladungen: Vec<Einladung>,
}

/// To which PC of the friend the invitation goes: a reachable one first,
/// otherwise the first. Empty = the friend has no FreeViewer PC.
pub fn einladung_ziel(fr: &Friend) -> String {
    let g = fr
        .geraete
        .iter()
        .find(|g| g.online && ist_id(&g.fvid))
        .or_else(|| fr.geraete.iter().find(|g| ist_id(&g.fvid)));
    match g {
        Some(g) => normalisieren(&g.fvid),
        None if ist_id(&fr.fvid) => normalisieren(&fr.fvid),
        None => String::new(),
    }
}

/// The link that may go to the relay: never with a fragment (the key is in it).
pub fn link_fuer_relay(link: &str) -> Option<String> {
    let l = link.trim();
    if l.is_empty() || l.contains('#') {
        return None;
    }
    Some(l.to_string())
}

/// May a received link be opened? Only the meeting page of this
/// brand (`base`, e.g. https://meet.fleitec.com) - a friend (or whoever poses
/// as one) must not be able to open an arbitrary address in the browser.
pub fn einladung_link_ok(link: &str, base: &str) -> bool {
    let l = link.trim();
    let b = base.trim().trim_end_matches('/');
    if b.is_empty() || l.is_empty() || l.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    match l.strip_prefix(b) {
        // "https://meet.fleitec.com.evil.de" and "...com@evil.de" fail here
        Some(rest) => rest.is_empty() || rest.starts_with('/') || rest.starts_with('?'),
        None => false,
    }
}

/// Postfach abholen. `FriendError::KeinKanal` = aelterer Relay.
pub fn hole_einladungen(
    relay: &str,
    geheimnis: &str,
    token: &str,
    name: &str,
) -> Result<Vec<Einladung>, FriendError> {
    let v = ruf(relay, "einladungen", ausweis(geheimnis, token, name))?;
    let a: EinladungenAntwort = serde_json::from_value(v)
        .map_err(|e| FriendError::Netz(format!("Antwort unverstaendlich: {}", e)))?;
    Ok(a.einladungen)
}

/// Invitation to a PC of the friend. `true` = delivered.
pub fn sende_einladung(
    relay: &str,
    geheimnis: &str,
    token: &str,
    name: &str,
    an: &str,
    titel: &str,
    link: &str,
) -> Result<bool, FriendError> {
    let mut v = mit(ausweis(geheimnis, token, name), "an", &normalisieren(an));
    v["titel"] = serde_json::Value::String(titel.to_string());
    v["link"] = serde_json::Value::String(link.to_string());
    let a = ruf(relay, "einladen", v)?;
    Ok(a.get("zugestellt").and_then(|x| x.as_bool()).unwrap_or(true))
}

/// Einladung(en) im Postfach abraeumen.
pub fn sende_einladung_weg(
    relay: &str,
    geheimnis: &str,
    token: &str,
    name: &str,
    ids: &[String],
) -> Result<(), FriendError> {
    let mut v = ausweis(geheimnis, token, name);
    v["ids"] = serde_json::json!(ids);
    ruf(relay, "einladung-weg", v).map(|_| ())
}

// ================================================================= Dienst

/// The friends service: holds the list, syncs in the background and lets
/// the UI NEVER wait.
///
/// Pattern like `presence::Watch` (background thread + cache) and
/// `account::sync_async` (mailbox for the result).
///
/// ```ignore
/// let friends = friends::Dienst::vom_geraet(shared.relay_url.clone());
/// friends.launch();
/// // in every frame of the UI:
/// if let Some(m) = friends.notice() { /* show briefly */ }
/// for f in friends.liste() { /* draw row, button "Meeting" */ }
/// ```
pub struct Dienst {
    relay: String,
    geheimnis: String,
    name: Mutex<String>,
    token: Mutex<String>,
    buch: Mutex<Buch>,
    /// Mailbox for the UI. As an Arc, so that a background thread can
    /// write into it without holding on to the whole service.
    postfach: Arc<Mutex<Vec<Notice>>>,
    gestartet: AtomicBool,
    beschaeftigt: AtomicBool,
    /// "Please sync soon" - every step sets it, the thread reads it.
    sofort: Arc<AtomicBool>,
    letzter: AtomicU64,
    /// Letzter Abgleich hat geklappt?
    verbunden: AtomicBool,
    /// Received meeting invitations (cache, not on disk).
    einladungen: Mutex<Vec<Einladung>>,
    /// Invitation IDs already reported - each is reported only ONCE.
    einl_gemeldet: Mutex<std::collections::HashSet<String>>,
    /// Until when (unix s) invitations are not asked for, because the relay does not know them.
    einl_pause_bis: AtomicU64,
}

impl Dienst {
    /// With explicit secret and name (for tests and special cases).
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
            einladungen: Mutex::new(Vec::new()),
            einl_gemeldet: Mutex::new(std::collections::HashSet::new()),
            einl_pause_bis: AtomicU64::new(0),
        })
    }

    /// The normal case: secret and name come from this machine.
    pub fn vom_geraet(relay_url: String) -> Arc<Self> {
        let g = crate::ident::load_or_create_secret();
        let n = crate::presence::device_name();
        Self::neu(relay_url, g, n)
    }

    /// Konto-Zeichen setzen (`None` = abgemeldet weiterarbeiten).
    pub fn set_account(&self, token: Option<&str>) {
        let neu = token.unwrap_or("").trim().to_string();
        let mut t = self.token.lock_ok();
        if *t != neu {
            *t = neu;
            self.sofort.store(true, Ordering::SeqCst);
        }
    }

    pub fn namen_setzen(&self, name: &str) {
        *self.name.lock_ok() = crate::presence::clean(name);
    }

    pub fn angemeldet(&self) -> bool {
        !self.token.lock_ok().is_empty()
    }

    /// Did the last sync succeed?
    pub fn verbunden(&self) -> bool {
        self.verbunden.load(Ordering::Relaxed)
    }

    /// The background thread. May be called several times.
    pub fn launch(self: &Arc<Self>) {
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

    // ------------------------------------------------------- What the GUI fetches

    /// The friend list - online first. Comes from the cache,
    /// so it never waits for the network.
    pub fn liste(&self) -> Vec<Friend> {
        self.buch.lock_ok().sortiert()
    }

    /// Alle offenen Anfragen, beide Richtungen.
    pub fn offene_anfragen(&self) -> Vec<Request> {
        self.buch.lock_ok().anfragen.clone()
    }

    /// Only those I have to answer.
    pub fn eingehende(&self) -> Vec<Request> {
        self.buch.lock_ok().eingehende()
    }

    pub fn ausgehende(&self) -> Vec<Request> {
        self.buch.lock_ok().ausgehende()
    }

    pub fn blockierte(&self) -> Vec<Blockiert> {
        self.buch.lock_ok().blockiert.clone()
    }

    pub fn is_friend(&self, fvid: &str) -> bool {
        self.buch.lock_ok().is_friend(fvid)
    }

    /// Reachable friend? (Offer one-click meeting/remote support only then.)
    pub fn erreichbar(&self, fvid: &str) -> bool {
        self.buch
            .lock()
            .unwrap()
            .friend(fvid)
            .map(|f| f.online)
            .unwrap_or(false)
    }

    /// Eine Abschrift des ganzen Buches.
    pub fn buch(&self) -> Buch {
        self.buch.lock_ok().clone()
    }

    /// Next message from the mailbox (and out of it).
    pub fn notice(&self) -> Option<Notice> {
        let mut p = self.postfach.lock_ok();
        if p.is_empty() {
            None
        } else {
            Some(p.remove(0))
        }
    }

    fn melde(&self, m: Notice) {
        let mut p = self.postfach.lock_ok();
        if p.len() > 20 {
            p.remove(0);
        }
        p.push(m);
    }

    // --------------------------------------------------------- What the GUI does
    //
    // All four report back IMMEDIATELY whether the step is permissible at all
    // (self request, duplicate, not a friend ...). The network runs afterwards in
    // its own thread; its result comes via notice().

    /// Friend request to a FreeViewer ID.
    pub fn anfragen(&self, fvid: &str) -> Result<(), FriendError> {
        self.anfragen_mit(fvid, "")
    }

    /// Like `anfragen`, but with a short greeting.
    pub fn anfragen_mit(&self, fvid: &str, message: &str) -> Result<(), FriendError> {
        let id = ziel_norm(fvid);
        // @name and account ID live in the address book - that hangs on the account.
        if ist_ziel(&id) && !ist_id(&id) && !self.angemeldet() {
            return Err(FriendError::AccountRequired);
        }
        {
            let mut b = self.buch.lock_ok();
            let name = self.name.lock_ok().clone();
            b.submit_request(&id, &name)?;
            b.save();
        }
        let (relay, geheimnis, token, name) = self.zugang();
        let message = message.to_string();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            match send_request(&relay, &geheimnis, &token, &name, &id, &message) {
                Ok(true) => ich.melde(Notice::SofortBefreundet(target_label(&id))),
                Ok(false) => ich.melde(Notice::Angefragt(target_label(&id))),
                Err(e) => ich.melde(Notice::Fehlgeschlagen(e.to_string())),
            }
            ich.sofort.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Accept an incoming request - only with this are both friends.
    pub fn annehmen(&self, fvid: &str) -> Result<(), FriendError> {
        let id = ziel_norm(fvid);
        let wer;
        {
            let mut b = self.buch.lock_ok();
            wer = b.label_for(&id);
            b.annehmen(&id)?;
            b.save();
        }
        let (relay, geheimnis, token, name) = self.zugang();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            match sende_annehmen(&relay, &geheimnis, &token, &name, &id) {
                Ok(()) => ich.melde(Notice::Angenommen(wer.clone())),
                Err(e) => ich.melde(Notice::Fehlgeschlagen(e.to_string())),
            }
            ich.sofort.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Decline or withdraw the own request.
    pub fn ablehnen(&self, fvid: &str) -> Result<(), FriendError> {
        let id = ziel_norm(fvid);
        let wer;
        {
            let mut b = self.buch.lock_ok();
            wer = b.label_for(&id);
            b.ablehnen(&id)?;
            b.save();
        }
        let (relay, geheimnis, token, name) = self.zugang();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            match sende_ablehnen(&relay, &geheimnis, &token, &name, &id) {
                Ok(()) => ich.melde(Notice::Abgelehnt(wer.clone())),
                Err(e) => ich.melde(Notice::Fehlgeschlagen(e.to_string())),
            }
            ich.sofort.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Freundschaft aufloesen (bei beiden).
    pub fn entfernen(&self, fvid: &str) -> Result<(), FriendError> {
        let id = ziel_norm(fvid);
        let wer;
        {
            let mut b = self.buch.lock_ok();
            wer = b.label_for(&id);
            b.entfernen(&id)?;
            b.save();
        }
        let (relay, geheimnis, token, name) = self.zugang();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            match sende_entfernen(&relay, &geheimnis, &token, &name, &id) {
                Ok(()) => ich.melde(Notice::Entfernt(wer.clone())),
                Err(e) => ich.melde(Notice::Fehlgeschlagen(e.to_string())),
            }
            ich.sofort.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Block (`an = true`) or unblock again.
    pub fn blockieren(&self, fvid: &str, an: bool) -> Result<(), FriendError> {
        let id = ziel_norm(fvid);
        {
            let mut b = self.buch.lock_ok();
            b.blockieren(&id, an)?;
            b.save();
        }
        let (relay, geheimnis, token, name) = self.zugang();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            if let Err(e) = sende_blockieren(&relay, &geheimnis, &token, &name, &id, an) {
                ich.melde(Notice::Fehlgeschlagen(e.to_string()));
            }
            ich.sofort.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    // ------------------------------------------------- Meeting-Einladungen (r32)

    /// The invitations currently in the mailbox (newest first).
    pub fn einladungen(&self) -> Vec<Einladung> {
        self.einladungen.lock_ok().clone()
    }

    /// Send a created meeting to a friend. The link must not carry a
    /// key. Permissible immediately or not; the delivery runs in the
    /// background and reports as `EinladungGesendet` or `EinladungNurLink`
    /// (relay without mailbox: then only the clipboard helps).
    pub fn meeting_einladen(&self, fr: &Friend, titel: &str, link: &str) -> Result<(), FriendError> {
        let an = einladung_ziel(fr);
        if an.is_empty() {
            return Err(FriendError::KeinGeraet);
        }
        let link = link_fuer_relay(link).ok_or(FriendError::LinkWithKey)?;
        let titel = titel.trim().to_string();
        let wer = fr.display_name();
        let (relay, geheimnis, token, name) = self.zugang();
        let ich = self.klon_briefkasten();
        std::thread::spawn(move || {
            match sende_einladung(&relay, &geheimnis, &token, &name, &an, &titel, &link) {
                Ok(_) => ich.melde(Notice::EinladungGesendet(wer)),
                Err(FriendError::KeinKanal) => ich.melde(Notice::EinladungNurLink(wer)),
                Err(e) => ich.melde(Notice::Fehlgeschlagen(e.to_string())),
            }
        });
        Ok(())
    }

    /// Tick off an invitation (accepted or discarded): it disappears
    /// here immediately, at the relay in the background.
    pub fn einladung_erledigt(&self, id: &str) {
        self.einladungen.lock_ok().retain(|e| e.id != id);
        let (relay, geheimnis, token, name) = self.zugang();
        let ids = vec![id.to_string()];
        std::thread::spawn(move || {
            let _ = sende_einladung_weg(&relay, &geheimnis, &token, &name, &ids);
        });
    }

    /// Take over a new mailbox state; reports every NEW invitation exactly once.
    /// Returns how many were new.
    pub fn einladungen_uebernehmen(&self, mut liste: Vec<Einladung>) -> usize {
        liste.retain(|e| !e.id.is_empty() && !e.link.is_empty());
        if liste.len() > 50 {
            liste.drain(50..);
        }
        let mut neu = Vec::new();
        {
            let mut gemeldet = self.einl_gemeldet.lock_ok();
            for e in &liste {
                if gemeldet.insert(e.id.clone()) {
                    neu.push(e.clone());
                }
            }
            if gemeldet.len() > 500 {
                gemeldet.clear();
                gemeldet.extend(liste.iter().map(|e| e.id.clone()));
            }
        }
        *self.einladungen.lock_ok() = liste;
        for e in &neu {
            self.melde(Notice::EinladungErhalten(e.von(), e.titel.clone()));
        }
        neu.len()
    }

    fn einladungen_holen(&self) {
        if jetzt() < self.einl_pause_bis.load(Ordering::Relaxed) {
            return;
        }
        let (relay, geheimnis, token, name) = self.zugang();
        match hole_einladungen(&relay, &geheimnis, &token, &name) {
            Ok(liste) => {
                self.einladungen_uebernehmen(liste);
            }
            // Older relay: stop asking for a while (it can be updated, after all).
            Err(FriendError::KeinKanal) => self
                .einl_pause_bis
                .store(jetzt() + EINLADUNG_PAUSE_SEKUNDEN, Ordering::Relaxed),
            Err(_) => {}
        }
    }

    /// Sofortigen Abgleich anstossen (kehrt sofort go_back).
    pub fn synchronisieren(&self) {
        self.sofort.store(true, Ordering::SeqCst);
    }

    /// Sync NOW and here - blocks. Only for the background thread and
    /// headless operation; the UI uses `synchronisieren()`.
    pub fn abgleich_blockierend(&self) -> Result<Stand, FriendError> {
        let (relay, geheimnis, token, name) = self.zugang();
        let stand = hole_stand(&relay, &geheimnis, &token, &name)?;
        let mut b = self.buch.lock_ok();
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
                {
                    let b = self.buch.lock_ok();
                    self.melde(Notice::Abgeglichen {
                        friends: b.friends.len(),
                        offen: b.anfragen.len(),
                    });
                }
                self.einladungen_holen();
            }
            Err(e) => {
                self.verbunden.store(false, Ordering::Relaxed);
                self.melde(Notice::Fehlgeschlagen(e.to_string()));
            }
        }
        self.beschaeftigt.store(false, Ordering::SeqCst);
    }

    fn zugang(&self) -> (String, String, String, String) {
        (
            self.relay.clone(),
            self.geheimnis.clone(),
            self.token.lock_ok().clone(),
            self.name.lock_ok().clone(),
        )
    }

    /// A small handle with which a background thread can report and trigger the
    /// next sync - it needs nothing more.
    fn klon_briefkasten(&self) -> Briefkasten {
        Briefkasten {
            postfach: Arc::clone(&self.postfach),
            sofort: Arc::clone(&self.sofort),
        }
    }
}

/// A small handle for background threads.
struct Briefkasten {
    postfach: Arc<Mutex<Vec<Notice>>>,
    sofort: Arc<AtomicBool>,
}

impl Briefkasten {
    fn melde(&self, m: Notice) {
        let mut p = self.postfach.lock_ok();
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

    /// Own throwaway folder - a test NEVER touches the real configuration
    /// (the data loss of 31.07.2026 came from exactly that).
    fn own_dir(name: &str) -> std::path::PathBuf {
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

    fn eingehend(von: &str, name: &str) -> Request {
        Request {
            von_fvid: von.to_string(),
            name: name.to_string(),
            wann: 1_700_000_000,
            richtung: Richtung::Eingehend,
            message: String::new(),
            online: true,
            ..Default::default()
        }
    }

    #[test]
    fn ein_test_schreibt_nie_in_die_echte_konfiguration() {
        own_dir("isoliert");
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
        own_dir("speichern");
        let mut b = buch_mit_mir("111111111");
        b.record_request(eingehend("222222222", "Laptop"));
        b.annehmen("222222222").unwrap();
        b.record_request(eingehend("333333333", "Handy"));
        b.blockieren("444444444", true).unwrap();
        b.save();

        let wieder = Buch::load();
        assert_eq!(wieder.friends.len(), 1);
        assert_eq!(wieder.friends[0].fvid, "222222222");
        assert_eq!(wieder.friends[0].name, "Laptop");
        assert_eq!(wieder.eingehende().len(), 1);
        assert_eq!(wieder.eingehende()[0].von_fvid, "333333333");
        assert!(wieder.ist_blockiert("444444444"));
        assert_eq!(wieder.fvid, "111111111");
    }

    #[test]
    fn empty_list_is_valid_and_fetches_nothing() {
        own_dir("leer");
        let mut b = buch_mit_mir("111111111");
        b.record_request(eingehend("222222222", "Weg"));
        b.annehmen("222222222").unwrap();
        b.save();
        // second write creates the backup copy
        b.entfernen("222222222").unwrap();
        b.save();
        let wieder = Buch::load();
        assert_eq!(wieder.friends.len(), 0, "entfernte Freunde duerfen nicht zurueckkommen");
    }

    #[test]
    fn broken_file_falls_back_to_backup() {
        own_dir("kaputt");
        let mut b = buch_mit_mir("111111111");
        b.record_request(eingehend("222222222", "Laptop"));
        b.annehmen("222222222").unwrap();
        b.save();
        b.save(); // jetzt existiert die Sicherungskopie
        std::fs::write(Buch::path(), b"{ kaputt").unwrap();
        let wieder = Buch::load();
        assert_eq!(wieder.friends.len(), 1);
        assert_eq!(wieder.friends[0].fvid, "222222222");
    }

    #[test]
    fn request_to_self_is_rejected() {
        let mut b = buch_mit_mir("111111111");
        assert_eq!(b.submit_request("111111111", "Ich"), Err(FriendError::SelfRequest));
        // also typed with spaces
        assert_eq!(b.submit_request("111 111 111", "Ich"), Err(FriendError::SelfRequest));
        assert!(b.anfragen.is_empty());
    }

    #[test]
    fn eine_unsinnige_id_wird_abgewiesen() {
        let mut b = buch_mit_mir("111111111");
        assert_eq!(b.submit_request("123", "Kurz"), Err(FriendError::UngueltigeId));
        assert_eq!(b.submit_request("", "Nichts"), Err(FriendError::UngueltigeId));
        assert_eq!(b.submit_request("abc", "Buchstaben"), Err(FriendError::UngueltigeId));
    }

    #[test]
    fn same_request_twice_is_rejected() {
        let mut b = buch_mit_mir("111111111");
        b.submit_request("222222222", "Ich").unwrap();
        assert_eq!(b.submit_request("222222222", "Ich"), Err(FriendError::SchonAngefragt));
        // andere Schreibweise, gleiche ID
        assert_eq!(b.submit_request("222 222 222", "Ich"), Err(FriendError::SchonAngefragt));
        assert_eq!(b.ausgehende().len(), 1);
    }

    #[test]
    fn request_to_a_friend_is_rejected() {
        let mut b = buch_mit_mir("111111111");
        b.record_request(eingehend("222222222", "Laptop"));
        b.annehmen("222222222").unwrap();
        assert_eq!(b.submit_request("222222222", "Ich"), Err(FriendError::SchonBefreundet));
    }

    #[test]
    fn nobody_becomes_friend_silently_without_request() {
        let mut b = buch_mit_mir("111111111");
        // request from a complete stranger is not present
        assert_eq!(b.annehmen("999999999"), Err(FriendError::NoRequest));
        assert!(b.friends.is_empty());
        // an OUTGOING request does not make me a friend - the other
        // has to agree
        b.submit_request("222222222", "Ich").unwrap();
        assert_eq!(b.annehmen("222222222"), Err(FriendError::NoRequest));
        assert!(b.friends.is_empty());
    }

    #[test]
    fn accept_turns_request_into_friend() {
        let mut b = buch_mit_mir("111111111");
        b.record_request(eingehend("222222222", "Laptop"));
        assert_eq!(b.eingehende().len(), 1);
        let f = b.annehmen("222222222").unwrap();
        assert_eq!(f.fvid, "222222222");
        assert_eq!(f.name, "Laptop");
        assert!(f.seit > 0);
        assert!(b.is_friend("222222222"));
        assert!(b.anfragen.is_empty(), "die Anfrage muss aus der Liste raus");
        // second click is not an error
        assert!(b.annehmen("222222222").is_ok());
        assert_eq!(b.friends.len(), 1);
    }

    #[test]
    fn mutual_requests_make_friends_immediately() {
        let mut b = buch_mit_mir("111111111");
        b.record_request(eingehend("222222222", "Laptop"));
        b.submit_request("222222222", "Ich").unwrap();
        assert!(b.is_friend("222222222"));
        assert!(b.anfragen.is_empty());
    }

    #[test]
    fn reject_drops_request_without_friendship() {
        let mut b = buch_mit_mir("111111111");
        b.record_request(eingehend("222222222", "Laptop"));
        b.ablehnen("222222222").unwrap();
        assert!(b.anfragen.is_empty());
        assert!(!b.is_friend("222222222"));
        assert_eq!(b.ablehnen("222222222"), Err(FriendError::NoRequest));
    }

    #[test]
    fn own_request_can_be_withdrawn() {
        let mut b = buch_mit_mir("111111111");
        b.submit_request("222222222", "Ich").unwrap();
        assert_eq!(b.ausgehende().len(), 1);
        b.ablehnen("222222222").unwrap();
        assert!(b.anfragen.is_empty());
        assert!(!b.is_friend("222222222"));
    }

    #[test]
    fn remove_takes_friend_off_the_list() {
        let mut b = buch_mit_mir("111111111");
        b.record_request(eingehend("222222222", "Laptop"));
        b.annehmen("222222222").unwrap();
        b.entfernen("222222222").unwrap();
        assert!(!b.is_friend("222222222"));
        assert_eq!(b.entfernen("222222222"), Err(FriendError::NotFriend));
    }

    #[test]
    fn blockieren_wirft_offene_anfragen_weg_und_sperrt_neue() {
        let mut b = buch_mit_mir("111111111");
        b.record_request(eingehend("222222222", "Nervt"));
        b.blockieren("222222222", true).unwrap();
        assert!(b.anfragen.is_empty());
        assert!(b.ist_blockiert("222222222"));
        assert_eq!(b.submit_request("222222222", "Ich"), Err(FriendError::Blockiert));
        // eine hereinkommende Request prallt ab
        assert!(!b.record_request(eingehend("222222222", "Nervt")));
        assert!(b.anfragen.is_empty());
        // freigeben
        b.blockieren("222222222", false).unwrap();
        assert!(!b.ist_blockiert("222222222"));
        assert!(b.record_request(eingehend("222222222", "Nervt")));
    }

    #[test]
    fn eingehend_und_ausgehend_bleiben_getrennt() {
        let mut b = buch_mit_mir("111111111");
        b.record_request(eingehend("222222222", "A"));
        b.submit_request("333333333", "Ich").unwrap();
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
        b.submit_request("999999999", "Ich").unwrap(); // nur lokal geraten
        let stand = Stand {
            ok: true,
            fvid: "111111111".into(),
            account: "justin".into(),
            friends: vec![Friend {
                fvid: "222222222".into(),
                name: "Laptop".into(),
                seit: 1_700_000_000,
                online: true,
                gesehen: 1_700_000_000_000,
                ..Default::default()
            }],
            eingehend: vec![eingehend("333333333", "Handy")],
            ausgehend: vec![],
            blockiert: vec![Blockiert { fvid: "444444444".into(), seit: 1, ..Default::default() }],
            error: String::new(),
        };
        assert!(b.uebernehmen(&stand));
        assert_eq!(b.friends.len(), 1);
        assert_eq!(b.friends[0].fvid, "222222222");
        assert_eq!(b.eingehende().len(), 1);
        assert_eq!(b.ausgehende().len(), 0, "die lokale Vermutung faellt weg");
        assert_eq!(b.account, "justin");
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
            // the relay sends "richtung" in both lists, deliberately
            // the wrong way round here - the assignment by list must win
            eingehend: vec![Request {
                von_fvid: "222222222".into(),
                richtung: Richtung::Ausgehend,
                ..Default::default()
            }],
            ausgehend: vec![Request {
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
        b.friends = vec![
            Friend { fvid: "222222222".into(), name: "Zebra".into(), online: false, ..Default::default() },
            Friend { fvid: "333333333".into(), name: "Anton".into(), online: false, ..Default::default() },
            Friend { fvid: "444444444".into(), name: "Willi".into(), online: true, ..Default::default() },
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
        // and back again - exactly how the relay sends it
        let roh = r#"{"fvid":"333333333","name":"Handy","wann":5,"richtung":"ausgehend","nachricht":"hi","online":true}"#;
        let b: Request = serde_json::from_str(roh).unwrap();
        assert_eq!(b.von_fvid, "333333333");
        assert_eq!(b.richtung, Richtung::Ausgehend);
        assert_eq!(b.message, "hi");
    }

    #[test]
    fn relay_reply_is_parseable() {
        // woertlich so, wie relay.js /fv/friends/stand antwortet
        let roh = r#"{"ok":true,"fvid":"7902410486","konto":"","freunde":[
            {"fvid":"8358538558","name":"FV-Test-B","seit":1786250165,"online":true,"gesehen":1786250165580}],
            "eingehend":[{"fvid":"2130270636","name":"FV-Test-C","wann":1786250165,"richtung":"eingehend","nachricht":"","online":true}],
            "ausgehend":[],"blockiert":[]}"#;
        let s: Stand = serde_json::from_str(roh).unwrap();
        assert!(s.ok);
        assert_eq!(s.friends.len(), 1);
        assert_eq!(s.friends[0].name, "FV-Test-B");
        assert!(s.friends[0].online);
        assert_eq!(s.eingehend.len(), 1);
        assert_eq!(s.eingehend[0].von_fvid, "2130270636");
    }

    #[test]
    fn label_falls_back_to_id() {
        let f = Friend { fvid: "497628420".into(), ..Default::default() };
        assert_eq!(f.display_name(), "497 628 420");
        let g = Friend { fvid: "497628420".into(), name: "Buero-PC".into(), ..Default::default() };
        assert_eq!(g.display_name(), "Buero-PC");
        assert_eq!(
            Friend { online: true, ..Default::default() }.zuletzt(),
            "online"
        );
    }

    #[test]
    fn namen_bleiben_harmlos() {
        let mut b = buch_mit_mir("111111111");
        let a = b.submit_request("222222222", "  Pa\"tis {Laptop}  ").unwrap();
        assert_eq!(a.name, "Patis Laptop");
    }

    // ------------------------------------------------ r7: ein Adressbuch

    /// This is exactly how the relay answers for an account since r7 (buch.js).
    const BUCH_STAND: &str = r#"{"ok":true,"fvid":"111111111","konto":"anna","handle":"anna","person":"u_ANNA","buch":true,
        "freunde":[
          {"fvid":"222222222","name":"Bert","seit":1790000000,"online":true,"gesehen":0,"person":"u_BERT","handle":"bert",
           "geraete":[{"fvid":"222222222","label":"Bert-PC","online":true,"gesehen":0},{"fvid":"333333333","label":"Bert-Laptop","online":false,"gesehen":5}],"quelle":"buch"},
          {"fvid":"","name":"Carla","seit":1790000001,"online":false,"gesehen":0,"person":"u_CARLA","handle":"carla","geraete":[],"quelle":"buch"},
          {"fvid":"444444444","name":"Dora-PC","seit":1790000002,"online":false,"gesehen":0,"quelle":"fv"}],
        "eingehend":[{"fvid":"","name":"Emil","wann":1790000003,"richtung":"eingehend","nachricht":"","online":false,"person":"u_EMIL","handle":"emil","quelle":"buch"}],
        "ausgehend":[{"fvid":"","name":"","wann":1790000004,"richtung":"ausgehend","nachricht":"","online":false,"person":"u_FRIDA","handle":"frida","quelle":"buch"}],
        "blockiert":[{"fvid":"","seit":1790000005,"person":"u_GUSTAV","handle":"gustav","name":"Gustav","quelle":"buch"}]}"#;

    fn buch_aus_dem_adressbuch() -> Buch {
        let s: Stand = serde_json::from_str(BUCH_STAND).unwrap();
        let mut b = buch_mit_mir("111111111");
        assert!(b.uebernehmen(&s));
        b
    }

    #[test]
    fn der_stand_aus_dem_adressbuch_wird_ganz_uebernommen() {
        let b = buch_aus_dem_adressbuch();
        assert_eq!(b.account, "anna");
        // Carla hat keinen PC - frueher waere sie still weggefallen
        assert_eq!(b.friends.len(), 3);
        let carla = b.friend("k:u_CARLA").expect("Carla fehlt");
        assert!(!carla.hat_pc());
        assert!(!carla.erreichbar());
        assert_eq!(carla.schluessel(), "k:u_CARLA");
        // requests without a PC number stay, via the account
        assert_eq!(b.eingehende().len(), 1);
        assert_eq!(b.eingehende()[0].gegenueber(), "k:u_EMIL");
        assert_eq!(b.ausgehende()[0].display_name(), "@frida");
        assert!(b.ist_blockiert("@gustav"));
        assert!(b.ist_blockiert("k:u_GUSTAV"));
    }

    #[test]
    fn friend_is_found_by_any_pc_and_name() {
        let b = buch_aus_dem_adressbuch();
        assert!(b.is_friend("222 222 222"));
        assert!(b.is_friend("333333333")); // sein Laptop
        assert!(b.is_friend("@Bert"));
        assert!(b.is_friend("k:u_BERT"));
        assert_eq!(b.friend("333333333").unwrap().name, "Bert");
        assert!(!b.is_friend("555555555"));
        assert!(!b.is_friend("@niemand"));
        // the old friend without account stays as it was
        assert!(b.is_friend("444444444"));
    }

    #[test]
    fn accept_via_account() {
        let mut b = buch_aus_dem_adressbuch();
        let f = b.annehmen("k:u_EMIL").unwrap();
        assert_eq!(f.person, "u_EMIL");
        assert_eq!(f.handle, "emil");
        assert!(b.eingehende().is_empty());
        assert!(b.is_friend("@emil"));
        assert_eq!(b.annehmen("@emil").unwrap().person, "u_EMIL"); // zweiter Klick
    }

    #[test]
    fn remove_reject_block_via_account() {
        let mut b = buch_aus_dem_adressbuch();
        b.entfernen("k:u_CARLA").unwrap();
        assert!(!b.is_friend("k:u_CARLA"));
        assert_eq!(b.entfernen("k:u_CARLA"), Err(FriendError::NotFriend));
        b.ablehnen("@frida").unwrap();
        assert!(b.ausgehende().is_empty());
        b.blockieren("k:u_EMIL", true).unwrap();
        assert!(b.eingehende().is_empty());
        assert!(b.ist_blockiert("k:u_EMIL"));
        b.blockieren("k:u_EMIL", false).unwrap();
        assert!(!b.ist_blockiert("k:u_EMIL"));
        b.blockieren("@gustav", false).unwrap();
        assert!(b.blockiert.is_empty());
    }

    #[test]
    fn request_by_name_is_stored_as_name() {
        let mut b = buch_mit_mir("111111111");
        let a = b.submit_request("@Zora", "").unwrap();
        assert_eq!(a.handle, "zora");
        assert_eq!(a.von_fvid, "");
        assert_eq!(a.gegenueber(), "@zora");
        assert_eq!(b.submit_request("@zora", ""), Err(FriendError::SchonAngefragt));
        assert_eq!(b.submit_request("@", ""), Err(FriendError::UngueltigeId));
        assert_eq!(b.submit_request("hallo", ""), Err(FriendError::UngueltigeId));
    }

    #[test]
    fn ziele_werden_bereinigt() {
        assert_eq!(ziel_norm(" @Anna.B "), "@anna.b");
        assert_eq!(ziel_norm("@a b<c>"), "@abc");
        assert_eq!(ziel_norm("k:u_ANNA"), "k:u_ANNA");
        assert_eq!(ziel_norm("k:u/../x"), "k:ux");
        assert_eq!(ziel_norm("497 628 420"), "497628420");
        assert!(ist_ziel("@anna"));
        assert!(ist_ziel("k:u_ANNA"));
        assert!(ist_ziel("497628420"));
        assert!(!ist_ziel("@"));
        assert!(!ist_ziel("12"));
        assert_eq!(target_label("497628420"), "497 628 420");
        assert_eq!(target_label("@Anna"), "@anna");
    }

    #[test]
    fn name_and_account_need_a_login() {
        own_dir("konto-noetig");
        let d = Dienst::neu("ws://127.0.0.1:9/fv/ws".into(), "00".repeat(32), "Test".into());
        assert_eq!(d.anfragen("@anna"), Err(FriendError::AccountRequired));
        assert_eq!(d.anfragen("k:u_ANNA"), Err(FriendError::AccountRequired));
        assert!(d.liste().is_empty());
    }

    #[test]
    fn die_alte_relay_antwort_geht_weiter() {
        // FreeViewer without account and a relay before r7: no new fields
        let roh = r#"{"ok":true,"fvid":"111111111","konto":"","freunde":[{"fvid":"222222222","name":"X","seit":1,"online":false,"gesehen":0}],
            "eingehend":[],"ausgehend":[],"blockiert":[{"fvid":"333333333","seit":2}]}"#;
        let s: Stand = serde_json::from_str(roh).unwrap();
        let mut b = buch_mit_mir("111111111");
        b.uebernehmen(&s);
        assert!(b.is_friend("222222222"));
        assert_eq!(b.friends[0].schluessel(), "222222222");
        assert!(b.ist_blockiert("333333333"));
    }

    // ------------------------------------------------ r32: Meeting-Einladung

    fn friend_with(fvid: &str, geraete: &[(&str, bool)]) -> Friend {
        Friend {
            fvid: fvid.to_string(),
            name: "Anna".to_string(),
            geraete: geraete
                .iter()
                .map(|(id, on)| FriendDevice {
                    fvid: id.to_string(),
                    label: String::new(),
                    online: *on,
                    gesehen: 0,
                })
                .collect(),
            ..Default::default()
        }
    }

    /// A mini relay: accepts ONE connection, remembers the request and
    /// answers with status + body. Returns (port, received).
    fn mini_relay(status: u16, rumpf: &'static str) -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                // bis Kopf + Rumpf da sind
                loop {
                    let n = c.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let len = text
                            .lines()
                            .find_map(|z| {
                                z.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if buf.len() >= i + 4 + len {
                            break;
                        }
                    }
                }
                let _ = tx.send(String::from_utf8_lossy(&buf).to_string());
                let antwort = format!(
                    "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    status,
                    rumpf.len(),
                    rumpf
                );
                let _ = c.write_all(antwort.as_bytes());
            }
        });
        (port, rx)
    }

    #[test]
    fn die_einladung_geht_an_den_erreichbaren_pc() {
        let f = friend_with("111111111", &[("111111111", false), ("222222222", true)]);
        assert_eq!(einladung_ziel(&f), "222222222");
        let f = friend_with("111111111", &[("111111111", false), ("222222222", false)]);
        assert_eq!(einladung_ziel(&f), "111111111");
    }

    #[test]
    fn ohne_pc_gibt_es_kein_ziel() {
        let f = Friend {
            person: "u_anna".into(),
            ..Default::default()
        };
        assert_eq!(einladung_ziel(&f), "");
        // only the old list: the ID itself
        let f = Friend {
            fvid: "333 444 555".into(),
            ..Default::default()
        };
        assert_eq!(einladung_ziel(&f), "333444555");
    }

    #[test]
    fn link_with_key_never_goes_to_relay() {
        assert_eq!(
            link_fuer_relay(" https://meet.fleitec.com/?room=1&pass=2 ").as_deref(),
            Some("https://meet.fleitec.com/?room=1&pass=2")
        );
        assert!(link_fuer_relay("https://meet.fleitec.com/?room=1#k=abc").is_none());
        assert!(link_fuer_relay("   ").is_none());
    }

    #[test]
    fn nur_die_eigene_meeting_seite_wird_geoeffnet() {
        let b = "https://meet.fleitec.com";
        assert!(einladung_link_ok("https://meet.fleitec.com/?room=1-2-3&pass=x", b));
        assert!(einladung_link_ok("https://meet.fleitec.com", b));
        assert!(einladung_link_ok("https://meet.fleitec.com?room=1", b));
        // base with a slash at the end is the same
        assert!(einladung_link_ok("https://meet.fleitec.com/?room=1", "https://meet.fleitec.com/"));
        assert!(!einladung_link_ok("https://meet.fleitec.com.evil.de/?room=1", b));
        assert!(!einladung_link_ok("https://meet.fleitec.com@evil.de/", b));
        assert!(!einladung_link_ok("http://meet.fleitec.com/?room=1", b));
        assert!(!einladung_link_ok("file:///C:/Windows/System32/calc.exe", b));
        assert!(!einladung_link_ok("https://meet.fleitec.com/?room=1 --x", b));
        assert!(!einladung_link_ok("", b));
        assert!(!einladung_link_ok("https://meet.fleitec.com/", ""));
    }

    #[test]
    fn invites_reply_is_parseable() {
        let roh = r#"{"ok":true,"fvid":"1","konto":"","einladungen":[
            {"id":"a1","fvid":"222222222","name":"Anna","titel":"Mit Justin","link":"https://meet.fleitec.com/?room=1&pass=2","wann":5,"online":true,"neues_feld":1}]}"#;
        let a: EinladungenAntwort = serde_json::from_str(roh).unwrap();
        assert_eq!(a.einladungen.len(), 1);
        assert_eq!(a.einladungen[0].von(), "Anna");
        assert_eq!(a.einladungen[0].titel, "Mit Justin");
        // without a mailbox field (old relay with 200?) it stays empty
        let a: EinladungenAntwort = serde_json::from_str(r#"{"ok":true}"#).unwrap();
        assert!(a.einladungen.is_empty());
    }

    #[test]
    fn jede_neue_einladung_wird_nur_einmal_gemeldet() {
        own_dir("einl-melden");
        let d = Dienst::neu("ws://127.0.0.1:9/fv/ws".into(), "00".repeat(32), "Test".into());
        let e = |id: &str| Einladung {
            id: id.into(),
            fvid: "222222222".into(),
            name: "Anna".into(),
            titel: "Mit Justin".into(),
            link: "https://meet.fleitec.com/?room=1&pass=2".into(),
            ..Default::default()
        };
        assert_eq!(d.einladungen_uebernehmen(vec![e("a"), e("b")]), 2);
        assert_eq!(d.einladungen().len(), 2);
        // gleicher Stand beim naechsten Abgleich: nichts Neues
        assert_eq!(d.einladungen_uebernehmen(vec![e("a"), e("b")]), 0);
        assert_eq!(d.einladungen_uebernehmen(vec![e("a"), e("b"), e("c")]), 1);
        let mut erhalten = 0;
        while let Some(m) = d.notice() {
            if let Notice::EinladungErhalten(von, titel) = m {
                assert_eq!(von, "Anna");
                assert_eq!(titel, "Mit Justin");
                erhalten += 1;
            }
        }
        assert_eq!(erhalten, 3);
        // entries without ID or link are junk and are ignored
        assert_eq!(
            d.einladungen_uebernehmen(vec![Einladung::default(), Einladung { id: "x".into(), ..Default::default() }]),
            0
        );
        assert!(d.einladungen().is_empty());
    }

    #[test]
    fn invite_link_with_key_rejected_before_send() {
        own_dir("einl-schluessel");
        let d = Dienst::neu("ws://127.0.0.1:9/fv/ws".into(), "00".repeat(32), "Test".into());
        let f = friend_with("111111111", &[("111111111", true)]);
        assert_eq!(
            d.meeting_einladen(&f, "x", "https://meet.fleitec.com/?room=1#k=abc"),
            Err(FriendError::LinkWithKey)
        );
        let ohne_pc = Friend {
            person: "u_anna".into(),
            ..Default::default()
        };
        assert_eq!(
            d.meeting_einladen(&ohne_pc, "x", "https://meet.fleitec.com/?room=1"),
            Err(FriendError::KeinGeraet)
        );
    }

    #[test]
    fn ein_relay_ohne_postfach_ist_kein_kanal() {
        // 404 {"error":"unbekannt"} = Relay vor Patch 0005
        let (port, rx) = mini_relay(404, r#"{"error":"unbekannt"}"#);
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        let r = sende_einladung(&relay, &"ab".repeat(16), "", "Test", "222222222", "T", "https://meet.fleitec.com/?room=1");
        assert_eq!(r, Err(FriendError::KeinKanal));
        let roh = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(roh.starts_with("POST /fv/freunde/einladen "), "{}", roh);

        // an unknown ID, in contrast, is a normal error (the path exists)
        let (port, _rx) = mini_relay(404, r#"{"error":"Diese FreeViewer-ID gibt es nicht"}"#);
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        let r = sende_einladung(&relay, &"ab".repeat(16), "", "Test", "222222222", "T", "https://meet.fleitec.com/?room=1");
        assert_eq!(r, Err(FriendError::Netz("Diese FreeViewer-ID gibt es nicht".into())));
    }

    #[test]
    fn die_einladung_traegt_ziel_titel_und_link_zum_relay() {
        let (port, rx) = mini_relay(200, r#"{"ok":true,"zugestellt":true}"#);
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        let r = sende_einladung(
            &relay,
            &"ab".repeat(16),
            "tok",
            "Mein PC",
            "222 222 222",
            "Mit Anna",
            "https://meet.fleitec.com/?room=1&pass=2",
        );
        assert_eq!(r, Ok(true));
        let roh = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let rumpf = roh.split("\r\n\r\n").nth(1).unwrap_or("");
        let v: serde_json::Value = serde_json::from_str(rumpf).unwrap();
        assert_eq!(v["an"], "222222222");
        assert_eq!(v["titel"], "Mit Anna");
        assert_eq!(v["link"], "https://meet.fleitec.com/?room=1&pass=2");
        assert_eq!(v["secret"], "ab".repeat(16));
        assert_eq!(v["token"], "tok");
        assert!(!rumpf.contains("#k="));
    }

    #[test]
    fn das_postfach_holen_und_abraeumen() {
        let (port, rx) = mini_relay(
            200,
            r#"{"ok":true,"einladungen":[{"id":"e1","fvid":"222222222","name":"Anna","titel":"T","link":"https://meet.fleitec.com/?room=1&pass=2","wann":1}]}"#,
        );
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        let l = hole_einladungen(&relay, &"ab".repeat(16), "", "Test").unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].id, "e1");
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().starts_with("POST /fv/freunde/einladungen "));

        let (port, rx) = mini_relay(200, r#"{"ok":true,"weg":1}"#);
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        assert_eq!(
            sende_einladung_weg(&relay, &"ab".repeat(16), "", "Test", &["e1".to_string()]),
            Ok(())
        );
        let roh = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(roh.starts_with("POST /fv/freunde/einladung-weg "));
        assert!(roh.contains(r#""ids":["e1"]"#), "{}", roh);
    }
}
