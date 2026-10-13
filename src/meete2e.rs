//! End-to-end encryption for the meeting (frame format v2).
//!
//! WHY: audio and video are only encrypted on the WIRE (DTLS-SRTP). The media
//! server has to forward the packets and therefore decrypts them. Here the
//! payload is encrypted at the sender with a key the server never gets.
//!
//! WHAT THE SERVER MAY SEE: it looks INTO video frames to find key frames
//! (it needs them to switch quality layers) and to repack NAL units into RTP.
//! So video keeps its NAL structure readable and only the contents are
//! encrypted (see the H.264 section). Audio gets a small clear header.
//!
//! FORMAT v2 (replaces v1, which reused (key, nonce) pairs - roadmap r1):
//!   v1 used ONE key per track kind for all participants and the nonce was
//!   `track kind + counter` starting at 0 for every sender. Two senders, the
//!   camera plus a screen share, or a rejoin therefore produced identical
//!   (key, nonce) pairs: AES-GCM then leaks the XOR of the plaintexts and the
//!   GHASH key. v2 fixes this:
//!
//!   * every outgoing stream (microphone, camera, each screen share, every
//!     rejoin) draws a fresh random 8-byte SENDER SALT,
//!   * the per-stream key is `HKDF-SHA256(root, salt = "freemeet-e2e-v2",
//!     info = kind || 0x00 || sender salt)`,
//!   * the nonce is `kind | nal index | salt[0..5] | counter`, so a nonce is
//!     unique per stream even if two salts ever shared a key,
//!   * a stream rotates to a new salt BEFORE its 32-bit counter would wrap.
//!
//! Audio frame:
//!   byte 0      0xE3 version (v1 frames start with 0xE2 and are rejected)
//!   byte 1      flags: bit 0 = key frame
//!   byte 2..10  sender salt
//!   byte 10..14 counter (32 bit, big endian)
//!   from 14     AES-256-GCM: ciphertext + 16 byte tag
//! The whole 14 byte header is the GCM associated data: the server can read
//! the key frame flag but cannot change it.
//!
//! Spec, vectors and migration: docs/FREEMEET-E2E-NONCE.md,
//! tests/freemeet-e2e-vectors.json (written by tools/freemeet-e2e/ref.py).

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// First byte of every v2 frame. A v1 receiver sees a foreign marker and
/// drops the frame instead of showing garbage.
pub const KENNUNG: u8 = 0xE3;
/// First byte of v1 audio frames (the vulnerable format). Never accepted.
pub const MARKER_V1: u8 = 0xE2;
/// Length of the clear audio header: version, flags, salt, counter.
pub const KOPF: usize = 14;
/// Length of the sender salt.
pub const SALT_LEN: usize = 8;
/// Length of the per-NAL blob header: version, salt, counter, NAL index.
const NAL_HEAD: usize = 1 + SALT_LEN + 4 + 2;
/// Flags byte, bit 0.
const MERKER_SCHLUESSELBILD: u8 = 0x01;
/// HKDF salt of format v2 (v1 used "freemeet-e2e-v1": keys never overlap).
const KDF_SALT: &[u8] = b"freemeet-e2e-v2";
/// How many derived stream keys a receiver remembers before it starts over.
const KEY_CACHE_MAX: usize = 64;
/// How many sender salts a replay guard remembers per track.
const GUARD_SALTS_MAX: usize = 16;

/// Random per-stream value that separates the keys and nonces of all senders.
pub type Salt = [u8; SALT_LEN];

/// Kind of track. Audio and video get different keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Spur {
    Ton,
    Bild,
}

impl Spur {
    fn id(self) -> u8 {
        match self {
            Spur::Ton => 1,
            Spur::Bild => 2,
        }
    }
    fn info(self) -> &'static [u8] {
        match self {
            Spur::Ton => b"ton",
            Spur::Bild => b"bild",
        }
    }
}

/// Key of one stream: HKDF(root, kind, sender salt).
pub fn derive_key(root: &[u8; 32], spur: Spur, salt: &Salt) -> [u8; 32] {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(KDF_SALT), root);
    let mut info = Vec::with_capacity(spur.info().len() + 1 + SALT_LEN);
    info.extend_from_slice(spur.info());
    info.push(0);
    info.extend_from_slice(salt);
    let mut k = [0u8; 32];
    // 32 bytes can never exceed the HKDF output limit.
    hk.expand(&info, &mut k).expect("32 bytes always fit");
    k
}

/// GCM nonce: kind | NAL index | first 5 salt bytes | counter.
pub fn make_nonce(spur: Spur, nal: u16, salt: &Salt, counter: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[0] = spur.id();
    n[1..3].copy_from_slice(&nal.to_be_bytes());
    n[3..8].copy_from_slice(&salt[..5]);
    n[8..12].copy_from_slice(&counter.to_be_bytes());
    n
}

fn random_salt() -> Salt {
    use rand::RngCore;
    let mut s = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut s);
    s
}

/// A room key. Only holders of the link have it - never the server.
///
/// Cloning is cheap and shares the cache of derived stream keys.
#[derive(Clone)]
pub struct Schluessel {
    roh: [u8; 32],
    cache: Arc<Mutex<HashMap<(Spur, Salt), Aes256Gcm>>>,
}

