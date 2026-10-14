//! Feste Passwoerter dieses Rechners.
//!
//! Das Sitzungspasswort auf der Startseite wird bei jedem Start neu
//! gewuerfelt - gut fuer "ruf mich an und sag mir die Zahlen", schlecht fuer
//! unbeaufsichtigten Zugriff. Darum kann man hier beliebig viele feste
//! Passwoerter hinterlegen (mit einer Bezeichnung, damit man weiss, wem man
//! welches gegeben hat). Jedes davon oeffnet diesen PC.
//!
//! Storage (r10/r64): the host has to recompute Argon2id(password, session
//! salt) for every session, so a one-way hash is impossible without a
//! protocol change (needs a PAKE). The passwords are therefore stored
//! ENCRYPTED in `<config>/passwords.json` (see `secstore`: Windows DPAPI,
//! machine scope; elsewhere AES-GCM with a key derived from the machine
//! identity; file mode 0600 on Unix). Old plain text files are read and
//! rewritten sealed on first load. A copied file is worthless on another PC;
//! anybody who can run code as a user on THIS PC can still open it.

use std::path::PathBuf;

/// Mehr als das ist keine Verwaltung mehr, sondern ein Datenleck.
pub const MAX: usize = 10;
/// Kuerzer nimmt ein Angreifer im Vorbeigehen mit.
pub const MIN_LEN: usize = 8;
// Existing shorter passwords keep working; the limit only applies to new ones.

#[derive(Clone, PartialEq, Default)]
pub struct Entry {
    /// Frei waehlbare Bezeichnung ("Handy", "Papa", "Buero").
    pub label: String,
    pub pw: String,
}

fn path() -> PathBuf {
    crate::ident::config_dir().join("passwords.json")
}

/// Liest die Liste. Kaputte Datei = leere Liste, nie ein Absturz.
pub fn load() -> Vec<Entry> {
    let raw = match std::fs::read_to_string(path()) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let s = raw.trim_start_matches('\u{feff}');
    let v: serde_json::Value = match serde_json::from_str(s) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let arr = match v.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut needs_migration = false;
    let mut skipped = false;
    for item in arr.iter().take(MAX) {
        let stored = item.get("pw").and_then(|x| x.as_str()).unwrap_or("");
        if crate::secstore::needs_reseal(stored) {
            needs_migration = true;
        }
        // a sealed value that cannot be opened here (other machine) is skipped
        let pw = match crate::secstore::open(stored) {
            Some(p) if !p.is_empty() => p,
            _ => {
                skipped = true;
                continue;
            }
        };
        let label = item
            .get("label")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_control())
            .take(40)
            .collect();
        out.push(Entry { label, pw });
    }
    // never rewrite while an entry could not be opened (other machine,
    // Keychain locked): saving would drop it for good
    if needs_migration && !skipped {
        // old plain text file: rewrite it sealed, best effort
        let _ = save(&out);
    }
    out
}

pub fn save(list: &[Entry]) -> std::io::Result<()> {
    let mut arr = Vec::new();
    for e in list.iter().take(MAX) {
        if e.pw.is_empty() {
            continue;
        }
        let mut m = serde_json::Map::new();
        m.insert("label".into(), serde_json::Value::from(e.label.clone()));
        m.insert(
            "pw".into(),
            serde_json::Value::from(crate::secstore::seal(&e.pw)),
        );
        arr.push(serde_json::Value::Object(m));
    }
    std::fs::create_dir_all(crate::ident::config_dir())?;
    let text = serde_json::to_string_pretty(&serde_json::Value::Array(arr))
        .unwrap_or_else(|_| "[]".to_string());
    crate::secstore::write_private(&path(), &text)
}

/// Nur die Passwoerter - das braucht die Anmeldung.
pub fn candidates() -> Vec<String> {
    load().into_iter().map(|e| e.pw).collect()
}

/// Passt das Passwort in die Liste? Gibt den Grund zurueck, wenn nicht.
/// `None` heisst: alles in Ordnung.
pub fn why_not(list: &[Entry], pw: &str) -> Option<&'static str> {
    if pw.trim().chars().count() < MIN_LEN {
        return Some("set.pw_min");
    }
    if list.len() >= MAX {
        return Some("set.pw_max");
    }
    if list.iter().any(|e| e.pw == pw) {
        return Some("set.pw_dup");
    }
    None
}

/// Punkte statt Zeichen, gleiche Laenge wie das Original (max 16 Punkte).
pub fn masked(pw: &str) -> String {
    let n = pw.chars().count().clamp(1, 16);
    "\u{2022}".repeat(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_password_is_refused() {
        assert_eq!(why_not(&[], "kurz"), Some("set.pw_min"));
        assert_eq!(why_not(&[], "1234567"), Some("set.pw_min"));
        assert_eq!(why_not(&[], "12345678"), None);
        assert_eq!(why_not(&[], "langgenug"), None);
    }

    #[test]
    fn duplicates_are_refused() {
        let list = vec![Entry {
            label: "a".into(),
            pw: "geheim123".into(),
        }];
        assert_eq!(why_not(&list, "geheim123"), Some("set.pw_dup"));
        assert_eq!(why_not(&list, "geheim124"), None);
    }

    #[test]
    fn full_list_is_refused() {
        let list: Vec<Entry> = (0..MAX)
            .map(|i| Entry {
                label: format!("{}", i),
                pw: format!("passwort{}", i),
            })
            .collect();
        assert_eq!(why_not(&list, "nochwas123"), Some("set.pw_max"));
    }

    #[test]
    fn mask_hides_everything_but_the_length() {
        assert_eq!(masked("abc").chars().count(), 3);
        assert!(!masked("geheim").contains('g'));
        // sehr lange Passwoerter verraten ihre Laenge nicht
        assert_eq!(masked(&"x".repeat(40)).chars().count(), 16);
    }

    fn own_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fv-pwl-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        crate::ident::set_test_config_dir(d.clone());
        d
    }

    #[test]
    fn list_is_stored_sealed_and_loads_back() {
        let d = own_dir("sealed");
        let list = vec![Entry {
            label: "phone".into(),
            pw: "very-secret-pw".into(),
        }];
        save(&list).unwrap();
        let raw = std::fs::read_to_string(d.join("passwords.json")).unwrap();
        assert!(!raw.contains("very-secret-pw"), "plain text on disk");
        assert!(raw.contains("phone"));
        assert!(load() == list);
    }

    #[test]
    fn legacy_plain_list_is_migrated() {
        let d = own_dir("migrate");
        std::fs::write(
            d.join("passwords.json"),
            r#"[{"label":"old","pw":"legacy-plain-pw"}]"#,
        )
        .unwrap();
        let l = load();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].pw, "legacy-plain-pw");
        let raw = std::fs::read_to_string(d.join("passwords.json")).unwrap();
        assert!(!raw.contains("legacy-plain-pw"), "not migrated");
        assert_eq!(load()[0].pw, "legacy-plain-pw");
        assert_eq!(candidates(), vec!["legacy-plain-pw".to_string()]);
    }

    #[test]
    fn unopenable_sealed_entry_is_skipped() {
        let d = own_dir("foreign");
        std::fs::write(
            d.join("passwords.json"),
            r#"[{"label":"x","pw":"fvk1:00ff"}]"#,
        )
        .unwrap();
        assert!(load().is_empty());
    }
}
