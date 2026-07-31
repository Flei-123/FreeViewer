//! `freeviewer://` - der Weg vom Browser zurueck in den Client.
//!
//! Im Meeting kann jemand die Fernsteuerung freigeben. Die anderen sehen dann
//! einen Knopf "Steuern"; der oeffnet
//!
//!     freeviewer://control/497628420
//!
//! Windows startet daraufhin FreeViewer mit dieser Adresse als Argument.
//! Laeuft das Programm schon (Regel: nur ein Fenster pro Nutzer), legt der
//! zweite Start die Adresse nur in einen Briefkasten und beendet sich - die
//! laufende Fassung holt sie sich von dort ab.
//!
//! Bewusst OHNE Passwort in der Adresse: die Verbindung geht den Anfrage-Weg,
//! der Besitzer muss am anderen Ende zulassen. Ein Link, der irgendwo im Chat
//! steht, darf keinen fremden Rechner aufsperren.

use std::path::PathBuf;

pub const SCHEME: &str = "freeviewer";

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Diesen Rechner fernsteuern (FreeViewer-ID).
    Control(String),
    /// Einem Meeting beitreten.
    Meet { room: String, pass: String },
}

/// Zerlegt `freeviewer://control/123456789` bzw.
/// `freeviewer://meet/482-913-770?pass=abc`.
pub fn parse(url: &str) -> Option<Action> {
    let u = url.trim().trim_end_matches('/');
    let rest = u
        .strip_prefix("freeviewer://")
        .or_else(|| u.strip_prefix("freeviewer:"))?;
    let (pfad, frage) = match rest.split_once('?') {
        Some((a, b)) => (a, b),
        None => (rest, ""),
    };
    let mut teile = pfad.splitn(2, '/');
    let was = teile.next().unwrap_or("");
    let wert = teile.next().unwrap_or("").trim().to_string();
    match was {
        "control" => {
            let id: String = wert.chars().filter(|c| c.is_ascii_digit()).collect();
            if id.len() < 6 {
                return None;
            }
            Some(Action::Control(id))
        }
        "meet" => {
            if wert.is_empty() {
                return None;
            }
            let mut pass = String::new();
            for kv in frage.split('&') {
                if let Some(v) = kv.strip_prefix("pass=") {
                    pass = urldec(v);
                }
            }
            Some(Action::Meet { room: wert, pass })
        }
        _ => None,
    }
}

fn urldec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Briefkasten fuer eine bereits laufende Fassung.
pub fn inbox() -> PathBuf {
    crate::ident::config_dir().join("inbox.txt")
}

pub fn drop_in(url: &str) {
    let _ = std::fs::create_dir_all(crate::ident::config_dir());
    let _ = std::fs::write(inbox(), url.trim());
}

/// Holt eine wartende Adresse ab (und raeumt den Briefkasten).
pub fn take() -> Option<Action> {
    let p = inbox();
    let raw = std::fs::read_to_string(&p).ok()?;
    let _ = std::fs::remove_file(&p);
    parse(raw.trim())
}

// ------------------------------------------------------------- Registrierung

/// Traegt das Schema ein. `machine_wide` = HKLM (Installation, alle Nutzer),
/// sonst HKCU - das geht auch portabel und ohne Administrator.
#[cfg(windows)]
pub fn register(machine_wide: bool) -> std::io::Result<()> {
    register_for(&std::env::current_exe()?, machine_wide)
}

/// Wie register, aber fuer eine bestimmte Datei - beim Installieren zeigt
/// der Eintrag sonst auf die Quelle statt auf das Programm in "Programme".
#[cfg(windows)]
pub fn register_for(exe: &std::path::Path, machine_wide: bool) -> std::io::Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let root = if machine_wide {
        RegKey::predef(HKEY_LOCAL_MACHINE)
    } else {
        RegKey::predef(HKEY_CURRENT_USER)
    };
    let (key, _) = root.create_subkey(format!(r"Software\Classes\{}", SCHEME))?;
    key.set_value("", &"URL:FreeViewer")?;
    key.set_value("URL Protocol", &"")?;
    let (icon, _) = key.create_subkey("DefaultIcon")?;
    icon.set_value("", &format!("{},0", exe.display()))?;
    let (cmd, _) = key.create_subkey(r"shell\open\command")?;
    cmd.set_value("", &format!("\"{}\" \"%1\"", exe.display()))?;
    Ok(())
}

#[cfg(not(windows))]
pub fn register(_machine_wide: bool) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn register_for(_exe: &std::path::Path, _machine_wide: bool) -> std::io::Result<()> {
    Ok(())
}

/// Zeigt der Eintrag auf diese Datei? (Nach einem Umzug stimmt er nicht mehr.)
#[cfg(windows)]
pub fn points_here() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let exe = match std::env::current_exe() {
        Ok(e) => e.to_string_lossy().to_lowercase(),
        Err(_) => return false,
    };
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(format!(r"Software\Classes\{}\shell\open\command", SCHEME))
        .and_then(|k| k.get_value::<String, _>(""))
        .map(|v| v.to_lowercase().contains(&exe))
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn points_here() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_link_is_read() {
        assert_eq!(
            parse("freeviewer://control/497628420"),
            Some(Action::Control("497628420".into()))
        );
        // Schreibweise mit Leerzeichen oder Schraegstrich am Ende
        assert_eq!(
            parse("freeviewer://control/497 628 420/"),
            Some(Action::Control("497628420".into()))
        );
    }

    #[test]
    fn meet_link_carries_the_password() {
        assert_eq!(
            parse("freeviewer://meet/482-913-770?pass=ab%20c"),
            Some(Action::Meet {
                room: "482-913-770".into(),
                pass: "ab c".into()
            })
        );
    }

    #[test]
    fn nonsense_is_refused() {
        assert_eq!(parse("https://example.com"), None);
        assert_eq!(parse("freeviewer://control/12"), None);
        assert_eq!(parse("freeviewer://unfug/1"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn percent_decoding_works() {
        assert_eq!(urldec("a%2Fb"), "a/b");
        assert_eq!(urldec("nichts"), "nichts");
        assert_eq!(urldec("a+b"), "a b");
    }
}
