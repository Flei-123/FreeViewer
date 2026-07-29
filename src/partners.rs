//! Address book: who did we connect to, when, and how do we get back there.
//!
//! TeamViewer keeps that list in its cloud account. This one lives in a plain
//! file next to the machine identity, so it works without any account at all -
//! and it can be synced later without changing the format.
//!
//! Saved passwords are encrypted with a key derived from this machine's
//! identity secret (`identity.txt`). That secret never leaves the machine and
//! is already the thing that makes this installation "us", so anybody able to
//! read it owns the installation anyway - but a stolen `partners.json` on its
//! own is worthless.

use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::crypto::random_bytes;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Partner {
    pub id: String,
    /// What the user called it. Empty = show the plain ID.
    #[serde(default)]
    pub name: String,
    /// Last successful connection, unix seconds.
    #[serde(default)]
    pub last: u64,
    /// How often we connected.
    #[serde(default)]
    pub count: u32,
    /// Total time connected, seconds.
    #[serde(default)]
    pub seconds: u64,
    /// Pinned entries stay on top.
    #[serde(default)]
    pub favorite: bool,
    /// Encrypted password (hex), only present if the user asked for it.
    #[serde(default)]
    pub secret: Option<String>,
}

impl Partner {
    /// What the list shows.
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() {
            pretty_id(&self.id)
        } else {
            self.name.clone()
        }
    }

    /// "vor 3 Minuten", "gestern", ...
    pub fn ago(&self) -> String {
        if self.last == 0 {
            return "noch nie".to_string();
        }
        let d = now().saturating_sub(self.last);
        match d {
            0..=59 => "gerade eben".to_string(),
            60..=3599 => format!("vor {} Min.", d / 60),
            3600..=86399 => format!("vor {} Std.", d / 3600),
            86400..=172799 => "gestern".to_string(),
            _ => format!("vor {} Tagen", d / 86400),
        }
    }

    pub fn total(&self) -> String {
        let s = self.seconds;
        if s < 60 {
            format!("{} s", s)
        } else if s < 3600 {
            format!("{} Min.", s / 60)
        } else {
            format!("{:.1} Std.", s as f64 / 3600.0)
        }
    }
}

/// 497628420 -> "497 628 420"
pub fn pretty_id(id: &str) -> String {
    if id.len() == 9 {
        format!("{} {} {}", &id[0..3], &id[3..6], &id[6..9])
    } else {
        id.to_string()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize, Deserialize, Default)]
pub struct Book {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<Partner>,
}

/// Never grow without bound - the list is a convenience, not an archive.
const MAX_ENTRIES: usize = 200;

impl Book {
    fn path() -> std::path::PathBuf {
        crate::ident::config_dir().join("partners.json")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let dir = crate::ident::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let tmp = dir.join("partners.json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, Self::path());
            }
        }
    }

    /// Favourites first, then most recently used.
    pub fn sorted(&self) -> Vec<Partner> {
        let mut v = self.entries.clone();
        v.sort_by(|a, b| b.favorite.cmp(&a.favorite).then(b.last.cmp(&a.last)));
        v
    }

    pub fn get(&self, id: &str) -> Option<&Partner> {
        self.entries.iter().find(|p| p.id == id)
    }

    fn entry(&mut self, id: &str) -> &mut Partner {
        if let Some(i) = self.entries.iter().position(|p| p.id == id) {
            return &mut self.entries[i];
        }
        self.entries.push(Partner {
            id: id.to_string(),
            ..Default::default()
        });
        let n = self.entries.len() - 1;
        &mut self.entries[n]
    }

    /// A session started. `remember` stores the password, `None` clears it.
    pub fn started(&mut self, id: &str, password: &str, remember: bool) {
        let secret = if remember && !password.is_empty() {
            protect(password)
        } else {
            None
        };
        {
            let e = self.entry(id);
            e.last = now();
            e.count += 1;
            if remember {
                if secret.is_some() {
                    e.secret = secret;
                }
            } else {
                e.secret = None;
            }
        }
        self.trim();
        self.save();
    }

    /// A session ended after `secs` seconds.
    pub fn ended(&mut self, id: &str, secs: u64) {
        {
            let e = self.entry(id);
            e.seconds += secs;
        }
        self.save();
    }

    pub fn rename(&mut self, id: &str, name: &str) {
        self.entry(id).name = name.trim().to_string();
        self.save();
    }

    pub fn toggle_favorite(&mut self, id: &str) {
        let e = self.entry(id);
        e.favorite = !e.favorite;
        self.save();
    }

    pub fn remove(&mut self, id: &str) {
        self.entries.retain(|p| p.id != id);
        self.save();
    }

    /// Decrypted password, if one was stored.
    pub fn password(&self, id: &str) -> Option<String> {
        self.get(id).and_then(|p| p.secret.as_ref()).and_then(|s| unprotect(s))
    }

    fn trim(&mut self) {
        if self.entries.len() <= MAX_ENTRIES {
            return;
        }
        let mut v = std::mem::take(&mut self.entries);
        v.sort_by(|a, b| b.favorite.cmp(&a.favorite).then(b.last.cmp(&a.last)));
        v.truncate(MAX_ENTRIES);
        self.entries = v;
    }
}

