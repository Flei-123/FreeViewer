//! Persistent machine identity: a 32 byte secret that the relay maps to a
//! stable 9 digit FreeViewer ID. The secret never leaves this machine except
//! as the plain value used for registration (relay stores only its SHA-256).

use std::fs;
use std::path::PathBuf;

use crate::crypto::random_bytes;

/// Per user configuration - the normal case without a service.
pub fn user_config_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("FreeViewer");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("freeviewer");
    }
    PathBuf::from(".freeviewer")
}

/// Machine wide folder. The service agent can run as SYSTEM or as the logged
/// in user, so identity, password and address book must not live in a user
/// profile - otherwise the FreeViewer ID would change with the account.
pub fn machine_config_dir() -> Option<PathBuf> {
    let pd = std::env::var("ProgramData").ok()?;
    Some(PathBuf::from(pd).join("FreeViewer"))
}

/// Where this installation keeps its files. As soon as a machine wide
/// identity exists (the service installer creates it) that one wins, so every
/// account sees the same FreeViewer ID.
pub fn config_dir() -> PathBuf {
    if let Ok(p) = std::env::var("FV_CONFIG") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(d) = machine_config_dir() {
        if d.join("identity.txt").exists() {
            return d;
        }
    }
    user_config_dir()
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

fn password_file() -> PathBuf {
    config_dir().join("password.txt")
}

/// Optional fixed password for unattended access:
/// put it into <config dir>/password.txt (one line). If the file is missing a
/// fresh random session password is generated on every start.
pub fn fixed_password() -> Option<String> {
    let s = fs::read_to_string(password_file()).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Is a permanent password configured on THIS machine? Every installation
/// decides that for itself - the password is per machine, never global.
pub fn has_fixed_password() -> bool {
    fixed_password().is_some()
}

/// Stores (or removes) the permanent password of this machine.
/// `None` or an empty string switches back to "new random password on every
/// start", which is the TeamViewer-like default.
pub fn set_fixed_password(pw: Option<&str>) -> std::io::Result<()> {
    let file = password_file();
    match pw.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(s) => {
            let _ = fs::create_dir_all(config_dir());
            fs::write(&file, s)
        }
        None => match fs::remove_file(&file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_password_is_readable_and_long_enough() {
        let p = random_password();
        assert_eq!(p.len(), 8);
        assert!(p.chars().all(|c| c.is_ascii_alphanumeric()));
        // the confusing characters must never show up
        assert!(!p.contains('0') && !p.contains('1') && !p.contains('l') && !p.contains('o'));
        // two draws in a row must not be identical
        assert_ne!(p, random_password());
    }
}

/// Zwischenablage teilen? Aus, wenn die Datei "noclip" im Ordner liegt.
pub fn clipboard_enabled() -> bool {
    !config_dir().join("noclip").exists()
}

pub fn set_clipboard(on: bool) {
    let f = config_dir().join("noclip");
    if on {
        let _ = std::fs::remove_file(f);
    } else {
        let _ = std::fs::create_dir_all(config_dir());
        let _ = std::fs::write(f, b"1");
    }
}