impl Schluessel {
    /// From 32 random bytes (the root secret of the room).
    pub fn aus_roh(roh: [u8; 32]) -> Schluessel {
        Schluessel {
            roh,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Roll a fresh random key (when the meeting is created).
    pub fn neu() -> Schluessel {
        use rand::RngCore;
        let mut roh = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut roh);
        Schluessel::aus_roh(roh)
    }

    /// For the link: 43 characters of URL-safe base64 without padding.
    pub fn als_text(&self) -> String {
        base64_url(&self.roh)
    }

    /// Back from the link. None = not a valid key.
    pub fn aus_text(t: &str) -> Option<Schluessel> {
        let roh = base64_url_zurueck(t.trim())?;
        if roh.len() != 32 {
            return None;
        }
        let mut a = [0u8; 32];
        a.copy_from_slice(&roh);
        Some(Schluessel::aus_roh(a))
    }

    fn cached(&self, spur: Spur, salt: &Salt) -> Option<Aes256Gcm> {
        let c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        c.get(&(spur, *salt)).cloned()
    }

    fn remember(&self, spur: Spur, salt: &Salt, cipher: &Aes256Gcm) {
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if c.len() >= KEY_CACHE_MAX {
            // Starting over is always safe: keys are re-derived on demand.
            c.clear();
        }
        c.insert((spur, *salt), cipher.clone());
    }

    fn derive_cipher(&self, spur: Spur, salt: &Salt) -> Aes256Gcm {
        let k = derive_key(&self.roh, spur, salt);
        Aes256Gcm::new((&k).into())
    }

    /// Cipher of an OWN stream (always cached).
    fn cipher(&self, spur: Spur, salt: &Salt) -> Aes256Gcm {
        if let Some(c) = self.cached(spur, salt) {
            return c;
        }
        let c = self.derive_cipher(spur, salt);
        self.remember(spur, salt, &c);
        c
    }

    /// Start an outgoing stream with a fresh random sender salt. Create ONE
    /// `Sender` per microphone, camera, screen share and per (re)join.
    pub fn sender(&self, spur: Spur) -> Sender {
        self.sender_mit_salz(spur, random_salt())
    }

    /// Like `sender`, with a chosen salt (tests only).
    pub fn sender_mit_salz(&self, spur: Spur, salt: Salt) -> Sender {
        Sender {
            key: self.clone(),
            spur,
            salt,
            next: 0,
        }
    }

    /// Encrypt one audio frame with an explicit salt + counter. LOW LEVEL:
    /// the caller must never use the same (salt, counter) twice - use a
    /// `Sender`, which guarantees it. Public for the test vectors.
    pub fn seal_audio_at(
        &self,
        spur: Spur,
        salt: &Salt,
        counter: u32,
        schluesselbild: bool,
        klar: &[u8],
    ) -> Vec<u8> {
        let mut kopf = [0u8; KOPF];
        kopf[0] = KENNUNG;
        kopf[1] = if schluesselbild { MERKER_SCHLUESSELBILD } else { 0 };
        kopf[2..2 + SALT_LEN].copy_from_slice(salt);
        kopf[2 + SALT_LEN..KOPF].copy_from_slice(&counter.to_be_bytes());
        let nonce = make_nonce(spur, 0, salt, counter);
        let geheim = self
            .cipher(spur, salt)
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: klar,
                    aad: &kopf,
                },
            )
            .unwrap_or_default();
        let mut aus = Vec::with_capacity(KOPF + geheim.len());
        aus.extend_from_slice(&kopf);
        aus.extend_from_slice(&geheim);
        aus
    }

    /// Decrypt one audio frame. None = not our format (also: v1), wrong key
    /// or tampered - in every case do NOT show it.
    pub fn oeffnen(&self, spur: Spur, rahmen: &[u8]) -> Option<Vec<u8>> {
        if !ist_geschuetzt(rahmen) {
            return None;
        }
        let kopf = &rahmen[..KOPF];
        let salt = salz_von(rahmen)?;
        let counter = zaehler_von(rahmen)?;
        let nonce = make_nonce(spur, 0, &salt, counter);
        // The salt is attacker-controlled until the tag checks out, so a
        // stream key is only remembered after a successful decryption.
        let (cipher, was_cached) = match self.cached(spur, &salt) {
            Some(c) => (c, true),
            None => (self.derive_cipher(spur, &salt), false),
        };
        let klar = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &rahmen[KOPF..],
                    aad: kopf,
                },
            )
            .ok()?;
        if !was_cached {
            self.remember(spur, &salt, &cipher);
        }
        Some(klar)
    }
}

/// One outgoing stream: owns the sender salt and the counter, so a
/// (key, nonce) pair can never repeat.
pub struct Sender {
    key: Schluessel,
    spur: Spur,
    salt: Salt,
    /// Next counter value. 64 bit so exhaustion is detectable without wrap.
    next: u64,
}

impl Sender {
    /// Current sender salt (it travels in every frame).
    pub fn salz(&self) -> Salt {
        self.salt
    }

    /// Hand out the next (salt, counter); a new salt starts right BEFORE the
    /// 32-bit counter would wrap.
    fn take(&mut self) -> (Salt, u32) {
        if self.next > u32::MAX as u64 {
            self.salt = random_salt();
            self.next = 0;
        }
        let c = self.next as u32;
        self.next += 1;
        (self.salt, c)
    }

    /// Encrypt one audio frame.
    pub fn schuetzen(&mut self, schluesselbild: bool, klar: &[u8]) -> Vec<u8> {
        debug_assert_eq!(self.spur, Spur::Ton, "audio frames need an audio sender");
        let (salt, c) = self.take();
        self.key
            .seal_audio_at(self.spur, &salt, c, schluesselbild, klar)
    }

    /// Encrypt one H.264 frame (structure stays readable).
    pub fn schuetzen_h264(&mut self, rahmen: &[u8]) -> Vec<u8> {
        debug_assert_eq!(self.spur, Spur::Bild, "video frames need a video sender");
        let (salt, c) = self.take();
        self.key.seal_h264_at(&salt, c, rahmen)
    }

    /// Test hook: jump the counter (to test the rotation at the end).
    #[cfg(test)]
    fn set_next(&mut self, n: u64) {
        self.next = n;
    }
}

// ------------------------------------------------------- H.264 (Bild) ----
//
// WARUM Bild anders behandelt wird als Ton:
//
// Der Medienserver ist KEIN reiner Weiterleiter. Er bekommt entpackte Bilder
// und setzt sie neu zu RTP-Paketen zusammen - dafuer MUSS er die Struktur
// des Bildes verstehen (NAL-Einheiten, STAP-A). Gemessen am 07.08.2026:
// verschluesselt man den ganzen Rahmen, scheitert er mit
// "NaluTypeIsNotHandled" und beim Gegenueber kommt kein Bild an.
//
// Deshalb bleibt die STRUKTUR sichtbar und nur der INHALT wird verschluesselt:
//   * Startcodes und das Kopfbyte jeder NAL-Einheit bleiben im Klartext,
//   * SPS (7), PPS (8) und AUD (9) bleiben ganz unangetastet - darin steht
//     nur, wie gross das Bild ist, kein Bildinhalt; der Packer braucht sie,
//   * alles andere wird verschluesselt.
//
// Denselben Kompromiss geht SFrame bei H.264 ein: der Server sieht die
// Rahmenstruktur und die Groessen, aber KEINE Inhalte.
//
// Zusaetzlich noetig: der Geheimtext darf zufaellig "00 00 01" enthalten -
// das waere ein falscher Startcode und wuerde das Bild zerlegen. Deshalb
// wird derselbe Schutz eingesetzt, den H.264 selbst benutzt (eine 0x03
// dazwischen, "emulation prevention").

