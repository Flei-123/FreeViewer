//! Feste Passwoerter dieses Rechners.
//!
//! Das Sitzungspasswort auf der Startseite wird bei jedem Start neu
//! gewuerfelt - gut fuer "ruf mich an und sag mir die Zahlen", schlecht fuer
//! unbeaufsichtigten Zugriff. Darum kann man hier beliebig viele feste
//! Passwoerter hinterlegen (mit einer Bezeichnung, damit man weiss, wem man
//! welches gegeben hat). Jedes davon oeffnet diesen PC.
//!
//! EHRLICH GESAGT: die Passwoerter stehen im Klartext in
//! `<config>/passwords.json`. Anders geht es nicht - der Rechner muss aus dem
//! Passwort denselben Argon2-Schluessel ableiten wie die Gegenseite, ein Hash
//! reicht dafuer nicht. Die Datei liegt im Konfigurationsordner, den nur
//! Administratoren beschreiben koennen. Wer Zugriff auf diese Datei hat, hat
//! ohnehin schon den Rechner.

use std::path::PathBuf;

/// Mehr als das ist keine Verwaltung mehr, sondern ein Datenleck.
pub const MAX: usize = 10;
/// Kuerzer nimmt ein Angreifer im Vorbeigehen mit.
pub const MIN_LEN: usize = 6;

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
    for item in arr.iter().take(MAX) {
        let pw = item
            .get("pw")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if pw.is_empty() {
            continue;
        }
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
        m.insert("pw".into(), serde_json::Value::from(e.pw.clone()));
        arr.push(serde_json::Value::Object(m));
    }
    std::fs::create_dir_all(crate::ident::config_dir())?;
    let text = serde_json::to_string_pretty(&serde_json::Value::Array(arr))
        .unwrap_or_else(|_| "[]".to_string());
    std::fs::write(path(), text)
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
}
