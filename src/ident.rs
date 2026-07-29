//! Persistent machine identity: a 32 byte secret that the relay maps to a
//! stable 9 digit FreeViewer ID. The secret never leaves this machine except
//! as the plain value used for registration (relay stores only its SHA-256).

use std::fs;
use std::path::PathBuf;

use crate::crypto::random_bytes;

pub fn config_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("FreeViewer");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("freeviewer");
    }
    PathBuf::from(".freeviewer")
}

pub fn load_or_create_secret() -> String {
    let dir = config_dir();
    let file = dir.join("identity.txt");
    if let Ok(s) = fs::read_to_string(&file) {
        let s = s.trim().to_string();
        if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            return s;
        }
    }
    let secret = hex::encode(random_bytes(32));
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&file, &secret);
    secret
}

/// Session password in TeamViewer style: short, readable, random.
pub fn random_password() -> String {
    // no 0/O/1/l to avoid confusion when reading it out loud
    const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let raw = random_bytes(8);
    raw.iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect()
}

/// Auto update is on unless <config dir>/noupdate exists.
pub fn auto_update_enabled() -> bool {
    !config_dir().join("noupdate").exists()
}

/// Turns automatic updates on/off (remembered across restarts).
pub fn set_auto_update(on: bool) {
    let flag = config_dir().join("noupdate");
    if on {
        let _ = std::fs::remove_file(flag);
    } else {
        let _ = std::fs::write(flag, b"1");
    }
}

/// Optional fixed password for unattended access:
/// put it into <config dir>/password.txt (one line). If the file is missing a
/// fresh random session password is generated on every start.
pub fn fixed_password() -> Option<String> {
    let file = config_dir().join("password.txt");
    let s = fs::read_to_string(file).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}