/// Wo faengt die naechste NAL-Einheit an? Liefert (Beginn des Startcodes,
/// Beginn der Daten).
fn naechster_startcode(daten: &[u8], ab: usize) -> Option<(usize, usize)> {
    let mut i = ab;
    while i + 3 <= daten.len() {
        if daten[i] == 0 && daten[i + 1] == 0 {
            if daten[i + 2] == 1 {
                return Some((i, i + 3));
            }
            if i + 4 <= daten.len() && daten[i + 2] == 0 && daten[i + 3] == 1 {
                return Some((i, i + 4));
            }
        }
        i += 1;
    }
    None
}

/// Einen Annex-B-Rahmen in seine NAL-Einheiten zerlegen: (Startcode-Beginn,
/// Daten-Beginn, Daten-Ende).
pub fn nal_einheiten(daten: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut aus = Vec::new();
    let mut suche = 0usize;
    let mut offen: Option<(usize, usize)> = None;
    while let Some((sc, db)) = naechster_startcode(daten, suche) {
        if let Some((osc, odb)) = offen {
            aus.push((osc, odb, sc));
        }
        offen = Some((sc, db));
        suche = db;
    }
    if let Some((osc, odb)) = offen {
        aus.push((osc, odb, daten.len()));
    }
    aus
}

/// Muss diese NAL-Einheit im Klartext bleiben?
///
/// SPS/PPS beschreiben nur Groesse und Kodierparameter - kein Bildinhalt -
/// und der Packer im Server braucht sie. AUD ist ein reiner Trenner.
fn nal_bleibt_klar(typ: u8) -> bool {
    matches!(typ, 7 | 8 | 9)
}

/// Den Schutz gegen falsche Startcodes einfuegen (wie H.264 selbst).
fn epb_ein(daten: &[u8]) -> Vec<u8> {
    let mut aus = Vec::with_capacity(daten.len() + daten.len() / 64 + 4);
    let mut nullen = 0usize;
    for &b in daten {
        if nullen >= 2 && b <= 3 {
            aus.push(3);
            nullen = 0;
        }
        aus.push(b);
        if b == 0 {
            nullen += 1;
        } else {
            nullen = 0;
        }
    }
    aus
}

/// Den Schutz wieder herausnehmen.
fn epb_raus(daten: &[u8]) -> Vec<u8> {
    let mut aus = Vec::with_capacity(daten.len());
    let mut nullen = 0usize;
    let mut i = 0usize;
    while i < daten.len() {
        let b = daten[i];
        if nullen >= 2 && b == 3 && i + 1 < daten.len() && daten[i + 1] <= 3 {
            nullen = 0;
            i += 1;
            continue;
        }
        aus.push(b);
        if b == 0 {
            nullen += 1;
        } else {
            nullen = 0;
        }
        i += 1;
    }
    aus
}

// Per-NAL blob (inside the emulation-prevention escaping), format v2:
//   byte 0       0xE3 version
//   byte 1..9    sender salt
//   byte 9..13   counter (32 bit, big endian)
//   byte 13..15  NAL index inside the frame (16 bit, big endian)
//   from 15      AES-256-GCM: ciphertext + 16 byte tag
// Associated data: NAL header byte + the 15 byte blob header.

impl Schluessel {
    /// Encrypt one H.264 frame with an explicit salt + counter. LOW LEVEL
    /// like `seal_audio_at`: use a `Sender`.
    pub fn seal_h264_at(&self, salt: &Salt, counter: u32, rahmen: &[u8]) -> Vec<u8> {
        let teile = nal_einheiten(rahmen);
        if teile.is_empty() {
            return rahmen.to_vec(); // not Annex B: leave untouched
        }
        if teile.len() > u16::MAX as usize {
            // The NAL index (part of the nonce) would repeat. Never happens
            // with a real encoder; send nothing rather than reuse a nonce.
            return Vec::new();
        }
        let cipher = self.cipher(Spur::Bild, salt);
        let mut aus = Vec::with_capacity(rahmen.len() + teile.len() * (NAL_HEAD + 16 + 2));
        for (nr, (sc, db, ende)) in teile.iter().enumerate() {
            let kopf_byte = rahmen[*db];
            let typ = kopf_byte & 0x1f;
            // Start code and NAL header byte always stay in the clear.
            aus.extend_from_slice(&rahmen[*sc..*db + 1]);
            let inhalt = &rahmen[*db + 1..*ende];
            if nal_bleibt_klar(typ) || inhalt.is_empty() {
                aus.extend_from_slice(inhalt);
                continue;
            }
            let nr16 = nr as u16;
            let mut blob = Vec::with_capacity(NAL_HEAD + inhalt.len() + 16);
            blob.push(KENNUNG);
            blob.extend_from_slice(salt);
            blob.extend_from_slice(&counter.to_be_bytes());
            blob.extend_from_slice(&nr16.to_be_bytes());
            // NAL header byte + blob header are authenticated: swapping the
            // counter, salt or NAL index invalidates the frame.
            let mut aad = Vec::with_capacity(1 + NAL_HEAD);
            aad.push(kopf_byte);
            aad.extend_from_slice(&blob);
            let nonce = make_nonce(Spur::Bild, nr16, salt, counter);
            let geheim = cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: inhalt,
                        aad: &aad,
                    },
                )
                .unwrap_or_default();
            blob.extend_from_slice(&geheim);
            aus.extend_from_slice(&epb_ein(&blob));
        }
        aus
    }

    /// Open an encrypted H.264 frame. None = wrong key, tampered, or an old
    /// (v1) / foreign format.
    pub fn oeffnen_h264(&self, rahmen: &[u8]) -> Option<Vec<u8>> {
        let teile = nal_einheiten(rahmen);
        if teile.is_empty() {
            return None;
        }
        let mut aus = Vec::with_capacity(rahmen.len());
        for (sc, db, ende) in teile {
            let kopf_byte = rahmen[db];
            let typ = kopf_byte & 0x1f;
            aus.extend_from_slice(&rahmen[sc..db + 1]);
            let inhalt = &rahmen[db + 1..ende];
            if nal_bleibt_klar(typ) || inhalt.is_empty() {
                aus.extend_from_slice(inhalt);
                continue;
            }
            let roh = epb_raus(inhalt);
            if roh.len() < NAL_HEAD + 16 || roh[0] != KENNUNG {
                return None;
            }
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&roh[1..1 + SALT_LEN]);
            let o = 1 + SALT_LEN;
            let counter = u32::from_be_bytes([roh[o], roh[o + 1], roh[o + 2], roh[o + 3]]);
            let nr16 = u16::from_be_bytes([roh[o + 4], roh[o + 5]]);
            let mut aad = Vec::with_capacity(1 + NAL_HEAD);
            aad.push(kopf_byte);
            aad.extend_from_slice(&roh[..NAL_HEAD]);
            let nonce = make_nonce(Spur::Bild, nr16, &salt, counter);
            let (cipher, was_cached) = match self.cached(Spur::Bild, &salt) {
                Some(c) => (c, true),
                None => (self.derive_cipher(Spur::Bild, &salt), false),
            };
            let klar = cipher
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &roh[NAL_HEAD..],
                        aad: &aad,
                    },
                )
                .ok()?;
            if !was_cached {
                self.remember(Spur::Bild, &salt, &cipher);
            }
            aus.extend_from_slice(&klar);
        }
        Some(aus)
    }
}