// ------------------------------------------------------------- encryption --

/// AES-256-GCM with a key derived from the machine identity. Format:
/// hex(12 byte nonce || ciphertext).
fn book_key() -> [u8; 32] {
    let secret = crate::ident::load_or_create_secret();
    let hk = Hkdf::<Sha256>::new(None, secret.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(b"freeviewer-v1 partners", &mut key)
        .expect("hkdf expand");
    key
}

fn protect(plain: &str) -> Option<String> {
    let aead = Aes256Gcm::new_from_slice(&book_key()).ok()?;
    let nonce_bytes = random_bytes(12);
    let ct = aead
        .encrypt(Nonce::from_slice(&nonce_bytes), plain.as_bytes())
        .ok()?;
    let mut out = nonce_bytes;
    out.extend_from_slice(&ct);
    Some(hex::encode(out))
}

fn unprotect(stored: &str) -> Option<String> {
    let raw = hex::decode(stored).ok()?;
    if raw.len() < 13 {
        return None;
    }
    let aead = Aes256Gcm::new_from_slice(&book_key()).ok()?;
    let pt = aead.decrypt(Nonce::from_slice(&raw[..12]), &raw[12..]).ok()?;
    String::from_utf8(pt).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_grouped_for_reading() {
        assert_eq!(pretty_id("497628420"), "497 628 420");
        assert_eq!(pretty_id("12345"), "12345");
    }

    #[test]
    fn favourites_come_first_then_most_recent() {
        let mut b = Book::default();
        b.entries = vec![
            Partner {
                id: "111111111".into(),
                last: 100,
                ..Default::default()
            },
            Partner {
                id: "222222222".into(),
                last: 500,
                ..Default::default()
            },
            Partner {
                id: "333333333".into(),
                last: 10,
                favorite: true,
                ..Default::default()
            },
        ];
        let order: Vec<String> = b.sorted().into_iter().map(|p| p.id).collect();
        assert_eq!(order, vec!["333333333", "222222222", "111111111"]);
    }

    #[test]
    fn stored_passwords_are_not_readable_in_the_file() {
        let pw = "FleiTec2026";
        let blob = protect(pw).expect("protect");
        assert!(!blob.contains("FleiTec"));
        assert!(!blob.contains(pw));
        assert_eq!(unprotect(&blob).as_deref(), Some(pw));
        // a damaged blob must not panic and must not return garbage
        let mut bad = blob.clone();
        bad.replace_range(20..21, if &bad[20..21] == "a" { "b" } else { "a" });
        assert!(unprotect(&bad).is_none() || unprotect(&bad).as_deref() != Some(pw));
        assert!(unprotect("zzzz").is_none());
        assert!(unprotect("").is_none());
    }

    #[test]
    fn labels_fall_back_to_the_id() {
        let p = Partner {
            id: "497628420".into(),
            ..Default::default()
        };
        assert_eq!(p.label(), "497 628 420");
        let p2 = Partner {
            id: "497628420".into(),
            name: "Werkstatt-PC".into(),
            ..Default::default()
        };
        assert_eq!(p2.label(), "Werkstatt-PC");
        assert_eq!(p.ago(), "noch nie");
    }
}
