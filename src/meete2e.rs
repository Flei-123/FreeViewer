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
pub const TAG: u8 = 0xE3;

/// Blob byte 0 of an encrypted slice NAL unit (type 1/5) by the slice's
/// pic_parameter_set_id (r82). A receiving WebRTC stack parses
/// first_mb_in_slice, slice_type and pps_id (Exp-Golomb) from the first bytes
/// of each slice and drops frames whose pps_id has no PPS. The byte is built
/// to read 0, 0, pps_id: bits '1' '1' ue(pps_id) + padding. Index = pps_id;
/// 0 is the classic `TAG`. Other pps_ids (> 6) keep `TAG`.
const VIDEO_TAG: [u8; 7] = [0xE3, 0xD3, 0xDB, 0xC9, 0xCB, 0xCD, 0xCF];

/// Does this blob byte 0 belong to a v2 video blob?
fn is_video_tag(b: u8) -> bool {
    VIDEO_TAG.contains(&b)
}

/// Exp-Golomb ue(v) at a bit offset: (value, next bit).
fn lese_ue(d: &[u8], mut bit: usize) -> Option<(u32, usize)> {
    let mut zero = 0u32;
    loop {
        if bit >= d.len() * 8 || zero > 16 {
            return None;
        }
        if (d[bit >> 3] >> (7 - (bit & 7))) & 1 == 1 {
            break;
        }
        zero += 1;
        bit += 1;
    }
    bit += 1;
    let mut wert = 1u32;
    for _ in 0..zero {
        if bit >= d.len() * 8 {
            return None;
        }
        wert = (wert << 1) | ((d[bit >> 3] >> (7 - (bit & 7))) & 1) as u32;
        bit += 1;
    }
    Some((wert - 1, bit))
}

/// pic_parameter_set_id of a slice NAL payload (after the NAL header byte).
fn slice_pps_id(content: &[u8]) -> Option<u32> {
    let head = &content[..content.len().min(8)];
    let (_, b) = lese_ue(head, 0)?;
    let (_, b) = lese_ue(head, b)?;
    lese_ue(head, b).map(|(pps, _)| pps)
}

/// Blob byte 0 of one encrypted NAL unit.
fn video_tag(nal_typ: u8, content: &[u8]) -> u8 {
    if nal_typ == 1 || nal_typ == 5 {
        if let Some(pps) = slice_pps_id(content) {
            if let Some(b) = VIDEO_TAG.get(pps as usize) {
                return *b;
            }
        }
    }
    TAG
}

/// Frame format version announced to the server in the join (`caps.e2e_ver`).
pub const FORMAT_VERSION: u8 = 2;
/// First byte of v1 audio frames (the vulnerable format). Never accepted.
pub const MARKER_V1: u8 = 0xE2;
/// Length of the clear audio header: version, flags, salt, counter.
pub const HEAD: usize = 14;
/// Length of the sender salt.
pub const SALT_LEN: usize = 8;
/// Length of the per-NAL blob header: version, salt, counter, NAL index.
const NAL_HEAD: usize = 1 + SALT_LEN + 4 + 2;
/// Flags byte, bit 0.
const MARKER_KEYFRAME: u8 = 0x01;
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
    Image,
}

impl Spur {
    fn id(self) -> u8 {
        match self {
            Spur::Ton => 1,
            Spur::Image => 2,
        }
    }
    fn info(self) -> &'static [u8] {
        match self {
            Spur::Ton => b"ton",
            Spur::Image => b"bild",
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
pub struct Key {
    roh: [u8; 32],
    cache: Arc<Mutex<HashMap<(Spur, Salt), Aes256Gcm>>>,
}

impl Key {
    /// From 32 random bytes (the root secret of the room).
    pub fn aus_roh(roh: [u8; 32]) -> Key {
        Key {
            roh,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Roll a fresh random key (when the meeting is created).
    pub fn neu() -> Key {
        use rand::RngCore;
        let mut roh = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut roh);
        Key::aus_roh(roh)
    }

    /// For the link: 43 characters of URL-safe base64 without padding.
    pub fn als_text(&self) -> String {
        base64_url(&self.roh)
    }

    /// Back from the link. None = not a valid key.
    pub fn aus_text(t: &str) -> Option<Key> {
        let roh = base64_url_roundtrip(t.trim())?;
        if roh.len() != 32 {
            return None;
        }
        let mut a = [0u8; 32];
        a.copy_from_slice(&roh);
        Some(Key::aus_roh(a))
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
        self.sender_with_salt(spur, random_salt())
    }

    /// Like `sender`, with a chosen salt (tests only).
    pub fn sender_with_salt(&self, spur: Spur, salt: Salt) -> Sender {
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
        keyframe: bool,
        cleartext: &[u8],
    ) -> Vec<u8> {
        let mut head = [0u8; HEAD];
        head[0] = TAG;
        head[1] = if keyframe { MARKER_KEYFRAME } else { 0 };
        head[2..2 + SALT_LEN].copy_from_slice(salt);
        head[2 + SALT_LEN..HEAD].copy_from_slice(&counter.to_be_bytes());
        let nonce = make_nonce(spur, 0, salt, counter);
        let secret = self
            .cipher(spur, salt)
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: cleartext,
                    aad: &head,
                },
            )
            .unwrap_or_default();
        let mut aus = Vec::with_capacity(HEAD + secret.len());
        aus.extend_from_slice(&head);
        aus.extend_from_slice(&secret);
        aus
    }

    /// Decrypt one audio frame. None = not our format (also: v1), wrong key
    /// or tampered - in every case do NOT show it.
    pub fn open(&self, spur: Spur, frame_box: &[u8]) -> Option<Vec<u8>> {
        if !is_protected(frame_box) {
            return None;
        }
        let head = &frame_box[..HEAD];
        let salt = salt_from(frame_box)?;
        let counter = counter_from(frame_box)?;
        let nonce = make_nonce(spur, 0, &salt, counter);
        // The salt is attacker-controlled until the tag checks out, so a
        // stream key is only remembered after a successful decryption.
        let (cipher, was_cached) = match self.cached(spur, &salt) {
            Some(c) => (c, true),
            None => (self.derive_cipher(spur, &salt), false),
        };
        let cleartext = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &frame_box[HEAD..],
                    aad: head,
                },
            )
            .ok()?;
        if !was_cached {
            self.remember(spur, &salt, &cipher);
        }
        Some(cleartext)
    }
}