/// Does this look like a protected (v2) audio frame? (The server asks too.)
pub fn ist_geschuetzt(rahmen: &[u8]) -> bool {
    rahmen.len() > KOPF + 16 && rahmen[0] == KENNUNG
}

/// Is this an audio frame of the OLD, vulnerable format (v1)? Such a
/// participant has to update; their frames are never opened.
pub fn ist_altes_format(rahmen: &[u8]) -> bool {
    !rahmen.is_empty() && rahmen[0] == MARKER_V1
}

/// Does it contain a key frame? The server may read this - it needs it to
/// switch quality layers.
pub fn ist_schluesselbild(rahmen: &[u8]) -> Option<bool> {
    if !ist_geschuetzt(rahmen) {
        return None;
    }
    Some(rahmen[1] & MERKER_SCHLUESSELBILD != 0)
}

/// Counter of an audio frame.
pub fn zaehler_von(rahmen: &[u8]) -> Option<u32> {
    if !ist_geschuetzt(rahmen) {
        return None;
    }
    let o = 2 + SALT_LEN;
    Some(u32::from_be_bytes([
        rahmen[o],
        rahmen[o + 1],
        rahmen[o + 2],
        rahmen[o + 3],
    ]))
}

/// Sender salt of an audio frame.
pub fn salz_von(rahmen: &[u8]) -> Option<Salt> {
    if !ist_geschuetzt(rahmen) {
        return None;
    }
    let mut s = [0u8; SALT_LEN];
    s.copy_from_slice(&rahmen[2..2 + SALT_LEN]);
    Some(s)
}

/// Replay window (64 numbers below the highest) of ONE sender salt.
#[derive(Default)]
struct Window {
    hoechste: u32,
    /// Bit field of the last 64 numbers below `hoechste` (the network may reorder).
    fenster: u64,
}

impl Window {
    fn pruefen(&mut self, zaehler: u32) -> bool {
        if zaehler > self.hoechste {
            let schritt = zaehler - self.hoechste;
            self.fenster = if schritt >= 64 {
                0
            } else {
                (self.fenster << schritt) | (1u64 << (schritt - 1))
            };
            self.hoechste = zaehler;
            return true;
        }
        let zurueck = self.hoechste - zaehler;
        if zurueck == 0 || zurueck > 64 {
            return false; // exactly the highest, or long out of the window
        }
        let bit = 1u64 << (zurueck - 1);
        if self.fenster & bit != 0 {
            return false;
        }
        self.fenster |= bit;
        true
    }
}

/// Remembers which numbers were already seen - against replayed frames.
///
/// A server-side attacker could otherwise re-inject old frames: the tag would
/// be valid, the picture a lie. One window per sender salt, because each
/// sender stream counts on its own. Call it only AFTER a frame decrypted.
/// The last `GUARD_SALTS_MAX` salts are remembered; a replay of a stream that
/// was evicted since is not detected (documented limit).
#[derive(Default)]
pub struct Wache {
    fenster: Vec<(Salt, Window)>,
}

impl Wache {
    /// True = new and fine. False = repeat or too old.
    pub fn pruefen(&mut self, salz: &Salt, zaehler: u32) -> bool {
        if let Some((_, f)) = self.fenster.iter_mut().find(|(s, _)| s == salz) {
            return f.pruefen(zaehler);
        }
        if self.fenster.len() >= GUARD_SALTS_MAX {
            self.fenster.remove(0);
        }
        let f = Window {
            hoechste: zaehler,
            fenster: 0,
        };
        self.fenster.push((*salz, f));
        true
    }
}

// ------------------------------------------------------------- Base64 -----
// URL-sicher, ohne Fuellzeichen. Selbst gerechnet, damit keine weitere
// Abhaengigkeit noetig ist.

