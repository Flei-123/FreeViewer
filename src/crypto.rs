//! End-to-end crypto for FreeViewer.
//!
//! Handshake (all frames go through the relay, which cannot read them):
//!   viewer -> host : 0x01 || client_pub(32)
//!   host -> viewer : 0x02 || host_pub(32) || salt(16)
//!   viewer -> host : 0x03 || proof(32)
//!   host -> viewer : 0x04            (password ok)
//!                  | 0x05            (password wrong)
//!   afterwards     : 0x10 || nonce(12) || AES-256-GCM ciphertext
//!
//! Session key  = HKDF-SHA256(ikm = X25519 shared secret, salt, "freeviewer-v1")
//! Password key = Argon2id(password, salt)
//! proof        = HMAC-SHA256(password key, "fv-auth" || client_pub || host_pub || salt)
//!
//! The relay never learns the password (only a proof bound to this one
//! session's ephemeral keys) and cannot decrypt the stream.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

type HmacSha256 = Hmac<Sha256>;

pub const TAG_HELLO: u8 = 0x01;
pub const TAG_HELLO_ACK: u8 = 0x02;
pub const TAG_PROOF: u8 = 0x03;
pub const TAG_OK: u8 = 0x04;
pub const TAG_FAIL: u8 = 0x05;
/// The viewer has no password and asks the person at the other end to allow
/// the session by hand (TeamViewer calls this "Bestaetigung anfordern").
pub const TAG_ASK: u8 = 0x06;
pub const TAG_DATA: u8 = 0x10;

/// Four digits derived from the session key. Both sides show the same number,
/// so a connection made without a password can still be verified by reading
/// it out loud - the relay cannot produce it without the private keys.
pub fn session_code(key: &[u8; 32]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"freeviewer-v1 code");
    h.update(key);
    let d = h.finalize();
    let n = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 10_000;
    format!("{:04}", n)
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    OsRng.fill_bytes(&mut v);
    v
}

pub struct Keypair {
    pub secret: StaticSecret,
    pub public: [u8; 32],
}

pub fn keypair() -> Keypair {
    let mut raw = [0u8; 32];
    OsRng.fill_bytes(&mut raw);
    let secret = StaticSecret::from(raw);
    let public = PublicKey::from(&secret).to_bytes();
    Keypair { secret, public }
}

pub fn session_key(secret: &StaticSecret, peer_pub: &[u8; 32], salt: &[u8; 16]) -> [u8; 32] {
    let shared = secret.diffie_hellman(&PublicKey::from(*peer_pub));
    let hk = Hkdf::<Sha256>::new(Some(salt), shared.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(b"freeviewer-v1 session", &mut okm)
        .expect("hkdf expand");
    okm
}

/// Argon2id over the session password. Deliberately slow so that a stolen
/// proof cannot be brute forced cheaply.
pub fn password_key(password: &str, salt: &[u8; 16]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let a = argon2::Argon2::default();
    if a.hash_password_into(password.as_bytes(), salt, &mut out).is_err() {
        // fall back to a plain hash so the handshake still completes deterministically
        let mut mac = <HmacSha256 as Mac>::new_from_slice(salt).expect("hmac key");
        mac.update(password.as_bytes());
        out.copy_from_slice(&mac.finalize().into_bytes());
    }
    out
}

pub fn auth_proof(
    pw_key: &[u8; 32],
    client_pub: &[u8; 32],
    host_pub: &[u8; 32],
    salt: &[u8; 16],
) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(pw_key).expect("hmac key");
    mac.update(b"fv-auth");
    mac.update(client_pub);
    mac.update(host_pub);
    mac.update(salt);
    let mut out = [0u8; 32];
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

pub fn proof_matches(expected: &[u8; 32], got: &[u8]) -> bool {
    if got.len() != 32 {
        return false;
    }
    // constant time compare
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= expected[i] ^ got[i];
    }
    diff == 0
}

pub struct Cipher {
    aead: Aes256Gcm,
    dir_send: u8,
    dir_recv: u8,
    ctr_send: u64,
    last_recv: u64,
}

impl Cipher {
    /// `is_host` only decides which nonce direction byte is used, so host and
    /// viewer can never collide on a nonce.
    pub fn new(key: &[u8; 32], is_host: bool) -> Self {
        let aead = Aes256Gcm::new_from_slice(key).expect("aes key");
        Self {
            aead,
            dir_send: if is_host { 1 } else { 2 },
            dir_recv: if is_host { 2 } else { 1 },
            ctr_send: 0,
            last_recv: 0,
        }
    }

    pub fn seal(&mut self, plain: &[u8]) -> Vec<u8> {
        self.ctr_send += 1;
        let mut nonce = [0u8; 12];
        nonce[0] = self.dir_send;
        nonce[4..12].copy_from_slice(&self.ctr_send.to_be_bytes());
        let ct = self
            .aead
            .encrypt(Nonce::from_slice(&nonce), plain)
            .expect("aes encrypt");
        let mut out = Vec::with_capacity(13 + ct.len());
        out.push(TAG_DATA);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    pub fn open(&mut self, frame: &[u8]) -> Option<Vec<u8>> {
        if frame.len() < 14 || frame[0] != TAG_DATA {
            return None;
        }
        let nonce = &frame[1..13];
        if nonce[0] != self.dir_recv {
            return None;
        }
        let mut ctr_bytes = [0u8; 8];
        ctr_bytes.copy_from_slice(&nonce[4..12]);
        let ctr = u64::from_be_bytes(ctr_bytes);
        if ctr <= self.last_recv {
            return None; // replay / reorder
        }
        let pt = self
            .aead
            .decrypt(Nonce::from_slice(nonce), &frame[13..])
            .ok()?;
        self.last_recv = ctr;
        Some(pt)
    }
}

/// Key for the direct UDP path. Derived from the session key so it is
/// completely independent of the WebSocket channel - reusing the same key with
/// two different counters would repeat a nonce, which breaks AES-GCM.
pub fn udp_key(session: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, session);
    let mut okm = [0u8; 32];
    hk.expand(b"freeviewer-v1 udp", &mut okm).expect("hkdf expand");
    okm
}

