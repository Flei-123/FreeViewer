//! Persistent machine identity: a 32 byte secret that the relay maps to a
//! stable 9 digit FreeViewer ID. The secret never leaves this machine except
//! as the plain value used for registration (relay stores only its SHA-256).

use std::fs;
use std::path::PathBuf;

use crate::crypto::random_bytes;

/// Per user configuration - the normal case without a service.
pub fn user_config_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join(crate::brand::DIR);
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
    Some(PathBuf::from(pd).join(crate::brand::DIR))
}

/// Where this installation keeps its files. As soon as a machine wide
/// identity exists (the service installer creates it) that one wins, so every
/// account sees the same FreeViewer ID.
pub fn config_dir() -> PathBuf {
    // Tests NEVER touch the real configuration. A unit test that ends up
    // via Book::save() in %ProgramData%\\FreeViewer deletes the user's
    // address book - exactly that happened on 31.07.2026.
    // Therefore in a test run the folder points hard to a temp directory of its own.
    #[cfg(test)]
    {
        return test_config_dir();
    }
    #[cfg(not(test))]
    {
        real_config_dir()
    }
}

#[cfg(test)]
thread_local! {
    /// A single test can give itself its own folder. Per thread,
    /// so that tests running in parallel do not get in each other's way.
    static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Sets the configuration folder ONLY for this test thread.
#[cfg(test)]
pub fn set_test_config_dir(p: PathBuf) {
    let _ = fs::create_dir_all(&p);
    TEST_DIR.with(|d| *d.borrow_mut() = Some(p));
}

/// Own throwaway folder per test. Cargo gives each test its own
/// thread, so the thread id is enough to separate tests cleanly -
/// otherwise tests running in parallel would write away each other's identity.txt
/// from under their feet.
#[cfg(test)]
pub fn test_config_dir() -> PathBuf {
    if let Some(p) = TEST_DIR.with(|d| d.borrow().clone()) {
        return p;
    }
    let tid = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>();
    let d = std::env::temp_dir().join(format!("freeviewer-test-{}-{}", std::process::id(), tid));
    let _ = fs::create_dir_all(&d);
    TEST_DIR.with(|t| *t.borrow_mut() = Some(d.clone()));
    d
}

/// The real folder in operation.
pub fn real_config_dir() -> PathBuf {
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
            // Catch up the permanent backup in the registry if there is none
            // yet - then the ID also survives a deleted config directory.
            #[cfg(windows)]
            if winid::load_backup().as_deref() != Some(s.as_str()) {
                winid::save_backup(&s);
            }
            return s;
        }
    }
    // File gone or invalid - ask the permanent backup first.
    #[cfg(windows)]
    if let Some(s) = winid::load_backup() {
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(&file, &s);
        return s;
    }
    // Brand new: derive from Windows' own machine id. It stays
    // the same across uninstalls and reinstalls (only a new Windows changes
    // it) - so the FreeViewer ID never changes again.
    #[cfg(windows)]
    let frisch = winid::machine_guid().map(|g| {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(b"freeviewer-id:");
        h.update(g.to_lowercase().as_bytes());
        hex::encode(h.finalize())
    });
    #[cfg(not(windows))]
    let frisch: Option<String> = None;
    let secret = frisch.unwrap_or_else(|| hex::encode(random_bytes(32)));
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&file, &secret);
    #[cfg(windows)]
    winid::save_backup(&secret);
    secret
}

/// Permanent identity backup (Windows): registry branch and the
/// machine id from the cryptography registration.
#[cfg(windows)]
pub mod winid {
    fn key_path() -> String {
        crate::brand::reg_key()
    }