const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn base64_url(daten: &[u8]) -> String {
    let mut aus = String::new();
    for stueck in daten.chunks(3) {
        let b = [
            stueck[0],
            *stueck.get(1).unwrap_or(&0),
            *stueck.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        let zeichen = stueck.len() + 1;
        for i in 0..zeichen {
            aus.push(ABC[((n >> (18 - i * 6)) & 0x3f) as usize] as char);
        }
    }
    aus
}

pub fn base64_url_zurueck(text: &str) -> Option<Vec<u8>> {
    let wert = |c: u8| -> Option<u32> { ABC.iter().position(|x| *x == c).map(|p| p as u32) };
    let roh: Vec<u8> = text.bytes().filter(|c| *c != b'=').collect();
    let mut aus = Vec::new();
    for stueck in roh.chunks(4) {
        if stueck.len() < 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, c) in stueck.iter().enumerate() {
            n |= wert(*c)? << (18 - i * 6);
        }
        for i in 0..stueck.len() - 1 {
            aus.push(((n >> (16 - i * 8)) & 0xff) as u8);
        }
    }
    Some(aus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const SALT_A: Salt = [0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8];
    const SALT_B: Salt = [1, 2, 3, 4, 5, 6, 7, 8];

    /// A frame like the encoder delivers it: SPS, PPS, then a picture.
    fn beispielrahmen() -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&[0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f, 0x96, 0x54]); // SPS (7)
        f.extend_from_slice(&[0, 0, 0, 1, 0x68, 0xce, 0x3c, 0x80]); // PPS (8)
        f.extend_from_slice(&[0, 0, 0, 1, 0x65]); // IDR (5)
        f.extend((0..400u32).map(|i| (i * 13 % 256) as u8));
        f.extend_from_slice(&[0, 0, 1, 0x41]); // non-IDR (1), 3-byte start code
        f.extend((0..250u32).map(|i| (i * 29 % 256) as u8));
        f
    }

    fn video(k: &Schluessel) -> Sender {
        k.sender(Spur::Bild)
    }
    fn audio(k: &Schluessel) -> Sender {
        k.sender(Spur::Ton)
    }

    // ------------------------------------------------------ H.264 --------

    #[test]
    fn h264_hin_und_zurueck() {
        let k = Schluessel::neu();
        let f = beispielrahmen();
        let g = video(&k).schuetzen_h264(&f);
        assert_ne!(g, f, "nothing encrypted");
        assert_eq!(k.oeffnen_h264(&g).as_deref(), Some(&f[..]), "does not come back intact");
    }

    /// THE point where the first attempt failed: the server must still be
    /// able to read the structure.
    #[test]
    fn die_struktur_bleibt_lesbar() {
        let k = Schluessel::neu();
        let f = beispielrahmen();
        let g = video(&k).schuetzen_h264(&f);
        let vorher: Vec<u8> = nal_einheiten(&f).iter().map(|(_, d, _)| f[*d] & 0x1f).collect();
        let nachher: Vec<u8> = nal_einheiten(&g).iter().map(|(_, d, _)| g[*d] & 0x1f).collect();
        assert_eq!(vorher, nachher, "NAL types changed");
        assert_eq!(vorher, vec![7, 8, 5, 1]);
    }

    /// SPS and PPS must stay identical - the packer in the server reads them.
    #[test]
    fn sps_und_pps_bleiben_unangetastet() {
        let k = Schluessel::neu();
        let f = beispielrahmen();
        let g = video(&k).schuetzen_h264(&f);
        for (sc, db, ende) in nal_einheiten(&f) {
            let typ = f[db] & 0x1f;
            if typ == 7 || typ == 8 {
                let stueck = &f[sc..ende];
                assert!(
                    g.windows(stueck.len()).any(|w| w == stueck),
                    "NAL type {} was changed",
                    typ
                );
            }
        }
    }

    /// The picture content must NOT be in the frame any more.
    #[test]
    fn h264_inhalt_ist_wirklich_weg() {
        let k = Schluessel::neu();
        let mut f = vec![0, 0, 0, 1, 0x65];
        let geheim: Vec<u8> = b"SECRETPICTURECONTENT".repeat(6);
        f.extend_from_slice(&geheim);
        let g = video(&k).schuetzen_h264(&f);
        assert!(
            !g.windows(geheim.len()).any(|w| w == &geheim[..]),
            "the picture content lies open in the frame"
        );
    }

    /// False start codes would tear the picture apart - never, not even if
    /// the ciphertext happens to contain 00 00 01.
    #[test]
    fn es_entstehen_keine_falschen_startcodes() {
        let k = Schluessel::neu();
        let mut s = video(&k);
        for runde in 0..300u32 {
            let mut f = vec![0, 0, 0, 1, 0x41];
            f.extend((0..64u32).map(|i| ((i * runde + 7) % 256) as u8));
            let g = s.schuetzen_h264(&f);
            assert_eq!(nal_einheiten(&g).len(), 1, "round {}: false start codes", runde);
            assert_eq!(k.oeffnen_h264(&g).as_deref(), Some(&f[..]), "round {}", runde);
        }
    }

    #[test]
    fn h264_fremder_schluessel_oeffnet_nichts() {
        let a = Schluessel::neu();
        let b = Schluessel::neu();
        let g = video(&a).schuetzen_h264(&beispielrahmen());
        assert!(b.oeffnen_h264(&g).is_none());
    }

    #[test]
    fn h264_verfaelschung_faellt_auf() {
        let k = Schluessel::neu();
        let g = video(&k).schuetzen_h264(&beispielrahmen());
        let mut kaputt = g.clone();
        let mitte = g.len() - 40;
        kaputt[mitte] ^= 0x01;
        assert!(k.oeffnen_h264(&kaputt).is_none(), "tampering went unnoticed");
    }

    /// The key frame stays recognisable at NAL type 5.
    #[test]
    fn schluesselbild_bleibt_erkennbar() {
        let k = Schluessel::neu();
        let g = video(&k).schuetzen_h264(&beispielrahmen());
        let hat_idr = nal_einheiten(&g).iter().any(|(_, d, _)| g[*d] & 0x1f == 5);
        assert!(hat_idr, "the server can no longer find the key frame");
    }

    #[test]
    fn schutz_vor_falschen_startcodes_ist_umkehrbar() {
        for muster in [
            vec![0u8, 0, 1, 2, 3],
            vec![0, 0, 0, 0, 0, 0],
            vec![0, 0, 3, 0, 0, 1],
            vec![1, 2, 3, 4, 5],
            vec![0, 0, 2],
        ] {
            let ein = epb_ein(&muster);
            assert_eq!(epb_raus(&ein), muster, "pattern {:?}", muster);
        }
    }

    /// An old (v1) video blob starts with the NAL index, not with 0xE3.
    /// A v2 receiver must reject it instead of guessing.
    #[test]
    fn old_video_format_is_rejected() {
        let k = Schluessel::neu();
        let mut f = vec![0, 0, 0, 1, 0x65];
        f.push(0); // v1: NAL index 0
        f.extend_from_slice(&[0, 0, 0, 5]); // v1: counter
        f.extend((0..40u8).map(|i| i.wrapping_mul(7).wrapping_add(9)));
        assert!(k.oeffnen_h264(&f).is_none());
    }

    // ------------------------------------------------------- audio -------

    #[test]
    fn hin_und_zurueck() {
        let k = Schluessel::neu();
        let klar = b"Hello, this is a frame.";
        let g = audio(&k).schuetzen(true, klar);
        assert!(ist_geschuetzt(&g));
        assert_eq!(ist_schluesselbild(&g), Some(true));
        assert_eq!(zaehler_von(&g), Some(0));
        assert_eq!(k.oeffnen(Spur::Ton, &g).as_deref(), Some(&klar[..]));
    }

    /// The core: the plaintext must NOT be in the frame.
    #[test]
    fn der_klartext_steht_nicht_drin() {
        let k = Schluessel::neu();
        let klar = b"SECRETWORD";
        let g = audio(&k).schuetzen(false, klar);
        assert!(g.windows(klar.len()).all(|f| f != klar), "plaintext lies open");
    }

    /// Another key must open NOTHING - that is the whole point.
    #[test]
    fn fremder_schluessel_oeffnet_nichts() {
        let a = Schluessel::neu();
        let b = Schluessel::neu();
        let g = audio(&a).schuetzen(false, b"content");
        assert!(b.oeffnen(Spur::Ton, &g).is_none());
    }

    /// Audio and video keys are separate: an audio frame never passes as video.
    #[test]
    fn spuren_sind_getrennt() {
        let k = Schluessel::neu();
        let g = audio(&k).schuetzen(false, b"sound");
        assert!(k.oeffnen(Spur::Bild, &g).is_none());
        assert!(k.oeffnen(Spur::Ton, &g).is_some());
    }

    /// Every change to the frame must be noticed - also in the CLEAR header
    /// (flags, salt, counter), or the server could forge the key frame flag
    /// or splice a frame onto another stream.
    #[test]
    fn jede_verfaelschung_faellt_auf() {
        let k = Schluessel::neu();
        let g = audio(&k).schuetzen(true, b"A longer content to check");
        for stelle in [1usize, 2, 9, 10, 13, KOPF, KOPF + 3, g.len() - 1] {
            let mut kaputt = g.clone();
            kaputt[stelle] ^= 0x01;
            assert!(
                k.oeffnen(Spur::Ton, &kaputt).is_none(),
                "change at position {} went unnoticed",
                stelle
            );
        }
    }

    #[test]
    fn unencrypted_frames_are_left_alone() {
        let k = Schluessel::neu();
        // An unencrypted H.264 frame starts with 00 00 00 01.
        let alt = [0u8, 0, 0, 1, 0x65, 0x88, 0x84, 0x21, 0x00, 0x11, 0x22, 0x33,
                   0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
                   0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert!(!ist_geschuetzt(&alt));
        assert_eq!(ist_schluesselbild(&alt), None);
        assert!(k.oeffnen(Spur::Bild, &alt).is_none());
    }

    /// v1 audio frames (marker 0xE2, 6 byte header) are recognised as OLD
    /// and never opened.
    #[test]
    fn old_audio_format_is_detected_and_rejected() {
        let k = Schluessel::neu();
        let mut v1 = vec![MARKER_V1, 0x01, 0, 0, 0, 7];
        v1.extend((0..40u8).map(|i| i ^ 0x5a));
        assert!(ist_altes_format(&v1));
        assert!(!ist_geschuetzt(&v1));
        assert!(k.oeffnen(Spur::Ton, &v1).is_none());
        let neu = audio(&k).schuetzen(false, b"x");
        assert!(!ist_altes_format(&neu));
    }

    #[test]
    fn schluessel_ueberlebt_den_link() {
        let k = Schluessel::neu();
        let t = k.als_text();
        assert_eq!(t.len(), 43, "43 characters expected, got {}", t.len());
        assert!(!t.contains('+') && !t.contains('/') && !t.contains('='), "not link safe: {}", t);
        let zurueck = Schluessel::aus_text(&t).expect("must come back");
        let g = audio(&k).schuetzen(false, b"probe");
        assert_eq!(zurueck.oeffnen(Spur::Ton, &g).as_deref(), Some(&b"probe"[..]));
    }

    #[test]
    fn unsinniger_schluesseltext_wird_abgelehnt() {
        assert!(Schluessel::aus_text("").is_none());
        assert!(Schluessel::aus_text("zu-kurz").is_none());
        assert!(Schluessel::aus_text(&"A".repeat(43)).is_some());
        assert!(Schluessel::aus_text("!!!!").is_none());
    }

    #[test]
    fn base64_haelt_beliebige_daten_aus() {
        for laenge in [1usize, 2, 3, 4, 31, 32, 33, 100] {
            let daten: Vec<u8> = (0..laenge).map(|i| (i * 7 % 256) as u8).collect();
            let t = base64_url(&daten);
            assert_eq!(base64_url_zurueck(&t).as_deref(), Some(&daten[..]), "length {}", laenge);
        }
    }

    /// Header is 14 bytes: the overhead per frame is 14 + 16 tag bytes.
    #[test]
    fn der_aufschlag_bleibt_klein() {
        let k = Schluessel::neu();
        let klar = vec![0u8; 5000];
        let g = k.sender(Spur::Ton).schuetzen(false, &klar);
        assert_eq!(g.len(), klar.len() + KOPF + 16, "overhead changed");
    }

    // ------------------------------------------------------ replay -------

    #[test]
    fn wache_laesst_neues_durch_und_wiederholungen_nicht() {
        let mut w = Wache::default();
        assert!(w.pruefen(&SALT_A, 10));
        assert!(w.pruefen(&SALT_A, 11));
        assert!(!w.pruefen(&SALT_A, 11), "repeat let through");
        assert!(!w.pruefen(&SALT_A, 10), "repeat let through");
        // The network may reorder: delivering a gap later is fine.
        assert!(w.pruefen(&SALT_A, 13));
        assert!(w.pruefen(&SALT_A, 12), "late packet wrongly dropped");
        assert!(!w.pruefen(&SALT_A, 12));
        // 1 is still INSIDE the 64 window at highest = 13: legit late packet.
        assert!(w.pruefen(&SALT_A, 1), "late packet inside the window wrongly dropped");
        assert!(!w.pruefen(&SALT_A, 1), "same one let through twice");
        // Only far outside the window it is refused.
        assert!(w.pruefen(&SALT_A, 200));
        assert!(!w.pruefen(&SALT_A, 13), "ancient frame (187 back) let through");
    }

    #[test]
    fn wache_kommt_mit_grossen_spruengen_klar() {
        let mut w = Wache::default();
        assert!(w.pruefen(&SALT_A, 1));
        assert!(w.pruefen(&SALT_A, 1_000_000));
        assert!(!w.pruefen(&SALT_A, 500), "ancient frame let through");
        assert!(w.pruefen(&SALT_A, 1_000_001));
    }

    /// Every sender stream counts on its own: counter 5 of sender B is not a
    /// repeat of counter 5 of sender A; but each one only once.
    #[test]
    fn guard_keeps_one_window_per_sender() {
        let mut w = Wache::default();
        assert!(w.pruefen(&SALT_A, 5));
        assert!(w.pruefen(&SALT_B, 5));
        assert!(!w.pruefen(&SALT_A, 5));
        assert!(!w.pruefen(&SALT_B, 5));
        assert!(w.pruefen(&SALT_B, 6));
        assert!(w.pruefen(&SALT_A, 4)); // late, inside A's window
    }

    #[test]
    fn guard_forgets_only_the_oldest_salts() {
        let mut w = Wache::default();
        for i in 0..(GUARD_SALTS_MAX as u8 + 3) {
            let s: Salt = [i, 9, 9, 9, 9, 9, 9, 9];
            assert!(w.pruefen(&s, 1));
            assert!(!w.pruefen(&s, 1));
        }
        assert!(w.fenster.len() <= GUARD_SALTS_MAX, "guard grows without limit");
    }

    // --------------------------------------------- nonce uniqueness ------
    //
    // The r1 bug: (key, nonce) repeated across senders, across camera and
    // screen share, and across a rejoin. These tests collect the REAL
    // (stream key, GCM nonce) pairs from sealed frames and demand that no
    // pair occurs twice.

    fn pairs_audio(k: &Schluessel, frame: &[u8]) -> (Vec<u8>, [u8; 12]) {
        let salt = salz_von(frame).unwrap();
        let c = zaehler_von(frame).unwrap();
        (derive_key(&k.roh, Spur::Ton, &salt).to_vec(), make_nonce(Spur::Ton, 0, &salt, c))
    }

    fn pairs_video(k: &Schluessel, frame: &[u8]) -> Vec<(Vec<u8>, [u8; 12])> {
        let mut out = Vec::new();
        for (_, db, ende) in nal_einheiten(frame) {
            let typ = frame[db] & 0x1f;
            let inhalt = &frame[db + 1..ende];
            if nal_bleibt_klar(typ) || inhalt.is_empty() {
                continue;
            }
            let roh = epb_raus(inhalt);
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&roh[1..9]);
            let c = u32::from_be_bytes([roh[9], roh[10], roh[11], roh[12]]);
            let nr = u16::from_be_bytes([roh[13], roh[14]]);
            out.push((
                derive_key(&k.roh, Spur::Bild, &salt).to_vec(),
                make_nonce(Spur::Bild, nr, &salt, c),
            ));
        }
        out
    }

    #[test]
    fn nonce_never_repeats_across_senders_streams_and_rejoin() {
        let k = Schluessel::neu();
        let rahmen = beispielrahmen();
        let mut gesehen: HashSet<(Vec<u8>, [u8; 12])> = HashSet::new();
        let mut nonces: HashSet<[u8; 12]> = HashSet::new();
        let mut n_pairs = 0usize;
        let mut ein = |p: (Vec<u8>, [u8; 12])| {
            n_pairs += 1;
            nonces.insert(p.1);
            assert!(gesehen.insert(p), "(key, nonce) used twice");
        };

        // Two rounds: "first join" and "rejoin" - every stream is created anew.
        for _beitritt in 0..2 {
            // Two participants (same room key), each with microphone,
            // camera AND screen share = 3 streams, 2 of them video.
            for _teilnehmer in 0..2 {
                let mut mikro = audio(&k);
                let mut kamera = video(&k);
                let mut schirm = video(&k);
                for _ in 0..200 {
                    ein(pairs_audio(&k, &mikro.schuetzen(false, b"opus")));
                    for p in pairs_video(&k, &kamera.schuetzen_h264(&rahmen)) {
                        ein(p);
                    }
                    for p in pairs_video(&k, &schirm.schuetzen_h264(&rahmen)) {
                        ein(p);
                    }
                }
            }
        }
        // 2 joins x 2 participants x 200 rounds x (1 audio + 2 NALs camera + 2 NALs screen)
        assert_eq!(n_pairs, 2 * 2 * 200 * 5, "unexpected number of pairs");
        assert_eq!(gesehen.len(), n_pairs);
        // Even the bare nonce (without the key) must not repeat: defence in
        // depth if two streams ever shared a key.
        assert_eq!(nonces.len(), n_pairs, "a nonce repeated across streams");
    }

    /// The actual harm of v1: two senders at counter 0 produced the same
    /// keystream, so ciphertext XOR = plaintext XOR. Must not happen now.
    #[test]
    fn two_senders_share_no_keystream() {
        let k = Schluessel::neu();
        let p1 = vec![0x11u8; 48];
        let p2 = vec![0x22u8; 48];
        let g1 = audio(&k).schuetzen(false, &p1);
        let g2 = audio(&k).schuetzen(false, &p2);
        assert_eq!(zaehler_von(&g1), zaehler_von(&g2), "both start at counter 0");
        let xor_klar: Vec<u8> = p1.iter().zip(&p2).map(|(a, b)| a ^ b).collect();
        let xor_geheim: Vec<u8> = g1[KOPF..KOPF + 48]
            .iter()
            .zip(&g2[KOPF..KOPF + 48])
            .map(|(a, b)| a ^ b)
            .collect();
        assert_ne!(xor_klar, xor_geheim, "keystream shared between senders");
    }

    #[test]
    fn every_sender_draws_its_own_salt() {
        let k = Schluessel::neu();
        let salze: HashSet<Salt> = (0..64).map(|_| audio(&k).salz()).collect();
        assert_eq!(salze.len(), 64, "salt collision");
    }

    /// A sender must not wrap its 32 bit counter: just before it would, it
    /// switches to a new salt with counter 0.
    #[test]
    fn counter_end_triggers_a_new_salt() {
        let k = Schluessel::neu();
        let mut s = audio(&k);
        s.set_next(u32::MAX as u64);
        let last_frame = s.schuetzen(false, b"last");
        assert_eq!(zaehler_von(&last_frame), Some(u32::MAX));
        let salt_before = salz_von(&last_frame).unwrap();
        let next_frame = s.schuetzen(false, b"next");
        assert_eq!(zaehler_von(&next_frame), Some(0), "counter wrapped without a new salt");
        assert_ne!(salz_von(&next_frame).unwrap(), salt_before, "same salt after exhaustion");
        assert_eq!(k.oeffnen(Spur::Ton, &next_frame).as_deref(), Some(&b"next"[..]));
        // And the receiving guard treats the new salt as a new stream.
        let mut w = Wache::default();
        assert!(w.pruefen(&salt_before, u32::MAX));
        assert!(w.pruefen(&salz_von(&next_frame).unwrap(), 0));
    }

    /// A receiver must not remember keys of frames that did not authenticate
    /// (random salts must not be able to flush the cache).
    #[test]
    fn forged_salts_do_not_fill_the_cache() {
        let k = Schluessel::neu();
        let genuine = audio(&k).schuetzen(false, b"genuine");
        let count = |k: &Schluessel| k.cache.lock().unwrap().len();
        let receiver = Schluessel::aus_roh(k.roh);
        assert_eq!(count(&receiver), 0);
        for i in 0..200u32 {
            let mut f = genuine.clone();
            f[2..6].copy_from_slice(&i.to_be_bytes()); // forged salt
            assert!(receiver.oeffnen(Spur::Ton, &f).is_none());
        }
        assert_eq!(count(&receiver), 0, "unauthenticated salts were cached");
        assert!(receiver.oeffnen(Spur::Ton, &genuine).is_some());
        assert_eq!(count(&receiver), 1);
    }

    // ------------------------------------------------ shared vectors -----

    fn vektoren() -> serde_json::Value {
        serde_json::from_str(include_str!("../tests/freemeet-e2e-vectors.json"))
            .expect("vector file is valid JSON")
    }

    fn salz_hex(s: &str) -> Salt {
        let mut a = [0u8; SALT_LEN];
        a.copy_from_slice(&hex::decode(s).unwrap());
        a
    }

    fn spur_von(s: &str) -> Spur {
        match s {
            "ton" => Spur::Ton,
            "bild" => Spur::Bild,
            other => panic!("unknown track kind {}", other),
        }
    }

    fn vector_key(v: &serde_json::Value) -> Schluessel {
        let mut roh = [0u8; 32];
        roh.copy_from_slice(&hex::decode(v["root_hex"].as_str().unwrap()).unwrap());
        Schluessel::aus_roh(roh)
    }

    /// Key derivation and nonce layout match the Python reference byte for byte.
    #[test]
    fn vectors_keys_and_nonces() {
        let v = vektoren();
        let k = vector_key(&v);
        for e in v["keys"].as_array().unwrap() {
            let got = derive_key(&k.roh, spur_von(e["spur"].as_str().unwrap()), &salz_hex(e["salt"].as_str().unwrap()));
            assert_eq!(hex::encode(got), e["key"].as_str().unwrap(), "key {}", e);
        }
        for e in v["nonces"].as_array().unwrap() {
            let got = make_nonce(
                spur_von(e["spur"].as_str().unwrap()),
                e["nal"].as_u64().unwrap() as u16,
                &salz_hex(e["salt"].as_str().unwrap()),
                e["counter"].as_u64().unwrap() as u32,
            );
            assert_eq!(hex::encode(got), e["nonce"].as_str().unwrap(), "nonce {}", e);
        }
    }

    /// Sealing reproduces the reference frames exactly, and opening the
    /// reference frames returns the plaintext.
    #[test]
    fn vectors_audio_and_video() {
        let v = vektoren();
        let k = vector_key(&v);
        for e in v["audio"].as_array().unwrap() {
            let plain = hex::decode(e["plain"].as_str().unwrap()).unwrap();
            let frame = hex::decode(e["frame"].as_str().unwrap()).unwrap();
            let got = k.seal_audio_at(
                Spur::Ton,
                &salz_hex(e["salt"].as_str().unwrap()),
                e["counter"].as_u64().unwrap() as u32,
                e["keyframe"].as_bool().unwrap(),
                &plain,
            );
            assert_eq!(got, frame, "audio vector {}", e["counter"]);
            // A fresh receiver (empty key cache) opens the reference frame.
            let rx = vector_key(&v);
            assert_eq!(rx.oeffnen(Spur::Ton, &frame), Some(plain));
        }
        for e in v["h264"].as_array().unwrap() {
            let plain = hex::decode(e["plain"].as_str().unwrap()).unwrap();
            let frame = hex::decode(e["frame"].as_str().unwrap()).unwrap();
            let got = k.seal_h264_at(
                &salz_hex(e["salt"].as_str().unwrap()),
                e["counter"].as_u64().unwrap() as u32,
                &plain,
            );
            assert_eq!(got, frame, "h264 vector {}", e["counter"]);
            let rx = vector_key(&v);
            assert_eq!(rx.oeffnen_h264(&frame), Some(plain));
        }
    }

    #[test]
    fn sample_frame_matches_the_reference() {
        let v = vektoren();
        let ref_plain = hex::decode(v["h264"][0]["plain"].as_str().unwrap()).unwrap();
        assert_eq!(beispielrahmen(), ref_plain, "sample frame drifted from ref.py");
    }
}