/// Same sealed format as `Cipher`, but built for datagrams: senders do not
/// need a lock and the receiver tolerates reordering through a 64 packet
/// sliding window (the scheme IPsec and WireGuard use) instead of demanding
/// strictly increasing counters.
pub struct UdpCipher {
    aead: Aes256Gcm,
    dir_send: u8,
    dir_recv: u8,
    ctr_send: std::sync::atomic::AtomicU64,
    window: std::sync::Mutex<(u64, u64)>,
}

impl UdpCipher {
    pub fn new(session_key: &[u8; 32], is_host: bool) -> Self {
        let key = udp_key(session_key);
        Self {
            aead: Aes256Gcm::new_from_slice(&key).expect("aes key"),
            dir_send: if is_host { 1 } else { 2 },
            dir_recv: if is_host { 2 } else { 1 },
            ctr_send: std::sync::atomic::AtomicU64::new(0),
            window: std::sync::Mutex::new((0, 0)),
        }
    }

    pub fn seal(&self, plain: &[u8]) -> Vec<u8> {
        let ctr = self
            .ctr_send
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        let mut nonce = [0u8; 12];
        nonce[0] = self.dir_send;
        nonce[4..12].copy_from_slice(&ctr.to_be_bytes());
        let ct = self
            .aead
            .encrypt(Nonce::from_slice(&nonce), plain)
            .expect("aes encrypt");
        let mut out = Vec::with_capacity(13 + ct.len());
        out.push(TAG_DATA);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    pub fn open(&self, frame: &[u8]) -> Option<Vec<u8>> {
        if frame.len() < 14 || frame[0] != TAG_DATA {
            return None;
        }
        let nonce = &frame[1..13];
        if nonce[0] != self.dir_recv {
            return None;
        }
        let mut ctr_bytes = [0u8; 8];
        ctr_bytes.copy_from_slice(&nonce[4..12]);
        let ctr = u64::from_be_bytes(ctr_bytes);
        if ctr == 0 {
            return None;
        }
        // check the window before spending time on decryption ...
        {
            let (high, mask) = *self.window.lock().unwrap();
            if ctr <= high {
                let d = high - ctr;
                if d >= 64 || mask & (1u64 << d) != 0 {
                    return None; // too old or already seen
                }
            }
        }
        let pt = self
            .aead
            .decrypt(Nonce::from_slice(nonce), &frame[13..])
            .ok()?;
        // ... and only mark it as seen once it is proven authentic
        let mut w = self.window.lock().unwrap();
        let (high, mask) = *w;
        if ctr > high {
            let shift = ctr - high;
            let m = if shift >= 64 { 0 } else { mask << shift };
            *w = (ctr, m | 1);
        } else {
            let d = high - ctr;
            if d < 64 {
                *w = (high, mask | (1u64 << d));
            }
        }
        Some(pt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_and_channel() {
        let host = keypair();
        let viewer = keypair();
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&random_bytes(16));

        let k_host = session_key(&host.secret, &viewer.public, &salt);
        let k_view = session_key(&viewer.secret, &host.public, &salt);
        assert_eq!(k_host, k_view, "ECDH must agree");

        let pw = password_key("hunter2", &salt);
        let good = auth_proof(&pw, &viewer.public, &host.public, &salt);
        assert!(proof_matches(&good, &good));
        let bad = auth_proof(&password_key("wrong", &salt), &viewer.public, &host.public, &salt);
        assert!(!proof_matches(&good, &bad));

        let mut h = Cipher::new(&k_host, true);
        let mut v = Cipher::new(&k_view, false);
        let frame = h.seal(b"hello viewer");
        assert_eq!(v.open(&frame).unwrap(), b"hello viewer");
        // replay must fail
        assert!(v.open(&frame).is_none());
        let back = v.seal(b"hello host");
        assert_eq!(h.open(&back).unwrap(), b"hello host");
    }

    #[test]
    fn udp_channel_tolerates_reordering_but_not_replay() {
        let key = [7u8; 32];
        let h = UdpCipher::new(&key, true);
        let v = UdpCipher::new(&key, false);
        assert_ne!(udp_key(&key), key, "UDP muss einen eigenen Schluessel haben");

        let a = h.seal(b"eins");
        let b = h.seal(b"zwei");
        let c = h.seal(b"drei");
        // out of order delivery is normal on UDP and must work
        assert_eq!(v.open(&c).unwrap(), b"drei");
        assert_eq!(v.open(&a).unwrap(), b"eins");
        assert_eq!(v.open(&b).unwrap(), b"zwei");
        // every packet only once
        assert!(v.open(&b).is_none());
        // wrong direction (our own packet) must not open
        assert!(h.open(&a).is_none());
        // a packet far outside the window is dropped
        for _ in 0..80 {
            let x = h.seal(b"fuellung");
            assert!(v.open(&x).is_some());
        }
        assert!(v.open(&a).is_none());

        // tampering is caught by the AEAD tag
        let mut bad = h.seal(b"echt");
        let n = bad.len();
        bad[n - 1] ^= 0xff;
        assert!(v.open(&bad).is_none());
    }
}
