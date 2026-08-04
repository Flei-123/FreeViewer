//! "Mit Windows starten" - the Run key of the logged in user.
//!
//! Deliberately HKCU and not a machine wide key: it needs no admin rights and
//! the host then runs inside the user's own session, which is exactly what
//! DXGI capture and SendInput need. Starting before anybody logs in is a
//! different job and belongs to the Windows service (`service.rs`).

use std::path::PathBuf;

/// Name of the value under the Run key.
/// Registry-Eintrag unter dem Markennamen.
pub const VALUE: &str = crate::brand::NAME;

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Full path of the running binary in quotes, plus the flag that starts us
/// silently into the tray instead of opening a window in the user's face.
pub fn command() -> String {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("freeviewer.exe"));
    format!("\"{}\" --tray", exe.display())
}

/// Do two Run-key commands point at the same executable? Compared case
/// insensitively and without the arguments, because Windows paths are case
/// insensitive and the flags may well differ between versions.
pub fn same_exe(a: &str, b: &str) -> bool {
    fn exe_of(s: &str) -> String {
        let s = s.trim();
        let path = if let Some(rest) = s.strip_prefix('"') {
            rest.split('"').next().unwrap_or("").to_string()
        } else {
            s.split_whitespace().next().unwrap_or("").to_string()
        };
        path.to_lowercase().replace('/', "\\")
    }
    !exe_of(a).is_empty() && exe_of(a) == exe_of(b)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    /// The command line that is currently registered, if any.
    pub fn current() -> Option<String> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY).ok()?;
        key.get_value::<String, _>(VALUE).ok()
    }

    pub fn enabled() -> bool {
        current().is_some()
    }

    /// Autostart is on *and* points at this very binary. After moving the
    /// program somewhere else the old entry is stale.
    pub fn points_here() -> bool {
        match current() {
            Some(c) => same_exe(&c, &command()),
            None => false,
        }
    }

    pub fn set(on: bool) -> std::io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(RUN_KEY)?;
        if on {
            key.set_value(VALUE, &command())
        } else {
            match key.delete_value(VALUE) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        }
    }

    /// Keeps a stale entry pointing at the right place (after a move or an
    /// update that landed in another folder). Does nothing when autostart is
    /// switched off - that decision belongs to the user.
    pub fn refresh() {
        if enabled() && !points_here() {
            let _ = set(true);
        }
    }
}

#[cfg(windows)]
pub use imp::{current, enabled, points_here, refresh, set};

#[cfg(not(windows))]
pub fn current() -> Option<String> {
    None
}
#[cfg(not(windows))]
pub fn enabled() -> bool {
    false
}
#[cfg(not(windows))]
pub fn points_here() -> bool {
    false
}
#[cfg(not(windows))]
pub fn refresh() {}
#[cfg(not(windows))]
pub fn set(_on: bool) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Autostart gibt es nur unter Windows",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_quoted_and_starts_into_the_tray() {
        let c = command();
        assert!(c.starts_with('"'), "{}", c);
        assert!(c.ends_with("--tray"), "{}", c);
        assert!(c.to_lowercase().contains(".exe") || cfg!(not(windows)));
    }

    #[test]
    fn same_exe_ignores_case_arguments_and_slashes() {
        assert!(same_exe(
            "\"C:\\Program Files\\FreeViewer\\freeviewer.exe\" --tray",
            "\"c:/program files/freeviewer/FreeViewer.EXE\" --tray --later"
        ));
        assert!(same_exe("C:\\tools\\fv.exe", "\"C:\\tools\\fv.exe\" --tray"));
        assert!(!same_exe(
            "\"C:\\a\\freeviewer.exe\" --tray",
            "\"C:\\b\\freeviewer.exe\" --tray"
        ));
        assert!(!same_exe("", "\"C:\\a\\freeviewer.exe\""));
    }
}
