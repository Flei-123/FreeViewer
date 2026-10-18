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
//! a friend can have several PCs (`devices`), or none at all - then
//! `fvid` is empty and they are addressed via `person` (account ID) or
//! `@handle`. Without an account everything stays on the FreeViewer ID as before.
//! The key of an entry is therefore `entry_key()`: the number,
//! otherwise "k:<person>", otherwise "@handle" - the relay understands exactly that too.
//!
//! The client identifies itself exactly as with the WebSocket login
//! (`net::json_register`): with the secret from `identity.txt`. It is
//! ALWAYS in the request body, never in the URL.
//!
//! Nothing here blocks the UI. Every network call runs in its
//! own thread; results come back via `Service::notice()` - the same
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
const SYNC_SECONDS: u64 = 20;

/// If the relay does not know invitations, it is not asked for them for this long.
const INVITE_PAUSE_SECONDS: u64 = 600;

/// Upper limit, so that a broken answer cannot blow up the file.
const MAX_FRIENDS: usize = 500;
const MAX_REQUEST: usize = 200;

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
    #[serde(rename = "seit")]
    pub since: u64,
    /// Is the friend reachable right now? Comes from the relay.
    #[serde(default)]
    pub online: bool,
    /// Zuletzt seen, unix MILLIsekunden (wie in `presence.rs`).
    #[serde(default)]
    #[serde(rename = "gesehen")]
    pub seen: u64,
    /// Account ID of the person in the address book (empty = friend only via ID).
    #[serde(default)]
    pub person: String,
    /// Their @username (without @), if they have an account.
    #[serde(default)]
    pub handle: String,
    /// All FreeViewer PCs of this person, online first. `fvid` is the first.
    #[serde(default)]
    #[serde(rename = "geraete")]
    pub devices: Vec<FriendDevice>,
    /// "buch" (address book) or "fv" (old list at the relay, without account).
    #[serde(default)]
    #[serde(rename = "quelle")]
    pub origin: String,
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
    #[serde(rename = "gesehen")]
    pub seen: u64,
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

/// Does key `k` (already cleaned with `target_norm`) match this
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

    pub fn entry_key(&self) -> String {
        key_from(&self.fvid, &self.person, &self.handle)
    }

    /// Is `k` this friend - via any of their PCs, their account or
    /// their @name?
    pub fn ist(&self, k: &str) -> bool {
        passt(k, &self.fvid, &self.person, &self.handle)
            || (!k.is_empty() && self.devices.iter().any(|g| g.fvid == k))
    }

    /// Does the friend have a PC one can connect to?
    pub fn hat_pc(&self) -> bool {
        !self.fvid.is_empty()
    }

    /// "just now", "3 min ago" - same wording as the address book.
    pub fn lately(&self) -> String {
        if self.online {
            "online".to_string()
        } else {
            crate::presence::ago_ms(self.seen)
        }
    }

    /// Can the friend be rung up right now? (Only with a PC.)
    pub fn reachable(&self) -> bool {
        self.online && self.hat_pc()
    }
}

/// Who asked - me or the other?
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Someone wants me as a friend. I have to accept or decline.
    #[serde(rename = "Eingehend")]
    Incoming,
    /// I asked and am waiting for the answer.
    #[serde(rename = "Ausgehend")]
    Outgoing,
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Incoming
    }
}

/// Eine offene Freundschaftsanfrage.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Request {
    /// The OTHER side: for `Incoming` the sender, for `Outgoing` the
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
    #[serde(rename = "richtung")]
    pub direction: Direction,
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
    #[serde(rename = "quelle")]
    pub origin: String,
}

impl Request {
    /// The key of the other side - independent of the direction:
    /// their number, otherwise "k:<person>", otherwise "@handle".
    pub fn opposite(&self) -> String {
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

    pub fn is_incoming(&self) -> bool {
        self.direction == Direction::Incoming
    }
}

/// An entry of the block list.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Blocked {
    #[serde(default)]
    pub fvid: String,
    #[serde(default)]
    #[serde(rename = "seit")]
    pub since: u64,
    #[serde(default)]
    pub person: String,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub name: String,
}

impl Blocked {
    pub fn entry_key(&self) -> String {
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
    AlreadyBefriended,
    AlreadyRequested,
    /// Accept/decline without a request being present.
    NoRequest,
    NotFriend,
    Blocked,
    InvalidId,
    /// An @name only works with an account (the address book hangs on the account).
    AccountRequired,
    /// The relay declined or was not reachable.
    Netz(String),
    /// The relay does not know meeting invitations yet (older version).
    NoChannel,
    /// The friend has no (yet) FreeViewer PC an invitation could go to.
    NoDevice,
    /// A link with an end-to-end key must never go through the relay.
    LinkWithKey,
}

impl std::fmt::Display for FriendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            FriendError::SelfRequest => "Das ist die eigene FreeViewer-ID".to_string(),
            FriendError::AlreadyBefriended => "Ihr seid schon befreundet".to_string(),
            FriendError::AlreadyRequested => "Die Anfrage laeuft schon".to_string(),
            FriendError::NoRequest => "Von dieser ID liegt keine Anfrage vor".to_string(),
            FriendError::NotFriend => "Diese ID steht nicht in der Freundesliste".to_string(),
            FriendError::Blocked => "Diese ID steht auf der Blockliste".to_string(),
            FriendError::InvalidId => "Das ist keine gueltige FreeViewer-ID oder kein @Name".to_string(),
            FriendError::AccountRequired => "Fuer @Namen bitte zuerst mit dem Fleitec-Konto anmelden".to_string(),
            FriendError::Netz(m) => m.clone(),
            FriendError::NoChannel => "Der Relay kann noch keine Einladungen zustellen".to_string(),
            FriendError::NoDevice => "Dieser Freund hat noch keinen FreeViewer-PC".to_string(),
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
    /// Abgleich done: so viele Friends, so viele offene Anfragen.
    Synced { friends: usize, is_open: usize },
    /// Request has arrived at the relay.
    Requested(String),
    /// The other had already asked me - we are friends immediately now.
    ImmediatelyBefriended(String),
    Accepted(String),
    Rejected(String),
    Removed(String),
    /// Meeting invitation has arrived at the relay (and thus at the friend).
    InviteSent(String),
    /// The relay cannot deliver invitations: the link has to go to the friend by hand.
    InviteOnlyLink(String),
    /// A friend invited me to a meeting (name, title).
    InviteReceived(String, String),
    /// The relay declined (text is ready to display).
    Failed(String),
}