/// One outgoing stream: owns the sender salt and the counter, so a
/// (key, nonce) pair can never repeat.
pub struct Sender {
    key: Key,
    spur: Spur,
    salt: Salt,
    /// Next counter value. 64 bit so exhaustion is detectable without wrap.
    next: u64,
}

impl Sender {
    /// Current sender salt (it travels in every frame).
    pub fn nonce_salt(&self) -> Salt {
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
    pub fn protect(&mut self, keyframe: bool, cleartext: &[u8]) -> Vec<u8> {
        debug_assert_eq!(self.spur, Spur::Ton, "audio frames need an audio sender");
        let (salt, c) = self.take();
        self.key
            .seal_audio_at(self.spur, &salt, c, keyframe, cleartext)
    }

    /// Encrypt one H.264 frame (structure stays readable).
    pub fn protect_h264(&mut self, frame_box: &[u8]) -> Vec<u8> {
        debug_assert_eq!(self.spur, Spur::Image, "video frames need a video sender");
        let (salt, c) = self.take();
        self.key.seal_h264_at(&salt, c, frame_box)
    }

    /// Test hook: jump the counter (to test the rotation at the end).
    #[cfg(test)]
    fn set_next(&mut self, n: u64) {
        self.next = n;
    }
}

// ------------------------------------------------------- H.264 (video) ----
//
// WHY video is treated differently from audio:
//
// The media server is NOT a pure forwarder. It receives unpacked frames
// and reassembles them into RTP packets - for that it MUST understand the structure
// of the picture (NAL units, STAP-A). Measured on 07.08.2026:
// if the whole frame is encrypted, it fails with
// "NaluTypeIsNotHandled" and no video arrives at the other end.
//
// That is why the STRUCTURE stays visible and only the CONTENT is encrypted:
//   * start codes and the header byte of each NAL unit stay in clear text,
//   * SPS (7), PPS (8) and AUD (9) stay completely untouched - they contain
//     only how large the picture is, no picture content; the packer needs them,
//   * everything else is encrypted.
//
// SFrame makes the same compromise for H.264: the server sees the
// frame structure and the sizes, but NO content.
//
// Additionally needed: the ciphertext may randomly contain "00 00 01" -
// that would be a false start code and would tear the picture apart. Therefore
// the same protection is used that H.264 itself uses (a 0x03
// in between, "emulation prevention").

/// Where does the next NAL unit start? Returns (start of the start code,
/// start of the data).
fn next_start_code(daten: &[u8], ab: usize) -> Option<(usize, usize)> {
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
pub fn nal_units(daten: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut aus = Vec::new();
    let mut search = 0usize;
    let mut offen: Option<(usize, usize)> = None;
    while let Some((sc, db)) = next_start_code(daten, search) {
        if let Some((osc, odb)) = offen {
            aus.push((osc, odb, sc));
        }
        offen = Some((sc, db));
        search = db;
    }
    if let Some((osc, odb)) = offen {
        aus.push((osc, odb, daten.len()));
    }
    aus
}

/// Must this NAL unit stay in clear text?
///
/// SPS/PPS describe only size and encoding parameters - no picture content -
/// and the packer in the server needs them. AUD is a pure separator.
fn nal_stays_clear(typ: u8) -> bool {
    matches!(typ, 7 | 8 | 9)
}

/// Den Schutz gegen falsche Startcodes einfuegen (wie H.264 selbst).
fn epb_ein(daten: &[u8]) -> Vec<u8> {
    let mut aus = Vec::with_capacity(daten.len() + daten.len() / 64 + 4);
    let mut zero = 0usize;
    for &b in daten {
        if zero >= 2 && b <= 3 {
            aus.push(3);
            zero = 0;
        }
        aus.push(b);
        if b == 0 {
            zero += 1;
        } else {
            zero = 0;
        }
    }
    aus
}

/// Take the protection out again.
fn epb_out(daten: &[u8]) -> Vec<u8> {
    let mut aus = Vec::with_capacity(daten.len());
    let mut zero = 0usize;
    let mut i = 0usize;
    while i < daten.len() {
        let b = daten[i];
        if zero >= 2 && b == 3 && i + 1 < daten.len() && daten[i + 1] <= 3 {
            zero = 0;
            i += 1;
            continue;
        }
        aus.push(b);
        if b == 0 {
            zero += 1;
        } else {
            zero = 0;
        }
        i += 1;
    }
    aus
}

// Per-NAL blob (inside the emulation-prevention escaping), format v2:
//   byte 0       version: 0xE3, or for slice NAL units the pps_id byte of
//                VIDEO_TAG (so a receiving WebRTC stack finds a PPS)
//   byte 1..9    sender salt
//   byte 9..13   counter (32 bit, big endian)
//   byte 13..15  NAL index inside the frame (16 bit, big endian)
//   from 15      AES-256-GCM: ciphertext + 16 byte tag
// Associated data: NAL header byte + the 15 byte blob header.

impl Key {
    /// Encrypt one H.264 frame with an explicit salt + counter. LOW LEVEL
    /// like `seal_audio_at`: use a `Sender`.
    pub fn seal_h264_at(&self, salt: &Salt, counter: u32, frame_box: &[u8]) -> Vec<u8> {
        let parts = nal_units(frame_box);
        if parts.is_empty() {
            return frame_box.to_vec(); // not Annex B: leave untouched
        }
        if parts.len() > u16::MAX as usize {
            // The NAL index (part of the nonce) would repeat. Never happens
            // with a real encoder; send nothing rather than reuse a nonce.
            return Vec::new();
        }
        let cipher = self.cipher(Spur::Image, salt);
        let mut aus = Vec::with_capacity(frame_box.len() + parts.len() * (NAL_HEAD + 16 + 2));
        for (nr, (sc, db, end)) in parts.iter().enumerate() {
            let head_byte = frame_box[*db];
            let typ = head_byte & 0x1f;
            // Start code and NAL header byte always stay in the clear.
            aus.extend_from_slice(&frame_box[*sc..*db + 1]);
            let content = &frame_box[*db + 1..*end];
            if nal_stays_clear(typ) || content.is_empty() {
                aus.extend_from_slice(content);
                continue;
            }
            let nr16 = nr as u16;
            let mut blob = Vec::with_capacity(NAL_HEAD + content.len() + 16);
            blob.push(video_tag(typ, content));
            blob.extend_from_slice(salt);
            blob.extend_from_slice(&counter.to_be_bytes());
            blob.extend_from_slice(&nr16.to_be_bytes());
            // NAL header byte + blob header are authenticated: swapping the
            // counter, salt or NAL index invalidates the frame.
            let mut aad = Vec::with_capacity(1 + NAL_HEAD);
            aad.push(head_byte);
            aad.extend_from_slice(&blob);
            let nonce = make_nonce(Spur::Image, nr16, salt, counter);
            let secret = cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: content,
                        aad: &aad,
                    },
                )
                .unwrap_or_default();
            blob.extend_from_slice(&secret);
            aus.extend_from_slice(&epb_ein(&blob));
        }
        aus
    }

