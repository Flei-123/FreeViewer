//! `freeviewer://` - the way from the browser back into the client.
//!
//! In a meeting someone can release remote control. The others then see
//! a "Control" button; it opens
//!
//!     freeviewer://control/497628420
//!
//! Windows then starts FreeViewer with this address as an argument.
//! If the program already runs (rule: only one window per user), the
//! second start only puts the address into a mailbox and exits - the
//! running instance picks it up from there.
//!
//! Deliberately WITHOUT a password in the address: the connection takes the request path,
//! the owner has to allow it at the other end. A link that sits somewhere in a chat
//! must not unlock a foreign machine.

use std::path::PathBuf;

/// Own scheme per brand. Two brands on ONE machine must not
/// take the links away from each other: up to v0.26.1 the scheme was called
/// "freeviewer" in every build, so the most recently installed build grabbed all
/// links (X-Remote opened FreeViewer meetings).
/// The default stays "freeviewer", X-Remote builds with FV_BRAND_SCHEME=xremote.
pub const SCHEME: &str = match option_env!("FV_BRAND_SCHEME") {
    Some(s) => s,
    None => "freeviewer",
};

/// The old, shared scheme. Links already in circulation should
/// keep working - therefore it is ALWAYS read along when parsing.
pub const SCHEME_ALT: &str = "freeviewer";

/// All schemes this build understands.
pub fn schemes() -> Vec<&'static str> {
    if SCHEME == SCHEME_ALT { vec![SCHEME] } else { vec![SCHEME, SCHEME_ALT] }
}

/// Does an address carry one of our schemes? (For the command line.)
pub fn is_ours(arg: &str) -> bool {
    let a = arg.trim().to_ascii_lowercase();
    schemes().iter().any(|s| a.starts_with(&format!("{}:", s)))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Diesen Rechner fernsteuern (FreeViewer-ID).
    Control(String),
    /// Einem Meeting beitreten.
    Meet { room: String, pass: String },
    /// Set up a new device (one-time code from the Installation tab).
    Setup(String),
}