/// Digits only - the user types "497 628 420" or "497-628-420".
pub fn normalize(fvid: &str) -> String {
    fvid.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// Does this look like a FreeViewer ID? (9 or 10 digits)
pub fn ist_id(fvid: &str) -> bool {
    let d = normalize(fvid);
    (9..=10).contains(&d.len())
}

/// What the user types, cleaned: "@Anna " -> "@anna",
/// "k:<account>" stays, everything else -> digits only.
pub fn target_norm(s: &str) -> String {
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
    normalize(t)
}

/// Is this usable as a target? A FreeViewer ID, an @name or "k:<account>".
pub fn is_target(s: &str) -> bool {
    let z = target_norm(s);
    z.starts_with('@') || z.starts_with("k:") || ist_id(&z)
}

/// For messages: "497 628 420", "@anna" or "dieser Kontakt".
pub fn target_label(s: &str) -> String {
    let z = target_norm(s);
    if z.starts_with('@') {
        z
    } else if z.starts_with("k:") {
        "dieser Kontakt".to_string()
    } else {
        crate::partners::pretty_id(&z)
    }
}

fn now_ts() -> u64 {
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
pub struct Book {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    #[serde(rename = "freunde")]
    pub friends: Vec<Friend>,
    #[serde(default)]
    #[serde(rename = "anfragen")]
    pub request: Vec<Request>,
    #[serde(default)]
    #[serde(rename = "blockiert")]
    pub blocked: Vec<Blocked>,
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
    Book::load()
        .friends
        .iter()
        .any(|f| f.display_name().trim().to_lowercase() == name)
}

impl Book {
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
        let id = target_norm(fvid);
        self.friends.iter().find(|f| f.ist(&id))
    }

    pub fn is_friend(&self, fvid: &str) -> bool {
        self.friend(fvid).is_some()
    }

    pub fn is_blocked(&self, fvid: &str) -> bool {
        let id = target_norm(fvid);
        self.blocked.iter().any(|b| b.ist(&id))
    }

    /// What is the entry behind this key called? For messages.
    pub fn label_for(&self, k: &str) -> String {
        let id = target_norm(k);
        if let Some(f) = self.friends.iter().find(|f| f.ist(&id)) {
            return f.display_name();
        }
        if let Some(a) = self.request.iter().find(|a| a.ist(&id)) {
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

    pub fn incoming_requests(&self) -> Vec<Request> {
        self.request
            .iter()
            .filter(|a| a.direction == Direction::Incoming)
            .cloned()
            .collect()
    }

    pub fn outgoing(&self) -> Vec<Request> {
        self.request
            .iter()
            .filter(|a| a.direction == Direction::Outgoing)
            .cloned()
            .collect()
    }

    fn request_from(&self, fvid: &str, direction: Direction) -> Option<usize> {
        let id = target_norm(fvid);
        self.request
            .iter()
            .position(|a| a.ist(&id) && a.direction == direction)
    }

    // ------------------------------------------------------- state changes
    //
    // These steps run WITHOUT network and are the places where the
    // state may change. The relay makes the same checks once
    // more - they are here so that the UI can answer immediately
    // and does not first wait on the wire.

    /// Ich stelle eine Request.
    pub fn submit_request(&mut self, fvid: &str, name: &str) -> Result<Request, FriendError> {
        let id = target_norm(fvid);
        if !is_target(&id) {
            return Err(FriendError::InvalidId);
        }
        if !self.fvid.is_empty() && id == self.fvid {
            return Err(FriendError::SelfRequest);
        }
        if self.is_friend(&id) {
            return Err(FriendError::AlreadyBefriended);
        }
        if self.is_blocked(&id) {
            return Err(FriendError::Blocked);
        }
        if self.request_from(&id, Direction::Outgoing).is_some() {
            return Err(FriendError::AlreadyRequested);
        }
        // They already asked me: then both agree.
        if let Some(i) = self.request_from(&id, Direction::Incoming) {
            let a = self.request[i].clone();
            self.accept_request(&a.opposite())?;
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
            wann: now_ts(),
            direction: Direction::Outgoing,
            message: String::new(),
            online: false,
            person,
            handle,
            origin: String::new(),
        };
        self.request.push(a.clone());
        self.shorten();
        Ok(a)
    }

    /// A request came in from the relay. `true` if it was new.
    pub fn record_request(&mut self, mut a: Request) -> bool {
        a.von_fvid = normalize(&a.von_fvid);
        let k = a.opposite();
        if k.is_empty() || self.is_friend(&k) || self.is_blocked(&k) {
            return false;
        }
        if let Some(i) = self.request_from(&k, a.direction) {
            self.request[i] = a;
            return false;
        }
        self.request.push(a);
        self.shorten();
        true
    }

    /// I accept an INCOMING request - only with this does the
    /// friendship arise. Without a pending request this is not possible; a stranger
    /// can thus never silently become a friend.
    pub fn accept_request(&mut self, fvid: &str) -> Result<Friend, FriendError> {
        let id = target_norm(fvid);
        if let Some(f) = self.friend(&id) {
            return Ok(f.clone()); // zweiter Klick - kein FriendError
        }
        let i = self
            .request_from(&id, Direction::Incoming)
            .ok_or(FriendError::NoRequest)?;
        let a = self.request.remove(i);
        // an own request in the opposite direction is dropped too
        if let Some(j) = self.request_from(&id, Direction::Outgoing) {
            self.request.remove(j);
        }
        let f = Friend {
            fvid: a.von_fvid,
            name: a.name,
            since: now_ts(),
            online: a.online,
            seen: 0,
            person: a.person,
            handle: a.handle,
            devices: Vec::new(),
            origin: a.origin,
        };
        self.friends.push(f.clone());
        self.shorten();
        Ok(f)
    }

    /// Decline (incoming) or withdraw the own request
    /// (outgoing). Both only throw the request away - NO
    /// friendship arises.
    pub fn reject(&mut self, fvid: &str) -> Result<(), FriendError> {
        let id = target_norm(fvid);
        let before = self.request.len();
        self.request.retain(|a| !a.ist(&id));
        if self.request.len() == before {
            return Err(FriendError::NoRequest);
        }
        Ok(())
    }

    /// Dissolve a friendship. Applies to both sides - the relay cleans it up
    /// at the other end too.
    pub fn remove_entry(&mut self, fvid: &str) -> Result<(), FriendError> {
        let id = target_norm(fvid);
        let before = self.friends.len();
        self.friends.retain(|f| !f.ist(&id));
        if self.friends.len() == before {
            return Err(FriendError::NotFriend);
        }
        Ok(())
    }

    /// Put on the block list (or take off again). Whoever is blocked
    /// can no longer ask; open requests are dropped.
    pub fn block(&mut self, fvid: &str, an: bool) -> Result<(), FriendError> {
        let id = target_norm(fvid);
        if !is_target(&id) {
            return Err(FriendError::InvalidId);
        }
        if !self.fvid.is_empty() && id == self.fvid {
            return Err(FriendError::SelfRequest);
        }
        if an {
            if !self.is_blocked(&id) {
                let name = self.label_for(&id);
                let mut b = Blocked {
                    since: now_ts(),
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
                self.blocked.push(b);
            }
            self.request.retain(|a| !a.ist(&id));
        } else {
            self.blocked.retain(|b| !b.ist(&id));
        }
        Ok(())
    }

    /// The relay has spoken. Its version applies - it is the only one both
    /// sides know. Returns whether something changed.
    pub fn adopt(&mut self, stand: &Stand) -> bool {
        let mut neu = Book {
            version: 1,
            friends: stand.friends.clone(),
            request: stand
                .incoming
                .iter()
                .cloned()
                .map(|mut a| {
                    a.direction = Direction::Incoming;
                    a
                })
                .chain(stand.outgoing.iter().cloned().map(|mut a| {
                    a.direction = Direction::Outgoing;
                    a
                }))
                .collect(),
            blocked: stand.blocked.clone(),
            fvid: normalize(&stand.fvid),
            account: stand.account.clone(),
            stand: now_ts(),
        };
        for f in &mut neu.friends {
            f.fvid = normalize(&f.fvid);
            for g in &mut f.devices {
                g.fvid = normalize(&g.fvid);
            }
            f.devices.retain(|g| !g.fvid.is_empty());
        }
        for a in &mut neu.request {
            a.von_fvid = normalize(&a.von_fvid);
        }
        for b in &mut neu.blocked {
            b.fvid = normalize(&b.fvid);
        }
        // A friend from the address book may also stand there WITHOUT a PC - then
        // they are carried by their account (person) or their @name.
        neu.friends.retain(|f| !f.entry_key().is_empty());
        neu.request.retain(|a| !a.opposite().is_empty());
        neu.blocked.retain(|b| !b.entry_key().is_empty());
        neu.shorten();
        let other = neu.friends != self.friends
            || neu.request != self.request
            || neu.blocked != self.blocked
            || neu.account != self.account;
        *self = neu;
        other
    }

    fn shorten(&mut self) {
        if self.friends.len() > MAX_FRIENDS {
            self.friends.truncate(MAX_FRIENDS);
        }
        if self.request.len() > MAX_REQUEST {
            self.request.truncate(MAX_REQUEST);
        }
        self.version = 1;
    }
}

// ================================================================ Netzteil

/// The answer of the relay to `/fv/friends/stand` (and `/list_items`).
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
    #[serde(rename = "freunde")]
    pub friends: Vec<Friend>,
    #[serde(default)]
    #[serde(rename = "eingehend")]
    pub incoming: Vec<Request>,
    #[serde(default)]
    #[serde(rename = "ausgehend")]
    pub outgoing: Vec<Request>,
    #[serde(default)]
    #[serde(rename = "blockiert")]
    pub blocked: Vec<Blocked>,
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
fn ruf(relay_url: &str, was: &str, json_body: serde_json::Value) -> Result<serde_json::Value, FriendError> {
    let dest = url(relay_url, was);
    let body = json_body.to_string();
    // Also read the body on 4xx/5xx: only then do the German sentences of the
    // relay arrive (and an old relay without /einladen is recognisable as 404 "unbekannt").
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let (status, text) = match agent
        .post(&dest)
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
        return Err(FriendError::NoChannel);
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
fn credential(secret: &str, token: &str, name: &str) -> serde_json::Value {
    let mut v = serde_json::json!({ "secret": secret, "name": name });
    if !token.is_empty() {
        v["token"] = serde_json::Value::String(token.to_string());
    }
    v
}

fn mit(mut v: serde_json::Value, json_key: &str, wert: &str) -> serde_json::Value {
    v[json_key] = serde_json::Value::String(wert.to_string());
    v
}

/// Fetch the complete state (one round trip for friends + requests + block list).
pub fn hole_stand(relay: &str, secret: &str, token: &str, name: &str) -> Result<Stand, FriendError> {
    let v = ruf(relay, "stand", credential(secret, token, name))?;
    serde_json::from_value(v).map_err(|e| FriendError::Netz(format!("Antwort unverstaendlich: {}", e)))
}

pub fn send_request(
    relay: &str,
    secret: &str,
    token: &str,
    name: &str,
    an: &str,
    message: &str,
) -> Result<bool, FriendError> {
    let mut v = mit(credential(secret, token, name), "an", &target_norm(an));
    v["nachricht"] = serde_json::Value::String(message.to_string());
    let a = ruf(relay, "anfragen", v)?;
    // true = the other had already asked, we are friends immediately
    Ok(a.get("angenommen").and_then(|x| x.as_bool()).unwrap_or(false))
}

pub fn send_accept(relay: &str, secret: &str, token: &str, name: &str, von: &str) -> Result<(), FriendError> {
    ruf(relay, "annehmen", mit(credential(secret, token, name), "von", &target_norm(von))).map(|_| ())
}

pub fn send_reject(relay: &str, secret: &str, token: &str, name: &str, wen: &str) -> Result<(), FriendError> {
    ruf(relay, "ablehnen", mit(credential(secret, token, name), "von", &target_norm(wen))).map(|_| ())
}

pub fn send_remove(relay: &str, secret: &str, token: &str, name: &str, wen: &str) -> Result<(), FriendError> {
    ruf(relay, "entfernen", mit(credential(secret, token, name), "fvid", &target_norm(wen))).map(|_| ())
}

pub fn send_block(
    relay: &str,
    secret: &str,
    token: &str,
    name: &str,
    wen: &str,
    an: bool,
) -> Result<(), FriendError> {
    let mut v = mit(credential(secret, token, name), "fvid", &target_norm(wen));
    v["an"] = serde_json::Value::Bool(an);
    ruf(relay, "blockieren", v).map(|_| ())
}

// ============================================ Meeting invitation to friends (r32)
//
// Flow: button "Meeting" -> create meeting -> invitation (only the https address)
// goes via the relay into the friend's mailbox (`/einladen`), the friend
// fetches it on the next sync (`/invites`) and clears it with
// `/einladung-weg`. Everything is additive: a relay without these paths
// answers 404 "unbekannt" -> `FriendError::NoChannel`, then the old way
// stays (link in the clipboard). The end-to-end key is in the
// fragment (#k=) and NEVER goes through the relay - such links are rejected here already
// before sending (and by the relay once more).

/// A meeting invitation from a friend that is in the mailbox.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Invite {
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
    #[serde(rename = "titel")]
    pub heading: String,
    /// https-Adresse des Meetings.
    #[serde(default)]
    pub link: String,
    /// Unix Sekunden.
    #[serde(default)]
    pub wann: u64,
    #[serde(default)]
    pub online: bool,
}

impl Invite {
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
struct InvitesReply {
    #[serde(default)]
    #[serde(rename = "einladungen")]
    invites: Vec<Invite>,
}

/// To which PC of the friend the invitation goes: a reachable one first,
/// otherwise the first. Empty = the friend has no FreeViewer PC.
pub fn invite_target(fr: &Friend) -> String {
    let g = fr
        .devices
        .iter()
        .find(|g| g.online && ist_id(&g.fvid))
        .or_else(|| fr.devices.iter().find(|g| ist_id(&g.fvid)));
    match g {
        Some(g) => normalize(&g.fvid),
        None if ist_id(&fr.fvid) => normalize(&fr.fvid),
        None => String::new(),
    }
}

/// The link that may go to the relay: never with a fragment (the key is in it).
pub fn link_for_relay(link: &str) -> Option<String> {
    let l = link.trim();
    if l.is_empty() || l.contains('#') {
        return None;
    }
    Some(l.to_string())
}

/// May a received link be opened? Only the meeting page of this
/// brand (`base`, e.g. https://meet.fleitec.com) - a friend (or whoever poses
/// as one) must not be able to open an arbitrary address in the browser.
pub fn invite_link_ok(link: &str, base: &str) -> bool {
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

/// Postfach abholen. `FriendError::NoChannel` = aelterer Relay.
pub fn fetch_invites(
    relay: &str,
    secret: &str,
    token: &str,
    name: &str,
) -> Result<Vec<Invite>, FriendError> {
    let v = ruf(relay, "einladungen", credential(secret, token, name))?;
    let a: InvitesReply = serde_json::from_value(v)
        .map_err(|e| FriendError::Netz(format!("Antwort unverstaendlich: {}", e)))?;
    Ok(a.invites)
}

/// Invitation to a PC of the friend. `true` = delivered.
pub fn send_invite(
    relay: &str,
    secret: &str,
    token: &str,
    name: &str,
    an: &str,
    heading: &str,
    link: &str,
) -> Result<bool, FriendError> {
    let mut v = mit(credential(secret, token, name), "an", &normalize(an));
    v["titel"] = serde_json::Value::String(heading.to_string());
    v["link"] = serde_json::Value::String(link.to_string());
    let a = ruf(relay, "einladen", v)?;
    Ok(a.get("zugestellt").and_then(|x| x.as_bool()).unwrap_or(true))
}

/// Einladung(en) im Postfach abraeumen.
pub fn send_invite_away(
    relay: &str,
    secret: &str,
    token: &str,
    name: &str,
    ids: &[String],
) -> Result<(), FriendError> {
    let mut v = credential(secret, token, name);
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
/// let friends = friends::Dienst::from_device(shared.relay_url.clone());
/// friends.launch();
/// // in every frame of the UI:
/// if let Some(m) = friends.notice() { /* show briefly */ }
/// for f in friends.liste() { /* draw row, button "Meeting" */ }
/// ```
pub struct Service {
    relay: String,
    secret: String,
    name: Mutex<String>,
    token: Mutex<String>,
    book: Mutex<Book>,
    /// Mailbox for the UI. As an Arc, so that a background thread can
    /// write into it without holding on to the whole service.
    mailbox: Arc<Mutex<Vec<Notice>>>,
    started: AtomicBool,
    busy: AtomicBool,
    /// "Please sync soon" - every step sets it, the thread reads it.
    immediately: Arc<AtomicBool>,
    last: AtomicU64,
    /// Letzter Abgleich hat geklappt?
    connected: AtomicBool,
    /// Received meeting invitations (cache, not on disk).
    invites: Mutex<Vec<Invite>>,
    /// Invitation IDs already reported - each is reported only ONCE.
    invite_reported: Mutex<std::collections::HashSet<String>>,
    /// Until when (unix s) invitations are not asked for, because the relay does not know them.
    invite_pause_to: AtomicU64,
}

impl Service {
    /// With explicit secret and name (for tests and special cases).
    pub fn neu(relay_url: String, secret: String, name: String) -> Arc<Self> {
        Arc::new(Self {
            relay: relay_url,
            secret,
            name: Mutex::new(name),
            token: Mutex::new(String::new()),
            book: Mutex::new(Book::load()),
            mailbox: Arc::new(Mutex::new(Vec::new())),
            started: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            immediately: Arc::new(AtomicBool::new(false)),
            last: AtomicU64::new(0),
            connected: AtomicBool::new(false),
            invites: Mutex::new(Vec::new()),
            invite_reported: Mutex::new(std::collections::HashSet::new()),
            invite_pause_to: AtomicU64::new(0),
        })
    }

    /// The normal case: secret and name come from this machine.
    pub fn from_device(relay_url: String) -> Arc<Self> {
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
            self.immediately.store(true, Ordering::SeqCst);
        }
    }

    pub fn names_set(&self, name: &str) {
        *self.name.lock_ok() = crate::presence::clean(name);
    }

    pub fn signed_in(&self) -> bool {
        !self.token.lock_ok().is_empty()
    }

    /// Did the last sync succeed?
    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// The background thread. May be called several times.
    pub fn launch(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let ich = self.clone();
        std::thread::spawn(move || loop {
            let due = now_ts().saturating_sub(ich.last.load(Ordering::Relaxed))
                >= SYNC_SECONDS;
            if ich.immediately.swap(false, Ordering::SeqCst) || due {
                ich.reconcile();
            }
            std::thread::sleep(Duration::from_millis(500));
        });
    }

    // ------------------------------------------------------- What the GUI fetches

    /// The friend list - online first. Comes from the cache,
    /// so it never waits for the network.
    pub fn list_items(&self) -> Vec<Friend> {
        self.book.lock_ok().sortiert()
    }

    /// Alle offenen Anfragen, beide Richtungen.
    pub fn open_request(&self) -> Vec<Request> {
        self.book.lock_ok().request.clone()
    }

    /// Only those I have to answer.
    pub fn incoming_requests(&self) -> Vec<Request> {
        self.book.lock_ok().incoming_requests()
    }

    pub fn outgoing(&self) -> Vec<Request> {
        self.book.lock_ok().outgoing()
    }

    pub fn blocked(&self) -> Vec<Blocked> {
        self.book.lock_ok().blocked.clone()
    }

    pub fn is_friend(&self, fvid: &str) -> bool {
        self.book.lock_ok().is_friend(fvid)
    }

    /// Reachable friend? (Offer one-click meeting/remote support only then.)
    pub fn reachable(&self, fvid: &str) -> bool {
        self.book
            .lock()
            .unwrap()
            .friend(fvid)
            .map(|f| f.online)
            .unwrap_or(false)
    }

    /// Eine Abschrift des ganzen Buches.
    pub fn book(&self) -> Book {
        self.book.lock_ok().clone()
    }

    /// Next message from the mailbox (and out of it).
    pub fn notice(&self) -> Option<Notice> {
        let mut p = self.mailbox.lock_ok();
        if p.is_empty() {
            None
        } else {
            Some(p.remove(0))
        }
    }

    fn melde(&self, m: Notice) {
        let mut p = self.mailbox.lock_ok();
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
    pub fn request(&self, fvid: &str) -> Result<(), FriendError> {
        self.request_with(fvid, "")
    }

    /// Like `request`, but with a short greeting.
    pub fn request_with(&self, fvid: &str, message: &str) -> Result<(), FriendError> {
        let id = target_norm(fvid);
        // @name and account ID live in the address book - that hangs on the account.
        if is_target(&id) && !ist_id(&id) && !self.signed_in() {
            return Err(FriendError::AccountRequired);
        }
        {
            let mut b = self.book.lock_ok();
            let name = self.name.lock_ok().clone();
            b.submit_request(&id, &name)?;
            b.save();
        }
        let (relay, secret, token, name) = self.access();
        let message = message.to_string();
        let ich = self.clone_mailbox();
        std::thread::spawn(move || {
            match send_request(&relay, &secret, &token, &name, &id, &message) {
                Ok(true) => ich.melde(Notice::ImmediatelyBefriended(target_label(&id))),
                Ok(false) => ich.melde(Notice::Requested(target_label(&id))),
                Err(e) => ich.melde(Notice::Failed(e.to_string())),
            }
            ich.immediately.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Accept an incoming request - only with this are both friends.
    pub fn accept_request(&self, fvid: &str) -> Result<(), FriendError> {
        let id = target_norm(fvid);
        let wer;
        {
            let mut b = self.book.lock_ok();
            wer = b.label_for(&id);
            b.accept_request(&id)?;
            b.save();
        }
        let (relay, secret, token, name) = self.access();
        let ich = self.clone_mailbox();
        std::thread::spawn(move || {
            match send_accept(&relay, &secret, &token, &name, &id) {
                Ok(()) => ich.melde(Notice::Accepted(wer.clone())),
                Err(e) => ich.melde(Notice::Failed(e.to_string())),
            }
            ich.immediately.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Decline or withdraw the own request.
    pub fn reject(&self, fvid: &str) -> Result<(), FriendError> {
        let id = target_norm(fvid);
        let wer;
        {
            let mut b = self.book.lock_ok();
            wer = b.label_for(&id);
            b.reject(&id)?;
            b.save();
        }
        let (relay, secret, token, name) = self.access();
        let ich = self.clone_mailbox();
        std::thread::spawn(move || {
            match send_reject(&relay, &secret, &token, &name, &id) {
                Ok(()) => ich.melde(Notice::Rejected(wer.clone())),
                Err(e) => ich.melde(Notice::Failed(e.to_string())),
            }
            ich.immediately.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Freundschaft aufloesen (bei beiden).
    pub fn remove_entry(&self, fvid: &str) -> Result<(), FriendError> {
        let id = target_norm(fvid);
        let wer;
        {
            let mut b = self.book.lock_ok();
            wer = b.label_for(&id);
            b.remove_entry(&id)?;
            b.save();
        }
        let (relay, secret, token, name) = self.access();
        let ich = self.clone_mailbox();
        std::thread::spawn(move || {
            match send_remove(&relay, &secret, &token, &name, &id) {
                Ok(()) => ich.melde(Notice::Removed(wer.clone())),
                Err(e) => ich.melde(Notice::Failed(e.to_string())),
            }
            ich.immediately.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    /// Block (`an = true`) or unblock again.
    pub fn block(&self, fvid: &str, an: bool) -> Result<(), FriendError> {
        let id = target_norm(fvid);
        {
            let mut b = self.book.lock_ok();
            b.block(&id, an)?;
            b.save();
        }
        let (relay, secret, token, name) = self.access();
        let ich = self.clone_mailbox();
        std::thread::spawn(move || {
            if let Err(e) = send_block(&relay, &secret, &token, &name, &id, an) {
                ich.melde(Notice::Failed(e.to_string()));
            }
            ich.immediately.store(true, Ordering::SeqCst);
        });
        Ok(())
    }

    // ------------------------------------------------- Meeting-Einladungen (r32)

    /// The invitations currently in the mailbox (newest first).
    pub fn invites(&self) -> Vec<Invite> {
        self.invites.lock_ok().clone()
    }

    /// Send a created meeting to a friend. The link must not carry a
    /// key. Permissible immediately or not; the delivery runs in the
    /// background and reports as `InviteSent` or `InviteOnlyLink`
    /// (relay without mailbox: then only the clipboard helps).
    pub fn meeting_invite(&self, fr: &Friend, heading: &str, link: &str) -> Result<(), FriendError> {
        let an = invite_target(fr);
        if an.is_empty() {
            return Err(FriendError::NoDevice);
        }
        let link = link_for_relay(link).ok_or(FriendError::LinkWithKey)?;
        let heading = heading.trim().to_string();
        let wer = fr.display_name();
        let (relay, secret, token, name) = self.access();
        let ich = self.clone_mailbox();
        std::thread::spawn(move || {
            match send_invite(&relay, &secret, &token, &name, &an, &heading, &link) {
                Ok(_) => ich.melde(Notice::InviteSent(wer)),
                Err(FriendError::NoChannel) => ich.melde(Notice::InviteOnlyLink(wer)),
                Err(e) => ich.melde(Notice::Failed(e.to_string())),
            }
        });
        Ok(())
    }

    /// Tick off an invitation (accepted or discarded): it disappears
    /// here immediately, at the relay in the background.
    pub fn invite_done(&self, id: &str) {
        self.invites.lock_ok().retain(|e| e.id != id);
        let (relay, secret, token, name) = self.access();
        let ids = vec![id.to_string()];
        std::thread::spawn(move || {
            let _ = send_invite_away(&relay, &secret, &token, &name, &ids);
        });
    }

    /// Take over a new mailbox state; reports every NEW invitation exactly once.
    /// Returns how many were new.
    pub fn invites_adopt(&self, mut list_items: Vec<Invite>) -> usize {
        list_items.retain(|e| !e.id.is_empty() && !e.link.is_empty());
        if list_items.len() > 50 {
            list_items.drain(50..);
        }
        let mut neu = Vec::new();
        {
            let mut reported = self.invite_reported.lock_ok();
            for e in &list_items {
                if reported.insert(e.id.clone()) {
                    neu.push(e.clone());
                }
            }
            if reported.len() > 500 {
                reported.clear();
                reported.extend(list_items.iter().map(|e| e.id.clone()));
            }
        }
        *self.invites.lock_ok() = list_items;
        for e in &neu {
            self.melde(Notice::InviteReceived(e.von(), e.heading.clone()));
        }
        neu.len()
    }

    fn invites_fetch(&self) {
        if now_ts() < self.invite_pause_to.load(Ordering::Relaxed) {
            return;
        }
        let (relay, secret, token, name) = self.access();
        match fetch_invites(&relay, &secret, &token, &name) {
            Ok(list_items) => {
                self.invites_adopt(list_items);
            }
            // Older relay: stop asking for a while (it can be updated, after all).
            Err(FriendError::NoChannel) => self
                .invite_pause_to
                .store(now_ts() + INVITE_PAUSE_SECONDS, Ordering::Relaxed),
            Err(_) => {}
        }
    }

    /// Sofortigen Abgleich anstossen (kehrt sofort go_back).
    pub fn synchronisieren(&self) {
        self.immediately.store(true, Ordering::SeqCst);
    }

    /// Sync NOW and here - blocks. Only for the background thread and
    /// headless operation; the UI uses `synchronisieren()`.
    pub fn sync_blocking(&self) -> Result<Stand, FriendError> {
        let (relay, secret, token, name) = self.access();
        let stand = hole_stand(&relay, &secret, &token, &name)?;
        let mut b = self.book.lock_ok();
        if b.adopt(&stand) {
            b.save();
        } else {
            b.stand = now_ts();
        }
        Ok(stand)
    }

    fn reconcile(&self) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let outcome = self.sync_blocking();
        self.last.store(now_ts(), Ordering::Relaxed);
        match outcome {
            Ok(_) => {
                self.connected.store(true, Ordering::Relaxed);
                {
                    let b = self.book.lock_ok();
                    self.melde(Notice::Synced {
                        friends: b.friends.len(),
                        is_open: b.request.len(),
                    });
                }
                self.invites_fetch();
            }
            Err(e) => {
                self.connected.store(false, Ordering::Relaxed);
                self.melde(Notice::Failed(e.to_string()));
            }
        }
        self.busy.store(false, Ordering::SeqCst);
    }

    fn access(&self) -> (String, String, String, String) {
        (
            self.relay.clone(),
            self.secret.clone(),
            self.token.lock_ok().clone(),
            self.name.lock_ok().clone(),
        )
    }

    /// A small handle with which a background thread can report and trigger the
    /// next sync - it needs nothing more.
    fn clone_mailbox(&self) -> Mailbox {
        Mailbox {
            mailbox: Arc::clone(&self.mailbox),
            immediately: Arc::clone(&self.immediately),
        }
    }
}

/// A small handle for background threads.
struct Mailbox {
    mailbox: Arc<Mutex<Vec<Notice>>>,
    immediately: Arc<AtomicBool>,
}

impl Mailbox {
    fn melde(&self, m: Notice) {
        let mut p = self.mailbox.lock_ok();
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

    fn book_with_me(my: &str) -> Book {
        Book {
            version: 1,
            fvid: my.to_string(),
            ..Default::default()
        }
    }

    fn incoming(von: &str, name: &str) -> Request {
        Request {
            von_fvid: von.to_string(),
            name: name.to_string(),
            wann: 1_700_000_000,
            direction: Direction::Incoming,
            message: String::new(),
            online: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_test_never_writes_to_the_real_config() {
        own_dir("isoliert");
        let b = book_with_me("111111111");
        b.save();
        assert!(Book::path().starts_with(std::env::temp_dir()));
        if let Some(m) = crate::ident::machine_config_dir() {
            assert!(!Book::path().starts_with(m));
        }
        assert!(!Book::path().starts_with(crate::ident::user_config_dir()));
    }

    #[test]
    fn save_and_load_keeps_everything() {
        own_dir("speichern");
        let mut b = book_with_me("111111111");
        b.record_request(incoming("222222222", "Laptop"));
        b.accept_request("222222222").unwrap();
        b.record_request(incoming("333333333", "Handy"));
        b.block("444444444", true).unwrap();
        b.save();

        let again = Book::load();
        assert_eq!(again.friends.len(), 1);
        assert_eq!(again.friends[0].fvid, "222222222");
        assert_eq!(again.friends[0].name, "Laptop");
        assert_eq!(again.incoming_requests().len(), 1);
        assert_eq!(again.incoming_requests()[0].von_fvid, "333333333");
        assert!(again.is_blocked("444444444"));
        assert_eq!(again.fvid, "111111111");
    }

    #[test]
    fn empty_list_is_valid_and_fetches_nothing() {
        own_dir("leer");
        let mut b = book_with_me("111111111");
        b.record_request(incoming("222222222", "Weg"));
        b.accept_request("222222222").unwrap();
        b.save();
        // second write creates the backup copy
        b.remove_entry("222222222").unwrap();
        b.save();
        let again = Book::load();
        assert_eq!(again.friends.len(), 0, "entfernte Freunde duerfen nicht zurueckkommen");
    }

    #[test]
    fn broken_file_falls_back_to_backup() {
        own_dir("kaputt");
        let mut b = book_with_me("111111111");
        b.record_request(incoming("222222222", "Laptop"));
        b.accept_request("222222222").unwrap();
        b.save();
        b.save(); // jetzt existiert die Sicherungskopie
        std::fs::write(Book::path(), b"{ kaputt").unwrap();
        let again = Book::load();
        assert_eq!(again.friends.len(), 1);
        assert_eq!(again.friends[0].fvid, "222222222");
    }

    #[test]
    fn request_to_self_is_rejected() {
        let mut b = book_with_me("111111111");
        assert_eq!(b.submit_request("111111111", "Ich"), Err(FriendError::SelfRequest));
        // also typed with spaces
        assert_eq!(b.submit_request("111 111 111", "Ich"), Err(FriendError::SelfRequest));
        assert!(b.request.is_empty());
    }

    #[test]
    fn nonsense_id_is_refused() {
        let mut b = book_with_me("111111111");
        assert_eq!(b.submit_request("123", "Kurz"), Err(FriendError::InvalidId));
        assert_eq!(b.submit_request("", "Nichts"), Err(FriendError::InvalidId));
        assert_eq!(b.submit_request("abc", "Buchstaben"), Err(FriendError::InvalidId));
    }

    #[test]
    fn same_request_twice_is_rejected() {
        let mut b = book_with_me("111111111");
        b.submit_request("222222222", "Ich").unwrap();
        assert_eq!(b.submit_request("222222222", "Ich"), Err(FriendError::AlreadyRequested));
        // andere Schreibweise, gleiche ID
        assert_eq!(b.submit_request("222 222 222", "Ich"), Err(FriendError::AlreadyRequested));
        assert_eq!(b.outgoing().len(), 1);
    }

    #[test]
    fn request_to_a_friend_is_rejected() {
        let mut b = book_with_me("111111111");
        b.record_request(incoming("222222222", "Laptop"));
        b.accept_request("222222222").unwrap();
        assert_eq!(b.submit_request("222222222", "Ich"), Err(FriendError::AlreadyBefriended));
    }

    #[test]
    fn nobody_becomes_friend_silently_without_request() {
        let mut b = book_with_me("111111111");
        // request from a complete stranger is not present
        assert_eq!(b.accept_request("999999999"), Err(FriendError::NoRequest));
        assert!(b.friends.is_empty());
        // an OUTGOING request does not make me a friend - the other
        // has to agree
        b.submit_request("222222222", "Ich").unwrap();
        assert_eq!(b.accept_request("222222222"), Err(FriendError::NoRequest));
        assert!(b.friends.is_empty());
    }

    #[test]
    fn accept_turns_request_into_friend() {
        let mut b = book_with_me("111111111");
        b.record_request(incoming("222222222", "Laptop"));
        assert_eq!(b.incoming_requests().len(), 1);
        let f = b.accept_request("222222222").unwrap();
        assert_eq!(f.fvid, "222222222");
        assert_eq!(f.name, "Laptop");
        assert!(f.since > 0);
        assert!(b.is_friend("222222222"));
        assert!(b.request.is_empty(), "die Anfrage muss aus der Liste raus");
        // second click is not an error
        assert!(b.accept_request("222222222").is_ok());
        assert_eq!(b.friends.len(), 1);
    }

    #[test]
    fn mutual_requests_make_friends_immediately() {
        let mut b = book_with_me("111111111");
        b.record_request(incoming("222222222", "Laptop"));
        b.submit_request("222222222", "Ich").unwrap();
        assert!(b.is_friend("222222222"));
        assert!(b.request.is_empty());
    }

    #[test]
    fn reject_drops_request_without_friendship() {
        let mut b = book_with_me("111111111");
        b.record_request(incoming("222222222", "Laptop"));
        b.reject("222222222").unwrap();
        assert!(b.request.is_empty());
        assert!(!b.is_friend("222222222"));
        assert_eq!(b.reject("222222222"), Err(FriendError::NoRequest));
    }

    #[test]
    fn own_request_can_be_withdrawn() {
        let mut b = book_with_me("111111111");
        b.submit_request("222222222", "Ich").unwrap();
        assert_eq!(b.outgoing().len(), 1);
        b.reject("222222222").unwrap();
        assert!(b.request.is_empty());
        assert!(!b.is_friend("222222222"));
    }

    #[test]
    fn remove_takes_friend_off_the_list() {
        let mut b = book_with_me("111111111");
        b.record_request(incoming("222222222", "Laptop"));
        b.accept_request("222222222").unwrap();
        b.remove_entry("222222222").unwrap();
        assert!(!b.is_friend("222222222"));
        assert_eq!(b.remove_entry("222222222"), Err(FriendError::NotFriend));
    }

    #[test]
    fn block_drops_open_requests_and_locks_new_ones() {
        let mut b = book_with_me("111111111");
        b.record_request(incoming("222222222", "Nervt"));
        b.block("222222222", true).unwrap();
        assert!(b.request.is_empty());
        assert!(b.is_blocked("222222222"));
        assert_eq!(b.submit_request("222222222", "Ich"), Err(FriendError::Blocked));
        // eine hereinkommende Request prallt ab
        assert!(!b.record_request(incoming("222222222", "Nervt")));
        assert!(b.request.is_empty());
        // freigeben
        b.block("222222222", false).unwrap();
        assert!(!b.is_blocked("222222222"));
        assert!(b.record_request(incoming("222222222", "Nervt")));
    }

    #[test]
    fn incoming_and_outgoing_stay_separate() {
        let mut b = book_with_me("111111111");
        b.record_request(incoming("222222222", "A"));
        b.submit_request("333333333", "Ich").unwrap();
        assert_eq!(b.incoming_requests().len(), 1);
        assert_eq!(b.outgoing().len(), 1);
        assert_eq!(b.incoming_requests()[0].opposite(), "222222222");
        assert!(b.incoming_requests()[0].is_incoming());
        assert_eq!(b.outgoing()[0].opposite(), "333333333");
        assert!(!b.outgoing()[0].is_incoming());
    }

    #[test]
    fn relay_state_wins() {
        let mut b = book_with_me("111111111");
        b.submit_request("999999999", "Ich").unwrap(); // nur lokal geraten
        let stand = Stand {
            ok: true,
            fvid: "111111111".into(),
            account: "justin".into(),
            friends: vec![Friend {
                fvid: "222222222".into(),
                name: "Laptop".into(),
                since: 1_700_000_000,
                online: true,
                seen: 1_700_000_000_000,
                ..Default::default()
            }],
            incoming: vec![incoming("333333333", "Handy")],
            outgoing: vec![],
            blocked: vec![Blocked { fvid: "444444444".into(), since: 1, ..Default::default() }],
            error: String::new(),
        };
        assert!(b.adopt(&stand));
        assert_eq!(b.friends.len(), 1);
        assert_eq!(b.friends[0].fvid, "222222222");
        assert_eq!(b.incoming_requests().len(), 1);
        assert_eq!(b.outgoing().len(), 0, "die lokale Vermutung faellt weg");
        assert_eq!(b.account, "justin");
        assert!(b.is_blocked("444444444"));
        // derselbe Stand nochmal aendert nichts
        assert!(!b.adopt(&stand));
    }

    #[test]
    fn relay_may_set_directions() {
        let mut b = book_with_me("111111111");
        let stand = Stand {
            ok: true,
            fvid: "111111111".into(),
            // the relay sends "richtung" in both lists, deliberately
            // the wrong way round here - the assignment by list must win
            incoming: vec![Request {
                von_fvid: "222222222".into(),
                direction: Direction::Outgoing,
                ..Default::default()
            }],
            outgoing: vec![Request {
                von_fvid: "333333333".into(),
                direction: Direction::Incoming,
                ..Default::default()
            }],
            ..Default::default()
        };
        b.adopt(&stand);
        assert_eq!(b.incoming_requests()[0].von_fvid, "222222222");
        assert_eq!(b.outgoing()[0].von_fvid, "333333333");
    }

    #[test]
    fn list_puts_online_first() {
        let mut b = book_with_me("111111111");
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
    fn address_comes_from_relay() {
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
    fn ids_are_cut_to_digits() {
        assert_eq!(normalize("497 628 420"), "497628420");
        assert_eq!(normalize("4-976-284-201"), "4976284201");
        assert_eq!(normalize("abc"), "");
        assert!(ist_id("497 628 420"));
        assert!(ist_id("1234567890"));
        assert!(!ist_id("12345"));
        assert!(!ist_id("12345678901"));
    }

    #[test]
    fn direction_is_written_like_in_relay() {
        let a = incoming("222222222", "A");
        let j = serde_json::to_string(&a).unwrap();
        assert!(j.contains("\"richtung\":\"eingehend\""), "{}", j);
        assert!(j.contains("\"fvid\":\"222222222\""), "{}", j);
        // and back again - exactly how the relay sends it
        let roh = r#"{"fvid":"333333333","name":"Handy","wann":5,"richtung":"ausgehend","nachricht":"hi","online":true}"#;
        let b: Request = serde_json::from_str(roh).unwrap();
        assert_eq!(b.von_fvid, "333333333");
        assert_eq!(b.direction, Direction::Outgoing);
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
        assert_eq!(s.incoming.len(), 1);
        assert_eq!(s.incoming[0].von_fvid, "2130270636");
    }

    #[test]
    fn label_falls_back_to_id() {
        let f = Friend { fvid: "497628420".into(), ..Default::default() };
        assert_eq!(f.display_name(), "497 628 420");
        let g = Friend { fvid: "497628420".into(), name: "Buero-PC".into(), ..Default::default() };
        assert_eq!(g.display_name(), "Buero-PC");
        assert_eq!(
            Friend { online: true, ..Default::default() }.lately(),
            "online"
        );
    }

    #[test]
    fn names_stay_harmless() {
        let mut b = book_with_me("111111111");
        let a = b.submit_request("222222222", "  Pa\"tis {Laptop}  ").unwrap();
        assert_eq!(a.name, "Patis Laptop");
    }

    // ------------------------------------------------ r7: ein Adressbuch

    /// This is exactly how the relay answers for an account since r7 (buch.js).
    const BOOK_STATE: &str = r#"{"ok":true,"fvid":"111111111","konto":"anna","handle":"anna","person":"u_ANNA","buch":true,
        "freunde":[
          {"fvid":"222222222","name":"Bert","seit":1790000000,"online":true,"gesehen":0,"person":"u_BERT","handle":"bert",
           "geraete":[{"fvid":"222222222","label":"Bert-PC","online":true,"gesehen":0},{"fvid":"333333333","label":"Bert-Laptop","online":false,"gesehen":5}],"quelle":"buch"},
          {"fvid":"","name":"Carla","seit":1790000001,"online":false,"gesehen":0,"person":"u_CARLA","handle":"carla","geraete":[],"quelle":"buch"},
          {"fvid":"444444444","name":"Dora-PC","seit":1790000002,"online":false,"gesehen":0,"quelle":"fv"}],
        "eingehend":[{"fvid":"","name":"Emil","wann":1790000003,"richtung":"eingehend","nachricht":"","online":false,"person":"u_EMIL","handle":"emil","quelle":"buch"}],
        "ausgehend":[{"fvid":"","name":"","wann":1790000004,"richtung":"ausgehend","nachricht":"","online":false,"person":"u_FRIDA","handle":"frida","quelle":"buch"}],
        "blockiert":[{"fvid":"","seit":1790000005,"person":"u_GUSTAV","handle":"gustav","name":"Gustav","quelle":"buch"}]}"#;

    fn book_from_the_address_book() -> Book {
        let s: Stand = serde_json::from_str(BOOK_STATE).unwrap();
        let mut b = book_with_me("111111111");
        assert!(b.adopt(&s));
        b
    }

    #[test]
    fn address_book_state_is_adopted_whole() {
        let b = book_from_the_address_book();
        assert_eq!(b.account, "anna");
        // Carla hat keinen PC - frueher waere sie still weggefallen
        assert_eq!(b.friends.len(), 3);
        let carla = b.friend("k:u_CARLA").expect("Carla fehlt");
        assert!(!carla.hat_pc());
        assert!(!carla.reachable());
        assert_eq!(carla.entry_key(), "k:u_CARLA");
        // requests without a PC number stay, via the account
        assert_eq!(b.incoming_requests().len(), 1);
        assert_eq!(b.incoming_requests()[0].opposite(), "k:u_EMIL");
        assert_eq!(b.outgoing()[0].display_name(), "@frida");
        assert!(b.is_blocked("@gustav"));
        assert!(b.is_blocked("k:u_GUSTAV"));
    }

    #[test]
    fn friend_is_found_by_any_pc_and_name() {
        let b = book_from_the_address_book();
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
        let mut b = book_from_the_address_book();
        let f = b.accept_request("k:u_EMIL").unwrap();
        assert_eq!(f.person, "u_EMIL");
        assert_eq!(f.handle, "emil");
        assert!(b.incoming_requests().is_empty());
        assert!(b.is_friend("@emil"));
        assert_eq!(b.accept_request("@emil").unwrap().person, "u_EMIL"); // zweiter Klick
    }

    #[test]
    fn remove_reject_block_via_account() {
        let mut b = book_from_the_address_book();
        b.remove_entry("k:u_CARLA").unwrap();
        assert!(!b.is_friend("k:u_CARLA"));
        assert_eq!(b.remove_entry("k:u_CARLA"), Err(FriendError::NotFriend));
        b.reject("@frida").unwrap();
        assert!(b.outgoing().is_empty());
        b.block("k:u_EMIL", true).unwrap();
        assert!(b.incoming_requests().is_empty());
        assert!(b.is_blocked("k:u_EMIL"));
        b.block("k:u_EMIL", false).unwrap();
        assert!(!b.is_blocked("k:u_EMIL"));
        b.block("@gustav", false).unwrap();
        assert!(b.blocked.is_empty());
    }

    #[test]
    fn request_by_name_is_stored_as_name() {
        let mut b = book_with_me("111111111");
        let a = b.submit_request("@Zora", "").unwrap();
        assert_eq!(a.handle, "zora");
        assert_eq!(a.von_fvid, "");
        assert_eq!(a.opposite(), "@zora");
        assert_eq!(b.submit_request("@zora", ""), Err(FriendError::AlreadyRequested));
        assert_eq!(b.submit_request("@", ""), Err(FriendError::InvalidId));
        assert_eq!(b.submit_request("hallo", ""), Err(FriendError::InvalidId));
    }

    #[test]
    fn targets_are_sanitized() {
        assert_eq!(target_norm(" @Anna.B "), "@anna.b");
        assert_eq!(target_norm("@a b<c>"), "@abc");
        assert_eq!(target_norm("k:u_ANNA"), "k:u_ANNA");
        assert_eq!(target_norm("k:u/../x"), "k:ux");
        assert_eq!(target_norm("497 628 420"), "497628420");
        assert!(is_target("@anna"));
        assert!(is_target("k:u_ANNA"));
        assert!(is_target("497628420"));
        assert!(!is_target("@"));
        assert!(!is_target("12"));
        assert_eq!(target_label("497628420"), "497 628 420");
        assert_eq!(target_label("@Anna"), "@anna");
    }

    #[test]
    fn name_and_account_need_a_login() {
        own_dir("konto-noetig");
        let d = Service::neu("ws://127.0.0.1:9/fv/ws".into(), "00".repeat(32), "Test".into());
        assert_eq!(d.request("@anna"), Err(FriendError::AccountRequired));
        assert_eq!(d.request("k:u_ANNA"), Err(FriendError::AccountRequired));
        assert!(d.list_items().is_empty());
    }

    #[test]
    fn old_relay_reply_still_works() {
        // FreeViewer without account and a relay before r7: no new fields
        let roh = r#"{"ok":true,"fvid":"111111111","konto":"","freunde":[{"fvid":"222222222","name":"X","seit":1,"online":false,"gesehen":0}],
            "eingehend":[],"ausgehend":[],"blockiert":[{"fvid":"333333333","seit":2}]}"#;
        let s: Stand = serde_json::from_str(roh).unwrap();
        let mut b = book_with_me("111111111");
        b.adopt(&s);
        assert!(b.is_friend("222222222"));
        assert_eq!(b.friends[0].entry_key(), "222222222");
        assert!(b.is_blocked("333333333"));
    }

    // ------------------------------------------------ r32: Meeting-Einladung

    fn friend_with(fvid: &str, devices: &[(&str, bool)]) -> Friend {
        Friend {
            fvid: fvid.to_string(),
            name: "Anna".to_string(),
            devices: devices
                .iter()
                .map(|(id, on)| FriendDevice {
                    fvid: id.to_string(),
                    label: String::new(),
                    online: *on,
                    seen: 0,
                })
                .collect(),
            ..Default::default()
        }
    }

    /// A mini relay: accepts ONE connection, remembers the request and
    /// answers with status + body. Returns (port, received).
    fn mini_relay(status: u16, json_body: &'static str) -> (u16, std::sync::mpsc::Receiver<String>) {
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
                let reply = format!(
                    "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    status,
                    json_body.len(),
                    json_body
                );
                let _ = c.write_all(reply.as_bytes());
            }
        });
        (port, rx)
    }

    #[test]
    fn invite_goes_to_the_reachable_pc() {
        let f = friend_with("111111111", &[("111111111", false), ("222222222", true)]);
        assert_eq!(invite_target(&f), "222222222");
        let f = friend_with("111111111", &[("111111111", false), ("222222222", false)]);
        assert_eq!(invite_target(&f), "111111111");
    }

    #[test]
    fn without_pc_there_is_no_target() {
        let f = Friend {
            person: "u_anna".into(),
            ..Default::default()
        };
        assert_eq!(invite_target(&f), "");
        // only the old list: the ID itself
        let f = Friend {
            fvid: "333 444 555".into(),
            ..Default::default()
        };
        assert_eq!(invite_target(&f), "333444555");
    }

    #[test]
    fn link_with_key_never_goes_to_relay() {
        assert_eq!(
            link_for_relay(" https://meet.fleitec.com/?room=1&pass=2 ").as_deref(),
            Some("https://meet.fleitec.com/?room=1&pass=2")
        );
        assert!(link_for_relay("https://meet.fleitec.com/?room=1#k=abc").is_none());
        assert!(link_for_relay("   ").is_none());
    }

    #[test]
    fn only_own_meeting_page_is_opened() {
        let b = "https://meet.fleitec.com";
        assert!(invite_link_ok("https://meet.fleitec.com/?room=1-2-3&pass=x", b));
        assert!(invite_link_ok("https://meet.fleitec.com", b));
        assert!(invite_link_ok("https://meet.fleitec.com?room=1", b));
        // base with a slash at the end is the same
        assert!(invite_link_ok("https://meet.fleitec.com/?room=1", "https://meet.fleitec.com/"));
        assert!(!invite_link_ok("https://meet.fleitec.com.evil.de/?room=1", b));
        assert!(!invite_link_ok("https://meet.fleitec.com@evil.de/", b));
        assert!(!invite_link_ok("http://meet.fleitec.com/?room=1", b));
        assert!(!invite_link_ok("file:///C:/Windows/System32/calc.exe", b));
        assert!(!invite_link_ok("https://meet.fleitec.com/?room=1 --x", b));
        assert!(!invite_link_ok("", b));
        assert!(!invite_link_ok("https://meet.fleitec.com/", ""));
    }

    #[test]
    fn invites_reply_is_parseable() {
        let roh = r#"{"ok":true,"fvid":"1","konto":"","einladungen":[
            {"id":"a1","fvid":"222222222","name":"Anna","titel":"Mit Justin","link":"https://meet.fleitec.com/?room=1&pass=2","wann":5,"online":true,"neues_feld":1}]}"#;
        let a: InvitesReply = serde_json::from_str(roh).unwrap();
        assert_eq!(a.invites.len(), 1);
        assert_eq!(a.invites[0].von(), "Anna");
        assert_eq!(a.invites[0].heading, "Mit Justin");
        // without a mailbox field (old relay with 200?) it stays empty
        let a: InvitesReply = serde_json::from_str(r#"{"ok":true}"#).unwrap();
        assert!(a.invites.is_empty());
    }

    #[test]
    fn each_new_invite_is_reported_once() {
        own_dir("einl-melden");
        let d = Service::neu("ws://127.0.0.1:9/fv/ws".into(), "00".repeat(32), "Test".into());
        let e = |id: &str| Invite {
            id: id.into(),
            fvid: "222222222".into(),
            name: "Anna".into(),
            heading: "Mit Justin".into(),
            link: "https://meet.fleitec.com/?room=1&pass=2".into(),
            ..Default::default()
        };
        assert_eq!(d.invites_adopt(vec![e("a"), e("b")]), 2);
        assert_eq!(d.invites().len(), 2);
        // gleicher Stand beim naechsten Abgleich: nichts Neues
        assert_eq!(d.invites_adopt(vec![e("a"), e("b")]), 0);
        assert_eq!(d.invites_adopt(vec![e("a"), e("b"), e("c")]), 1);
        let mut received = 0;
        while let Some(m) = d.notice() {
            if let Notice::InviteReceived(von, heading) = m {
                assert_eq!(von, "Anna");
                assert_eq!(heading, "Mit Justin");
                received += 1;
            }
        }
        assert_eq!(received, 3);
        // entries without ID or link are junk and are ignored
        assert_eq!(
            d.invites_adopt(vec![Invite::default(), Invite { id: "x".into(), ..Default::default() }]),
            0
        );
        assert!(d.invites().is_empty());
    }

    #[test]
    fn invite_link_with_key_rejected_before_send() {
        own_dir("einl-schluessel");
        let d = Service::neu("ws://127.0.0.1:9/fv/ws".into(), "00".repeat(32), "Test".into());
        let f = friend_with("111111111", &[("111111111", true)]);
        assert_eq!(
            d.meeting_invite(&f, "x", "https://meet.fleitec.com/?room=1#k=abc"),
            Err(FriendError::LinkWithKey)
        );
        let without_pc = Friend {
            person: "u_anna".into(),
            ..Default::default()
        };
        assert_eq!(
            d.meeting_invite(&without_pc, "x", "https://meet.fleitec.com/?room=1"),
            Err(FriendError::NoDevice)
        );
    }

    #[test]
    fn relay_without_mailbox_is_no_channel() {
        // 404 {"error":"unbekannt"} = Relay vor Patch 0005
        let (port, rx) = mini_relay(404, r#"{"error":"unbekannt"}"#);
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        let r = send_invite(&relay, &"ab".repeat(16), "", "Test", "222222222", "T", "https://meet.fleitec.com/?room=1");
        assert_eq!(r, Err(FriendError::NoChannel));
        let roh = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(roh.starts_with("POST /fv/freunde/einladen "), "{}", roh);

        // an unknown ID, in contrast, is a normal error (the path exists)
        let (port, _rx) = mini_relay(404, r#"{"error":"Diese FreeViewer-ID gibt es nicht"}"#);
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        let r = send_invite(&relay, &"ab".repeat(16), "", "Test", "222222222", "T", "https://meet.fleitec.com/?room=1");
        assert_eq!(r, Err(FriendError::Netz("Diese FreeViewer-ID gibt es nicht".into())));
    }

    #[test]
    fn invite_carries_target_title_and_link_to_relay() {
        let (port, rx) = mini_relay(200, r#"{"ok":true,"zugestellt":true}"#);
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        let r = send_invite(
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
        let json_body = roh.split("\r\n\r\n").nth(1).unwrap_or("");
        let v: serde_json::Value = serde_json::from_str(json_body).unwrap();
        assert_eq!(v["an"], "222222222");
        assert_eq!(v["titel"], "Mit Anna");
        assert_eq!(v["link"], "https://meet.fleitec.com/?room=1&pass=2");
        assert_eq!(v["secret"], "ab".repeat(16));
        assert_eq!(v["token"], "tok");
        assert!(!json_body.contains("#k="));
    }

    #[test]
    fn fetch_and_clear_the_mailbox() {
        let (port, rx) = mini_relay(
            200,
            r#"{"ok":true,"einladungen":[{"id":"e1","fvid":"222222222","name":"Anna","titel":"T","link":"https://meet.fleitec.com/?room=1&pass=2","wann":1}]}"#,
        );
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        let l = fetch_invites(&relay, &"ab".repeat(16), "", "Test").unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].id, "e1");
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().starts_with("POST /fv/freunde/einladungen "));

        let (port, rx) = mini_relay(200, r#"{"ok":true,"weg":1}"#);
        let relay = format!("ws://127.0.0.1:{}/fv/ws", port);
        assert_eq!(
            send_invite_away(&relay, &"ab".repeat(16), "", "Test", &["e1".to_string()]),
            Ok(())
        );
        let roh = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(roh.starts_with("POST /fv/freunde/einladung-weg "));
        assert!(roh.contains(r#""ids":["e1"]"#), "{}", roh);
    }
}