    /// Open an encrypted H.264 frame. None = wrong key, tampered, or an old
    /// (v1) / foreign format. Does NOT guard against replays: use
    /// `open_h264_guard` in a receiver.
    pub fn open_h264(&self, frame_box: &[u8]) -> Option<Vec<u8>> {
        self.open_h264_head(frame_box).map(|(cleartext, _)| cleartext)
    }

    /// Open an encrypted H.264 frame AND reject replays (r60). The check runs
    /// only after the tags verified, so forged frames cannot move the window.
    /// A frame without any encrypted NAL unit (only SPS/PPS/AUD) has no
    /// counter and is passed on.
    pub fn open_h264_guard(&self, guard: &mut Guard, frame_box: &[u8]) -> Option<Vec<u8>> {
        let (cleartext, head) = self.open_h264_head(frame_box)?;
        match head {
            Some((nonce_salt, ctr)) if !guard.check(&nonce_salt, ctr) => None,
            _ => Some(cleartext),
        }
    }

    /// Like `open_h264`, plus the (salt, counter) every encrypted NAL unit
    /// of the frame carries. Frame-level rules on top of the per-NAL tags:
    /// all encrypted NAL units must name the SAME salt+counter, and their NAL
    /// indices must strictly increase. Otherwise NAL units of different frames
    /// could be spliced (each tag is valid on its own), repeated or reordered.
    pub fn open_h264_head(&self, frame_box: &[u8]) -> Option<(Vec<u8>, Option<(Salt, u32)>)> {
        let parts = nal_units(frame_box);
        if parts.is_empty() {
            return None;
        }
        let mut aus = Vec::with_capacity(frame_box.len());
        let mut frame_head: Option<(Salt, u32)> = None;
        let mut last_nr: Option<u16> = None;
        for (sc, db, end) in parts {
            let head_byte = frame_box[db];
            let typ = head_byte & 0x1f;
            aus.extend_from_slice(&frame_box[sc..db + 1]);
            let content = &frame_box[db + 1..end];
            if nal_stays_clear(typ) || content.is_empty() {
                aus.extend_from_slice(content);
                continue;
            }
            let roh = epb_out(content);
            if roh.len() < NAL_HEAD + 16 || !is_video_tag(roh[0]) {
                return None;
            }
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&roh[1..1 + SALT_LEN]);
            let o = 1 + SALT_LEN;
            let counter = u32::from_be_bytes([roh[o], roh[o + 1], roh[o + 2], roh[o + 3]]);
            let nr16 = u16::from_be_bytes([roh[o + 4], roh[o + 5]]);
            match frame_head {
                Some(k) if k != (salt, counter) => return None, // spliced from two frames
                _ => frame_head = Some((salt, counter)),
            }
            if last_nr.is_some_and(|l| nr16 <= l) {
                return None; // repeated or reordered NAL unit
            }
            last_nr = Some(nr16);
            let mut aad = Vec::with_capacity(1 + NAL_HEAD);
            aad.push(head_byte);
            aad.extend_from_slice(&roh[..NAL_HEAD]);
            let nonce = make_nonce(Spur::Image, nr16, &salt, counter);
            let (cipher, was_cached) = match self.cached(Spur::Image, &salt) {
                Some(c) => (c, true),
                None => (self.derive_cipher(Spur::Image, &salt), false),
            };
            let cleartext = cipher
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &roh[NAL_HEAD..],
                        aad: &aad,
                    },
                )
                .ok()?;
            if !was_cached {
                self.remember(Spur::Image, &salt, &cipher);
            }
            aus.extend_from_slice(&cleartext);
        }
        Some((aus, frame_head))
    }
}

/// Does this look like a protected (v2) audio frame? (The server asks too.)
pub fn is_protected(frame_box: &[u8]) -> bool {
    frame_box.len() > HEAD + 16 && frame_box[0] == TAG
}

/// Is this an audio frame of the OLD, vulnerable format (v1)? Such a
/// participant has to update; their frames are never opened.
pub fn is_old_format(frame_box: &[u8]) -> bool {
    !frame_box.is_empty() && frame_box[0] == MARKER_V1
}

/// Does it contain a key frame? The server may read this - it needs it to
/// switch quality layers.
pub fn is_keyframe_data(frame_box: &[u8]) -> Option<bool> {
    if !is_protected(frame_box) {
        return None;
    }
    Some(frame_box[1] & MARKER_KEYFRAME != 0)
}

/// Counter of an audio frame.
pub fn counter_from(frame_box: &[u8]) -> Option<u32> {
    if !is_protected(frame_box) {
        return None;
    }
    let o = 2 + SALT_LEN;
    Some(u32::from_be_bytes([
        frame_box[o],
        frame_box[o + 1],
        frame_box[o + 2],
        frame_box[o + 3],
    ]))
}