/// Zerlegt `freeviewer://control/123456789` bzw.
/// `freeviewer://meet/482-913-770?pass=abc`.
pub fn parse(url: &str) -> Option<Action> {
    let u = url.trim().trim_end_matches('/');
    // Accept both schemes - the brand's own and the old
    // shared "freeviewer://" (links that were already sent).
    let mut rest_opt: Option<&str> = None;
    for sch in schemes() {
        let lang = format!("{}://", sch);
        let kurz = format!("{}:", sch);
        if let Some(r) = u.strip_prefix(&lang).or_else(|| u.strip_prefix(&kurz)) {
            rest_opt = Some(r);
            break;
        }
    }
    let rest = rest_opt?;
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
        "setup" => {
            // base64url, 8 Zeichen - kommt vom Relay (/fv/setup/create)
            if wert.len() < 6
                || wert.len() > 24
                || !wert
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return None;
            }
            Some(Action::Setup(wert))
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

/// Marker at the end of a downloaded exe: the setup page appends
/// code and desired name to the program, so that a fresh device sets itself up on
/// first start - without anyone having to type anything.
const EMBED_MARK: &str = "FVSETUP1:";

/// Reads the appended setup package from the own file (if present).
/// Returns: (code, desired name).
pub fn embedded_setup() -> Option<(String, String)> {
    let exe = std::env::current_exe().ok()?;
    let bytes = std::fs::read(exe).ok()?;
    // the package is small and sits at the very end
    let start = bytes.len().saturating_sub(4096);
    let tail = String::from_utf8_lossy(&bytes[start..]);
    let pos = tail.rfind(EMBED_MARK)?;
    let rest = &tail[pos + EMBED_MARK.len()..];
    let json: serde_json::Value = serde_json::from_str(rest.trim()).ok()?;
    let code = json.get("code")?.as_str()?.to_string();
    let name = json
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    if parse(&format!("{}://setup/{}", SCHEME, code)).is_none() {
        return None;
    }
    Some((code, name))
}

/// Mailbox for an already running instance.
pub fn inbox() -> PathBuf {
    crate::ident::config_dir().join("inbox.txt")
}

pub fn drop_in(url: &str) {
    let _ = std::fs::create_dir_all(crate::ident::config_dir());
    let _ = std::fs::write(inbox(), url.trim());
}

/// Picks up a waiting address (and cleans up the mailbox).
pub fn take() -> Option<Action> {
    let p = inbox();
    let raw = std::fs::read_to_string(&p).ok()?;
    let _ = std::fs::remove_file(&p);
    parse(raw.trim())
}

// ------------------------------------------------------------- Registrierung

/// Registers the scheme. `machine_wide` = HKLM (installation, all users),
/// otherwise HKCU - that also works portably and without administrator.
#[cfg(windows)]
pub fn register(machine_wide: bool) -> std::io::Result<()> {
    register_for(&std::env::current_exe()?, machine_wide)
}

/// Like register, but for a specific file - when installing, the
/// entry would otherwise point to the source instead of the program in "Program Files".
#[cfg(windows)]
pub fn register_for(exe: &std::path::Path, machine_wide: bool) -> std::io::Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let root = if machine_wide {
        RegKey::predef(HKEY_LOCAL_MACHINE)
    } else {
        RegKey::predef(HKEY_CURRENT_USER)
    };
    schreibe_schema(&root, SCHEME, exe)?;

    // Only take over the old shared scheme if it is still free.
    // A second brand must NEVER take the links away from one already
    // installed - that is exactly why X-Remote opened
    // FreeViewer meetings.
    if SCHEME != SCHEME_ALT && !schema_belegt(SCHEME_ALT) {
        let _ = schreibe_schema(&root, SCHEME_ALT, exe);
    }
    Ok(())
}

/// Einen einzelnen Schema-Eintrag schreiben.
#[cfg(windows)]
fn schreibe_schema(
    root: &winreg::RegKey,
    schema: &str,
    exe: &std::path::Path,
) -> std::io::Result<()> {
    let (key, _) = root.create_subkey(format!(r"Software\Classes\{}", schema))?;
    key.set_value("", &format!("URL:{}", crate::brand::NAME))?;
    key.set_value("URL Protocol", &"")?;
    let (icon, _) = key.create_subkey("DefaultIcon")?;
    icon.set_value("", &format!("{},0", exe.display()))?;
    let (cmd, _) = key.create_subkey(r"shell\open\command")?;
    cmd.set_value("", &format!("\"{}\" \"%1\"", exe.display()))?;
    Ok(())
}

/// Is a program already attached to this scheme? (No matter whether HKLM or HKCU.)
#[cfg(windows)]
pub fn schema_belegt(schema: &str) -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    for r in [
        RegKey::predef(HKEY_LOCAL_MACHINE),
        RegKey::predef(HKEY_CURRENT_USER),
    ] {
        if let Ok(k) = r.open_subkey(format!(r"Software\Classes\{}\shell\open\command", schema))
        {
            if let Ok(v) = k.get_value::<String, _>("") {
                if !v.trim().is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(not(windows))]
pub fn register(_machine_wide: bool) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn register_for(_exe: &std::path::Path, _machine_wide: bool) -> std::io::Result<()> {
    Ok(())
}

/// Does the entry point to this file? (After a move it no longer matches.)
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
        // spelling with a space or slash at the end
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
    fn setup_link_tragt_den_code() {
        assert_eq!(
            parse("freeviewer://setup/zZpRpr4L"),
            Some(Action::Setup("zZpRpr4L".into()))
        );
        assert_eq!(parse("freeviewer://setup/kurz"), None);
        assert_eq!(parse("freeviewer://setup/mit leer!"), None);
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