    pub fn load_backup() -> Option<String> {
        let k = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
            .open_subkey(key_path())
            .ok()?;
        let s: String = k.get_value("identity").ok()?;
        let s = s.trim().to_string();
        if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(s)
        } else {
            None
        }
    }

    pub fn save_backup(secret: &str) {
        if let Ok((k, _)) = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
            .create_subkey(key_path())
        {
            let _ = k.set_value("identity", &secret);
        }
    }

    /// Only for complete removal (--uninstall --all).
    pub fn drop_backup() {
        if let Ok(k) = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(r"SOFTWARE", winreg::enums::KEY_ALL_ACCESS)
        {
            let _ = k.delete_subkey_all(crate::brand::DIR);
        }
    }

    pub fn machine_guid() -> Option<String> {
        let k = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Cryptography")
            .ok()?;
        k.get_value("MachineGuid").ok()
    }
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

/// Optional fixed password for unattended access, stored sealed
/// (`secstore`: DPAPI on Windows, AES-GCM elsewhere) in
/// <config dir>/password.txt. A legacy plain text file is still read and
/// rewritten sealed on first use (r10). If the file is missing a fresh random
/// session password is generated on every start.
pub fn fixed_password() -> Option<String> {
    let raw = fs::read_to_string(password_file()).ok()?;
    let raw = raw.trim().trim_start_matches('\u{feff}').to_string();
    if raw.is_empty() {
        return None;
    }
    let plain = crate::secstore::open(&raw)?;
    if plain.is_empty() {
        return None;
    }
    if crate::secstore::needs_reseal(&raw) {
        // migrate the old plain text / pre-Keychain file; failing to do so changes nothing
        let _ = crate::secstore::write_sealed_file(&password_file(), &plain);
    }
    Some(plain)
}

/// Is a permanent password configured on THIS machine? Every installation
/// decides that for itself - the password is per machine, never global.
pub fn has_fixed_password() -> bool {
    fixed_password().is_some()
}

/// Stores (or removes) the permanent password of this machine, sealed.
/// `None` or an empty string switches back to "new random password on every
/// start", which is the TeamViewer-like default.
pub fn set_fixed_password(pw: Option<&str>) -> std::io::Result<()> {
    let file = password_file();
    match pw.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(s) => crate::secstore::write_sealed_file(&file, s),
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

    fn own_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fv-ident-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&d);
        set_test_config_dir(d.clone());
        d
    }

    #[test]
    fn fixed_password_is_not_stored_in_plain_text() {
        let d = own_dir("sealed");
        set_fixed_password(Some("my-secret-pw-42")).unwrap();
        let raw = fs::read_to_string(d.join("password.txt")).unwrap();
        assert!(!raw.contains("my-secret-pw-42"), "plain text on disk: {raw}");
        assert!(crate::secstore::is_sealed(&raw));
        assert_eq!(fixed_password().as_deref(), Some("my-secret-pw-42"));
        assert!(has_fixed_password());
        set_fixed_password(None).unwrap();
        assert!(!has_fixed_password());
    }

    #[test]
    fn legacy_plain_password_file_is_migrated() {
        let d = own_dir("migrate");
        fs::write(d.join("password.txt"), "legacy-plain-pw\n").unwrap();
        assert_eq!(fixed_password().as_deref(), Some("legacy-plain-pw"));
        let raw = fs::read_to_string(d.join("password.txt")).unwrap();
        assert!(!raw.contains("legacy-plain-pw"), "not migrated: {raw}");
        // still the same password after the migration
        assert_eq!(fixed_password().as_deref(), Some("legacy-plain-pw"));
    }
}

/// Share the clipboard? Off if the file "noclip" is in the folder.
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

#[cfg(test)]
mod config_dir_tests {
    use super::*;

    /// Protection against the data loss of 31.07.2026: in a test run the
    /// configuration folder must never be the real one.
    #[test]
    fn tests_schreiben_nie_in_die_echte_konfiguration() {
        let d = config_dir();
        assert_eq!(d, test_config_dir());
        assert!(d.starts_with(std::env::temp_dir()));
        if let Some(m) = machine_config_dir() {
            assert_ne!(d, m);
        }
        assert_ne!(d, user_config_dir());
    }
}