/// Sender salt of an audio frame.
pub fn salt_from(frame_box: &[u8]) -> Option<Salt> {
    if !is_protected(frame_box) {
        return None;
    }
    let mut s = [0u8; SALT_LEN];
    s.copy_from_slice(&frame_box[2..2 + SALT_LEN]);
    Some(s)
}

/// Replay window (64 numbers below the highest) of ONE sender salt.
#[derive(Default)]
struct Window {
    highest: u32,
    /// Bit field of the last 64 numbers below `highest` (the network may reorder).
    window: u64,
}

impl Window {
    fn check(&mut self, ctr: u32) -> bool {
        if ctr > self.highest {
            let step = ctr - self.highest;
            // The old highest becomes bit (schritt - 1). With a step of
            // exactly 64 that is bit 63 and the old bits fall out; a larger
            // step forgets everything.
            self.window = match step {
                1..=63 => (self.window << step) | (1u64 << (step - 1)),
                64 => 1u64 << 63,
                _ => 0,
            };
            self.highest = ctr;
            return true;
        }
        let go_back = self.highest - ctr;
        if go_back == 0 || go_back > 64 {
            return false; // exactly the highest, or long out of the window
        }
        let bit = 1u64 << (go_back - 1);
        if self.window & bit != 0 {
            return false;
        }
        self.window |= bit;
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
pub struct Guard {
    window: Vec<(Salt, Window)>,
}

impl Guard {
    /// True = new and fine. False = repeat or too old.
    pub fn check(&mut self, nonce_salt: &Salt, ctr: u32) -> bool {
        if let Some((_, f)) = self.window.iter_mut().find(|(s, _)| s == nonce_salt) {
            return f.check(ctr);
        }
        if self.window.len() >= GUARD_SALTS_MAX {
            self.window.remove(0);
        }
        let f = Window {
            highest: ctr,
            window: 0,
        };
        self.window.push((*nonce_salt, f));
        true
    }
}

// ------------------------------------------------------------- Base64 -----
// URL-safe, without padding. Self-computed, so that no further
// dependency is needed.

const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn base64_url(daten: &[u8]) -> String {
    let mut aus = String::new();
    for piece in daten.chunks(3) {
        let b = [
            piece[0],
            *piece.get(1).unwrap_or(&0),
            *piece.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        let glyphs = piece.len() + 1;
        for i in 0..glyphs {
            aus.push(ABC[((n >> (18 - i * 6)) & 0x3f) as usize] as char);
        }
    }
    aus
}

pub fn base64_url_roundtrip(text: &str) -> Option<Vec<u8>> {
    let wert = |c: u8| -> Option<u32> { ABC.iter().position(|x| *x == c).map(|p| p as u32) };
    let roh: Vec<u8> = text.bytes().filter(|c| *c != b'=').collect();
    let mut aus = Vec::new();
    for piece in roh.chunks(4) {
        if piece.len() < 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, c) in piece.iter().enumerate() {
            n |= wert(*c)? << (18 - i * 6);
        }
        for i in 0..piece.len() - 1 {
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
    fn example_frame() -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&[0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f, 0x96, 0x54]); // SPS (7)
        f.extend_from_slice(&[0, 0, 0, 1, 0x68, 0xce, 0x3c, 0x80]); // PPS (8)
        f.extend_from_slice(&[0, 0, 0, 1, 0x65]); // IDR (5)
        f.extend((0..400u32).map(|i| (i * 13 % 256) as u8));
        f.extend_from_slice(&[0, 0, 1, 0x41]); // non-IDR (1), 3-byte start code
        f.extend((0..250u32).map(|i| (i * 29 % 256) as u8));
        f
    }

    fn video(k: &Key) -> Sender {
        k.sender(Spur::Image)
    }
    fn audio(k: &Key) -> Sender {
        k.sender(Spur::Ton)
    }

    // ------------------------------------------------------ H.264 --------

    #[test]
    fn h264_round_trip() {
        let k = Key::neu();
        let f = example_frame();
        let g = video(&k).protect_h264(&f);
        assert_ne!(g, f, "nothing encrypted");
        assert_eq!(k.open_h264(&g).as_deref(), Some(&f[..]), "does not come back intact");
    }

    /// THE point where the first attempt failed: the server must still be
    /// able to read the structure.
    #[test]
    fn the_structure_stays_readable() {
        let k = Key::neu();
        let f = example_frame();
        let g = video(&k).protect_h264(&f);
        let before: Vec<u8> = nal_units(&f).iter().map(|(_, d, _)| f[*d] & 0x1f).collect();
        let after: Vec<u8> = nal_units(&g).iter().map(|(_, d, _)| g[*d] & 0x1f).collect();
        assert_eq!(before, after, "NAL types changed");
        assert_eq!(before, vec![7, 8, 5, 1]);
    }

    /// SPS and PPS must stay identical - the packer in the server reads them.
    #[test]
    fn sps_and_pps_stay_untouched() {
        let k = Key::neu();
        let f = example_frame();
        let g = video(&k).protect_h264(&f);
        for (sc, db, end) in nal_units(&f) {
            let typ = f[db] & 0x1f;
            if typ == 7 || typ == 8 {
                let piece = &f[sc..end];
                assert!(
                    g.windows(piece.len()).any(|w| w == piece),
                    "NAL type {} was changed",
                    typ
                );
            }
        }
    }

    /// The picture content must NOT be in the frame any more.
    #[test]
    fn h264_content_is_really_gone() {
        let k = Key::neu();
        let mut f = vec![0, 0, 0, 1, 0x65];
        let secret: Vec<u8> = b"SECRETPICTURECONTENT".repeat(6);
        f.extend_from_slice(&secret);
        let g = video(&k).protect_h264(&f);
        assert!(
            !g.windows(secret.len()).any(|w| w == &secret[..]),
            "the picture content lies open in the frame"
        );
    }

    /// False start codes would tear the picture apart - never, not even if
    /// the ciphertext happens to contain 00 00 01.
    #[test]
    fn no_false_start_codes_appear() {
        let k = Key::neu();
        let mut s = video(&k);
        for runde in 0..300u32 {
            let mut f = vec![0, 0, 0, 1, 0x41];
            f.extend((0..64u32).map(|i| ((i * runde + 7) % 256) as u8));
            let g = s.protect_h264(&f);
            assert_eq!(nal_units(&g).len(), 1, "round {}: false start codes", runde);
            assert_eq!(k.open_h264(&g).as_deref(), Some(&f[..]), "round {}", runde);
        }
    }

    #[test]
    fn h264_foreign_key_opens_nothing() {
        let a = Key::neu();
        let b = Key::neu();
        let g = video(&a).protect_h264(&example_frame());
        assert!(b.open_h264(&g).is_none());
    }

    #[test]
    fn h264_tampering_is_detected() {
        let k = Key::neu();
        let g = video(&k).protect_h264(&example_frame());
        let mut broken = g.clone();
        let mitte = g.len() - 40;
        broken[mitte] ^= 0x01;
        assert!(k.open_h264(&broken).is_none(), "tampering went unnoticed");
    }

    /// The key frame stays recognisable at NAL type 5.
    #[test]
    fn keyframe_stays_recognizable() {
        let k = Key::neu();
        let g = video(&k).protect_h264(&example_frame());
        let hat_idr = nal_units(&g).iter().any(|(_, d, _)| g[*d] & 0x1f == 5);
        assert!(hat_idr, "the server can no longer find the key frame");
    }

    #[test]
    fn protection_against_false_start_codes_is_reversible() {
        for muster in [
            vec![0u8, 0, 1, 2, 3],
            vec![0, 0, 0, 0, 0, 0],
            vec![0, 0, 3, 0, 0, 1],
            vec![1, 2, 3, 4, 5],
            vec![0, 0, 2],
        ] {
            let ein = epb_ein(&muster);
            assert_eq!(epb_out(&ein), muster, "pattern {:?}", muster);
        }
    }

    /// An old (v1) video blob starts with the NAL index, not with 0xE3.
    /// A v2 receiver must reject it instead of guessing.
    #[test]
    fn old_video_format_is_rejected() {
        let k = Key::neu();
        let mut f = vec![0, 0, 0, 1, 0x65];
        f.push(0); // v1: NAL index 0
        f.extend_from_slice(&[0, 0, 0, 5]); // v1: counter
        f.extend((0..40u8).map(|i| i.wrapping_mul(7).wrapping_add(9)));
        assert!(k.open_h264(&f).is_none());
    }

    // ------------------------------------------------------- audio -------

    #[test]
    fn round_trip() {
        let k = Key::neu();
        let cleartext = b"Hello, this is a frame.";
        let g = audio(&k).protect(true, cleartext);
        assert!(is_protected(&g));
        assert_eq!(is_keyframe_data(&g), Some(true));
        assert_eq!(counter_from(&g), Some(0));
        assert_eq!(k.open(Spur::Ton, &g).as_deref(), Some(&cleartext[..]));
    }

    /// The core: the plaintext must NOT be in the frame.
    #[test]
    fn plaintext_is_not_inside() {
        let k = Key::neu();
        let cleartext = b"SECRETWORD";
        let g = audio(&k).protect(false, cleartext);
        assert!(g.windows(cleartext.len()).all(|f| f != cleartext), "plaintext lies open");
    }

    /// Another key must open NOTHING - that is the whole point.
    #[test]
    fn foreign_key_opens_nothing() {
        let a = Key::neu();
        let b = Key::neu();
        let g = audio(&a).protect(false, b"content");
        assert!(b.open(Spur::Ton, &g).is_none());
    }

    /// Audio and video keys are separate: an audio frame never passes as video.
    #[test]
    fn tracks_are_separated() {
        let k = Key::neu();
        let g = audio(&k).protect(false, b"sound");
        assert!(k.open(Spur::Image, &g).is_none());
        assert!(k.open(Spur::Ton, &g).is_some());
    }

    /// Every change to the frame must be noticed - also in the CLEAR header
    /// (flags, salt, counter), or the server could forge the key frame flag
    /// or splice a frame onto another stream.
    #[test]
    fn any_tampering_is_detected() {
        let k = Key::neu();
        let g = audio(&k).protect(true, b"A longer content to check");
        for place in [1usize, 2, 9, 10, 13, HEAD, HEAD + 3, g.len() - 1] {
            let mut broken = g.clone();
            broken[place] ^= 0x01;
            assert!(
                k.open(Spur::Ton, &broken).is_none(),
                "change at position {} went unnoticed",
                place
            );
        }
    }

    #[test]
    fn unencrypted_frames_are_left_alone() {
        let k = Key::neu();
        // An unencrypted H.264 frame starts with 00 00 00 01.
        let alt = [0u8, 0, 0, 1, 0x65, 0x88, 0x84, 0x21, 0x00, 0x11, 0x22, 0x33,
                   0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
                   0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert!(!is_protected(&alt));
        assert_eq!(is_keyframe_data(&alt), None);
        assert!(k.open(Spur::Image, &alt).is_none());
    }

    /// v1 audio frames (marker 0xE2, 6 byte header) are recognised as OLD
    /// and never opened.
    #[test]
    fn old_audio_format_is_detected_and_rejected() {
        let k = Key::neu();
        let mut v1 = vec![MARKER_V1, 0x01, 0, 0, 0, 7];
        v1.extend((0..40u8).map(|i| i ^ 0x5a));
        assert!(is_old_format(&v1));
        assert!(!is_protected(&v1));
        assert!(k.open(Spur::Ton, &v1).is_none());
        let neu = audio(&k).protect(false, b"x");
        assert!(!is_old_format(&neu));
    }

    #[test]
    fn key_survives_the_link() {
        let k = Key::neu();
        let t = k.als_text();
        assert_eq!(t.len(), 43, "43 characters expected, got {}", t.len());
        assert!(!t.contains('+') && !t.contains('/') && !t.contains('='), "not link safe: {}", t);
        let go_back = Key::aus_text(&t).expect("must come back");
        let g = audio(&k).protect(false, b"probe");
        assert_eq!(go_back.open(Spur::Ton, &g).as_deref(), Some(&b"probe"[..]));
    }

    #[test]
    fn nonsense_key_text_is_rejected() {
        assert!(Key::aus_text("").is_none());
        assert!(Key::aus_text("zu-kurz").is_none());
        assert!(Key::aus_text(&"A".repeat(43)).is_some());
        assert!(Key::aus_text("!!!!").is_none());
    }

    #[test]
    fn base64_haelt_beliebige_daten_aus() {
        for length in [1usize, 2, 3, 4, 31, 32, 33, 100] {
            let daten: Vec<u8> = (0..length).map(|i| (i * 7 % 256) as u8).collect();
            let t = base64_url(&daten);
            assert_eq!(base64_url_roundtrip(&t).as_deref(), Some(&daten[..]), "length {}", length);
        }
    }

    /// Header is 14 bytes: the overhead per frame is 14 + 16 tag bytes.
    #[test]
    fn the_overhead_stays_small() {
        let k = Key::neu();
        let cleartext = vec![0u8; 5000];
        let g = k.sender(Spur::Ton).protect(false, &cleartext);
        assert_eq!(g.len(), cleartext.len() + HEAD + 16, "overhead changed");
    }

    // ------------------------------------------------------ replay -------

    #[test]
    fn guard_lets_new_through_and_repeats_not() {
        let mut w = Guard::default();
        assert!(w.check(&SALT_A, 10));
        assert!(w.check(&SALT_A, 11));
        assert!(!w.check(&SALT_A, 11), "repeat let through");
        assert!(!w.check(&SALT_A, 10), "repeat let through");
        // The network may reorder: delivering a gap later is fine.
        assert!(w.check(&SALT_A, 13));
        assert!(w.check(&SALT_A, 12), "late packet wrongly dropped");
        assert!(!w.check(&SALT_A, 12));
        // 1 is still INSIDE the 64 window at highest = 13: legit late packet.
        assert!(w.check(&SALT_A, 1), "late packet inside the window wrongly dropped");
        assert!(!w.check(&SALT_A, 1), "same one let through twice");
        // Only far outside the window it is refused.
        assert!(w.check(&SALT_A, 200));
        assert!(!w.check(&SALT_A, 13), "ancient frame (187 back) let through");
    }

    #[test]
    fn guard_copes_with_big_jumps() {
        let mut w = Guard::default();
        assert!(w.check(&SALT_A, 1));
        assert!(w.check(&SALT_A, 1_000_000));
        assert!(!w.check(&SALT_A, 500), "ancient frame let through");
        assert!(w.check(&SALT_A, 1_000_001));
    }

    /// Every sender stream counts on its own: counter 5 of sender B is not a
    /// repeat of counter 5 of sender A; but each one only once.
    #[test]
    fn guard_keeps_one_window_per_sender() {
        let mut w = Guard::default();
        assert!(w.check(&SALT_A, 5));
        assert!(w.check(&SALT_B, 5));
        assert!(!w.check(&SALT_A, 5));
        assert!(!w.check(&SALT_B, 5));
        assert!(w.check(&SALT_B, 6));
        assert!(w.check(&SALT_A, 4)); // late, inside A's window
    }

    #[test]
    fn guard_forgets_only_the_oldest_salts() {
        let mut w = Guard::default();
        for i in 0..(GUARD_SALTS_MAX as u8 + 3) {
            let s: Salt = [i, 9, 9, 9, 9, 9, 9, 9];
            assert!(w.check(&s, 1));
            assert!(!w.check(&s, 1));
        }
        assert!(w.window.len() <= GUARD_SALTS_MAX, "guard grows without limit");
    }

    // --------------------------------------------- nonce uniqueness ------
    //
    // The r1 bug: (key, nonce) repeated across senders, across camera and
    // screen share, and across a rejoin. These tests collect the REAL
    // (stream key, GCM nonce) pairs from sealed frames and demand that no
    // pair occurs twice.

    fn pairs_audio(k: &Key, frame: &[u8]) -> (Vec<u8>, [u8; 12]) {
        let salt = salt_from(frame).unwrap();
        let c = counter_from(frame).unwrap();
        (derive_key(&k.roh, Spur::Ton, &salt).to_vec(), make_nonce(Spur::Ton, 0, &salt, c))
    }

    fn pairs_video(k: &Key, frame: &[u8]) -> Vec<(Vec<u8>, [u8; 12])> {
        let mut out = Vec::new();
        for (_, db, end) in nal_units(frame) {
            let typ = frame[db] & 0x1f;
            let content = &frame[db + 1..end];
            if nal_stays_clear(typ) || content.is_empty() {
                continue;
            }
            let roh = epb_out(content);
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&roh[1..9]);
            let c = u32::from_be_bytes([roh[9], roh[10], roh[11], roh[12]]);
            let nr = u16::from_be_bytes([roh[13], roh[14]]);
            out.push((
                derive_key(&k.roh, Spur::Image, &salt).to_vec(),
                make_nonce(Spur::Image, nr, &salt, c),
            ));
        }
        out
    }

    #[test]
    fn nonce_never_repeats_across_senders_streams_and_rejoin() {
        let k = Key::neu();
        let frame_box = example_frame();
        let mut seen: HashSet<(Vec<u8>, [u8; 12])> = HashSet::new();
        let mut nonces: HashSet<[u8; 12]> = HashSet::new();
        let mut n_pairs = 0usize;
        let mut ein = |p: (Vec<u8>, [u8; 12])| {
            n_pairs += 1;
            nonces.insert(p.1);
            assert!(seen.insert(p), "(key, nonce) used twice");
        };

        // Two rounds: "first join" and "rejoin" - every stream is created anew.
        for _join in 0..2 {
            // Two participants (same room key), each with microphone,
            // camera AND screen share = 3 streams, 2 of them video.
            for _participant in 0..2 {
                let mut mic_name = audio(&k);
                let mut camera = video(&k);
                let mut screen = video(&k);
                for _ in 0..200 {
                    ein(pairs_audio(&k, &mic_name.protect(false, b"opus")));
                    for p in pairs_video(&k, &camera.protect_h264(&frame_box)) {
                        ein(p);
                    }
                    for p in pairs_video(&k, &screen.protect_h264(&frame_box)) {
                        ein(p);
                    }
                }
            }
        }
        // 2 joins x 2 participants x 200 rounds x (1 audio + 2 NALs camera + 2 NALs screen)
        assert_eq!(n_pairs, 2 * 2 * 200 * 5, "unexpected number of pairs");
        assert_eq!(seen.len(), n_pairs);
        // Even the bare nonce (without the key) must not repeat: defence in
        // depth if two streams ever shared a key.
        assert_eq!(nonces.len(), n_pairs, "a nonce repeated across streams");
    }

    /// The actual harm of v1: two senders at counter 0 produced the same
    /// keystream, so ciphertext XOR = plaintext XOR. Must not happen now.
    #[test]
    fn two_senders_share_no_keystream() {
        let k = Key::neu();
        let p1 = vec![0x11u8; 48];
        let p2 = vec![0x22u8; 48];
        let g1 = audio(&k).protect(false, &p1);
        let g2 = audio(&k).protect(false, &p2);
        assert_eq!(counter_from(&g1), counter_from(&g2), "both start at counter 0");
        let xor_clear: Vec<u8> = p1.iter().zip(&p2).map(|(a, b)| a ^ b).collect();
        let xor_secret: Vec<u8> = g1[HEAD..HEAD + 48]
            .iter()
            .zip(&g2[HEAD..HEAD + 48])
            .map(|(a, b)| a ^ b)
            .collect();
        assert_ne!(xor_clear, xor_secret, "keystream shared between senders");
    }

    #[test]
    fn every_sender_draws_its_own_salt() {
        let k = Key::neu();
        let salts: HashSet<Salt> = (0..64).map(|_| audio(&k).nonce_salt()).collect();
        assert_eq!(salts.len(), 64, "salt collision");
    }

    /// A sender must not wrap its 32 bit counter: just before it would, it
    /// switches to a new salt with counter 0.
    #[test]
    fn counter_end_triggers_a_new_salt() {
        let k = Key::neu();
        let mut s = audio(&k);
        s.set_next(u32::MAX as u64);
        let last_frame = s.protect(false, b"last");
        assert_eq!(counter_from(&last_frame), Some(u32::MAX));
        let salt_before = salt_from(&last_frame).unwrap();
        let next_frame = s.protect(false, b"next");
        assert_eq!(counter_from(&next_frame), Some(0), "counter wrapped without a new salt");
        assert_ne!(salt_from(&next_frame).unwrap(), salt_before, "same salt after exhaustion");
        assert_eq!(k.open(Spur::Ton, &next_frame).as_deref(), Some(&b"next"[..]));
        // And the receiving guard treats the new salt as a new stream.
        let mut w = Guard::default();
        assert!(w.check(&salt_before, u32::MAX));
        assert!(w.check(&salt_from(&next_frame).unwrap(), 0));
    }

    /// A receiver must not remember keys of frames that did not authenticate
    /// (random salts must not be able to flush the cache).
    #[test]
    fn forged_salts_do_not_fill_the_cache() {
        let k = Key::neu();
        let genuine = audio(&k).protect(false, b"genuine");
        let count = |k: &Key| k.cache.lock().unwrap().len();
        let receiver = Key::aus_roh(k.roh);
        assert_eq!(count(&receiver), 0);
        for i in 0..200u32 {
            let mut f = genuine.clone();
            f[2..6].copy_from_slice(&i.to_be_bytes()); // forged salt
            assert!(receiver.open(Spur::Ton, &f).is_none());
        }
        assert_eq!(count(&receiver), 0, "unauthenticated salts were cached");
        assert!(receiver.open(Spur::Ton, &genuine).is_some());
        assert_eq!(count(&receiver), 1);
    }

    // ------------------------------------------------ shared vectors -----

    fn vektoren() -> serde_json::Value {
        serde_json::from_str(include_str!("../tests/freemeet-e2e-vectors.json"))
            .expect("vector file is valid JSON")
    }

    fn salt_hex(s: &str) -> Salt {
        let mut a = [0u8; SALT_LEN];
        a.copy_from_slice(&hex::decode(s).unwrap());
        a
    }

    fn spur_von(s: &str) -> Spur {
        match s {
            "ton" => Spur::Ton,
            "bild" => Spur::Image,
            other => panic!("unknown track kind {}", other),
        }
    }

    fn vector_key(v: &serde_json::Value) -> Key {
        let mut roh = [0u8; 32];
        roh.copy_from_slice(&hex::decode(v["root_hex"].as_str().unwrap()).unwrap());
        Key::aus_roh(roh)
    }

    /// Key derivation and nonce layout match the Python reference byte for byte.
    #[test]
    fn vectors_keys_and_nonces() {
        let v = vektoren();
        let k = vector_key(&v);
        for e in v["keys"].as_array().unwrap() {
            let got = derive_key(&k.roh, spur_von(e["spur"].as_str().unwrap()), &salt_hex(e["salt"].as_str().unwrap()));
            assert_eq!(hex::encode(got), e["key"].as_str().unwrap(), "key {}", e);
        }
        for e in v["nonces"].as_array().unwrap() {
            let got = make_nonce(
                spur_von(e["spur"].as_str().unwrap()),
                e["nal"].as_u64().unwrap() as u16,
                &salt_hex(e["salt"].as_str().unwrap()),
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
                &salt_hex(e["salt"].as_str().unwrap()),
                e["counter"].as_u64().unwrap() as u32,
                e["keyframe"].as_bool().unwrap(),
                &plain,
            );
            assert_eq!(got, frame, "audio vector {}", e["counter"]);
            // A fresh receiver (empty key cache) opens the reference frame.
            let rx = vector_key(&v);
            assert_eq!(rx.open(Spur::Ton, &frame), Some(plain));
        }
        for e in v["h264"].as_array().unwrap() {
            let plain = hex::decode(e["plain"].as_str().unwrap()).unwrap();
            let frame = hex::decode(e["frame"].as_str().unwrap()).unwrap();
            let got = k.seal_h264_at(
                &salt_hex(e["salt"].as_str().unwrap()),
                e["counter"].as_u64().unwrap() as u32,
                &plain,
            );
            assert_eq!(got, frame, "h264 vector {}", e["counter"]);
            let rx = vector_key(&v);
            assert_eq!(rx.open_h264(&frame), Some(plain));
        }
    }

    /// r82: slice blobs name the slice's pps_id in byte 0.
    #[test]
    fn vectors_pps_id_in_slice_blobs() {
        let v = vektoren();
        let k = vector_key(&v);
        for e in v["h264_pps"].as_array().unwrap() {
            let plain = hex::decode(e["plain"].as_str().unwrap()).unwrap();
            let frame = hex::decode(e["frame"].as_str().unwrap()).unwrap();
            let pps = e["pps"].as_u64().unwrap();
            let got = k.seal_h264_at(
                &salt_hex(e["salt"].as_str().unwrap()),
                e["counter"].as_u64().unwrap() as u32,
                &plain,
            );
            assert_eq!(got, frame, "pps vector {pps}");
            let rx = vector_key(&v);
            assert_eq!(rx.open_h264(&frame), Some(plain));
            // The receiver-side view of each encrypted slice: pps_id as read from the blob.
            for (_, db, end) in nal_units(&frame) {
                let typ = frame[db] & 0x1f;
                if typ == 1 || typ == 5 {
                    let roh = epb_out(&frame[db + 1..end]);
                    assert_eq!(slice_pps_id(&roh), Some(if pps <= 6 { pps as u32 } else { 0 }));
                }
            }
        }
        // Table sanity: one byte, reads (0, 0, pps), all distinct.
        for (p, b) in VIDEO_TAG.iter().enumerate() {
            assert_eq!(slice_pps_id(&[*b, 0, 0, 0, 0, 0, 0, 0]), Some(p as u32));
            assert_eq!(b >> 6, 3);
        }
        // A byte 0 outside the table is refused.
        let e = &v["h264_pps"][1];
        let mut bad = hex::decode(e["frame"].as_str().unwrap()).unwrap();
        let (_, db, _) = nal_units(&bad)[2];
        bad[db + 1] = 0xC0;
        assert_eq!(vector_key(&v).open_h264(&bad), None);
    }

    #[test]
    fn sample_frame_matches_the_reference() {
        let v = vektoren();
        let ref_plain = hex::decode(v["h264"][0]["plain"].as_str().unwrap()).unwrap();
        assert_eq!(example_frame(), ref_plain, "sample frame drifted from ref.py");
    }

    // ------------------------------------------------ replay (r60) --------

    #[test]
    fn vectors_guard_windows() {
        let v = vektoren();
        for c in v["guard"].as_array().unwrap() {
            let mut w = Guard::default();
            for (i, st) in c["steps"].as_array().unwrap().iter().enumerate() {
                let got = w.check(
                    &salt_hex(st["salt"].as_str().unwrap()),
                    st["counter"].as_u64().unwrap() as u32,
                );
                assert_eq!(got, st["accept"].as_bool().unwrap(), "{} step {}", c["name"], i);
            }
        }
    }

    #[test]
    fn vectors_video_replay() {
        let v = vektoren();
        let frames: Vec<Vec<u8>> = v["replay_video"]["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| hex::decode(f.as_str().unwrap()).unwrap())
            .collect();
        for c in v["replay_video"]["cases"].as_array().unwrap() {
            let k = vector_key(&v);
            let mut w = Guard::default();
            for (i, st) in c["steps"].as_array().unwrap().iter().enumerate() {
                let f = &frames[st["frame"].as_u64().unwrap() as usize];
                let got = k.open_h264_guard(&mut w, f).is_some();
                assert_eq!(got, st["accept"].as_bool().unwrap(), "{} step {}", c["name"], i);
            }
        }
    }

    #[test]
    fn window_step_of_64_keeps_the_old_highest() {
        // Regression: a step of exactly 64 used to drop the old highest, so
        // its replay was accepted.
        let mut w = Guard::default();
        let s = [7u8; SALT_LEN];
        assert!(w.check(&s, 0));
        assert!(w.check(&s, 64));
        assert!(!w.check(&s, 0), "replay of the old highest accepted");
    }

    #[test]
    fn video_replay_is_dropped_live() {
        let k = Key::neu();
        let mut tx = video(&k);
        let f = example_frame();
        let a = tx.protect_h264(&f);
        let b = tx.protect_h264(&f);
        let rx = Key::aus_roh(k.roh);
        let mut w = Guard::default();
        assert_eq!(rx.open_h264_guard(&mut w, &a).as_deref(), Some(&f[..]));
        assert_eq!(rx.open_h264_guard(&mut w, &b).as_deref(), Some(&f[..]));
        assert!(rx.open_h264_guard(&mut w, &a).is_none(), "replay shown");
        assert!(rx.open_h264_guard(&mut w, &b).is_none(), "replay shown");
    }

    #[test]
    fn forged_video_frame_does_not_move_the_window() {
        let k = Key::neu();
        let mut tx = video(&k);
        let f = example_frame();
        let a = tx.protect_h264(&f);
        let mut bad = tx.protect_h264(&f);
        let n = bad.len();
        bad[n - 1] ^= 1; // break the tag of the last NAL unit
        let rx = Key::aus_roh(k.roh);
        let mut w = Guard::default();
        assert!(rx.open_h264_guard(&mut w, &bad).is_none());
        assert!(rx.open_h264_guard(&mut w, &a).is_some(), "forgery used up the counter");
    }
}
