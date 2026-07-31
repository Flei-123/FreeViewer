//! FreeViewer Meet - Konferenzen.
//!
//! Der Medienserver (SFU) steht schon und bedient den Browser unter
//! meet.fleitec.com. FreeViewer ist ab jetzt das Dach: hier legt man ein
//! Meeting an, sieht was laeuft und tritt bei. Das Bild-und-Ton-Fenster ist
//! die gleiche Oberflaeche wie im Browser, nur ohne Adressleiste - genau der
//! Weg, den Zoom und Teams gehen. Wer die MAUS des anderen uebernehmen will,
//! braucht ohnehin den Client, und der ist ja schon da.
//!
//! Schnittstelle des Servers (axum):
//!   POST   /api/meeting        {titel, termin, termin_text}  -> Meeting
//!   GET    /api/meetings                                     -> Liste ohne Passwort
//!   GET    /api/meeting/{id}                                 -> Meeting
//! Beitritt im Browser:  <BASE>/?room=<id>&pass=<passwort>

use anyhow::{anyhow, Result};

/// Ueberschreibbar mit FV_MEET, damit man gegen einen Testserver arbeiten kann.
pub fn base() -> String {
    std::env::var("FV_MEET").unwrap_or_else(|_| "https://meet.fleitec.com".to_string())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meeting {
    /// Anzeige-ID im Zoom-Stil: "482-913-770"
    pub id: String,
    pub titel: String,
    /// Bei der Liste leer - die gibt es bewusst nicht heraus.
    pub passwort: String,
    pub termin_text: String,
}

fn from_json(v: &serde_json::Value) -> Meeting {
    let s = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Meeting {
        id: s("id"),
        titel: s("titel"),
        passwort: s("passwort"),
        termin_text: s("termin_text"),
    }
}

fn get_json(url: &str) -> Result<serde_json::Value> {
    let mut resp = ureq::get(url).call().map_err(|e| anyhow!("{}", e))?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow!("{}", e))?;
    serde_json::from_str(&body).map_err(|e| anyhow!("{}", e))
}

/// Legt ein Meeting an und liefert ID + Passwort zurueck.
pub fn create(titel: &str) -> Result<Meeting> {
    let url = format!("{}/api/meeting", base());
    let payload = serde_json::json!({
        "titel": titel,
        "termin": serde_json::Value::Null,
        "termin_text": "",
    });
    let body_out = serde_json::to_string(&payload).map_err(|e| anyhow!("{}", e))?;
    let mut resp = ureq::post(&url)
        .header("content-type", "application/json")
        .send(&body_out)
        .map_err(|e| anyhow!("{}", e))?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow!("{}", e))?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| anyhow!("{}", e))?;
    let m = from_json(&v);
    if m.id.is_empty() {
        return Err(anyhow!("Server hat keine Meeting-ID geliefert"));
    }
    Ok(m)
}

/// Was gerade im Verzeichnis steht (ohne Passwoerter).
pub fn list() -> Result<Vec<Meeting>> {
    let v = get_json(&format!("{}/api/meetings", base()))?;
    let arr = v.as_array().cloned().unwrap_or_default();
    // Neueste zuerst und hoechstens ein Dutzend - der Server haelt 24 Stunden vor.
    let mut list: Vec<Meeting> = arr.iter().map(from_json).collect();
    list.reverse();
    Ok(list.into_iter().take(12).collect())
}

/// Gibt es die ID? (Der Server antwortet ohne Passwort.)
pub fn exists(id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() {
        return false;
    }
    get_json(&format!("{}/api/meeting/{}", base(), id))
        .map(|v| !from_json(&v).id.is_empty())
        .unwrap_or(false)
}

/// Die Adresse, mit der man dem Meeting beitritt.
pub fn join_url(id: &str, pass: &str) -> String {
    format!(
        "{}/?room={}&pass={}",
        base(),
        urlenc(id.trim()),
        urlenc(pass.trim())
    )
}

/// Wie join_url, aber mit der eigenen FreeViewer-ID.
pub fn join_url_with_id(id: &str, pass: &str, fvid: &str) -> String {
    let ziffern: String = fvid.chars().filter(|c| c.is_ascii_digit()).collect();
    if ziffern.is_empty() {
        return join_url(id, pass);
    }
    format!("{}&fv={}", join_url(id, pass), ziffern)
}

/// Text zum Weitergeben - Telefon, Chat, E-Mail.
pub fn invite(m: &Meeting) -> String {
    let titel = if m.titel.trim().is_empty() {
        "FreeViewer Meet".to_string()
    } else {
        m.titel.trim().to_string()
    };
    format!(
        "{}\nMeeting-ID: {}\nPasswort: {}\n{}",
        titel,
        m.id,
        m.passwort,
        join_url(&m.id, &m.passwort)
    )
}

/// Nur die Zeichen kodieren, die in ID und Passwort ueberhaupt vorkommen
/// koennen - eine eigene Abhaengigkeit lohnt dafuer nicht.
fn urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            _ => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    out
}

/// Oeffnet das Meeting in einem eigenen Fenster ohne Adressleiste.
///
/// Edge und Chrome koennen mit `--app=` genau das: ein nacktes Fenster, das
/// wie ein Programm aussieht. Gibt es beide nicht, nehmen wir den normalen
/// Browser - Hauptsache, der Nutzer landet im Meeting.
pub fn open_window(url: &str) -> Result<()> {
    #[cfg(windows)]
    {
        let candidates = [
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ];
        for exe in candidates.iter() {
            if std::path::Path::new(exe).exists() {
                let ok = std::process::Command::new(exe)
                    .arg(format!("--app={}", url))
                    .arg("--window-size=1280,800")
                    .spawn()
                    .is_ok();
                if ok {
                    return Ok(());
                }
            }
        }
        // Notfalls der Standardbrowser
        let ok = std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn()
            .is_ok();
        if ok {
            return Ok(());
        }
        return Err(anyhow!("Kein Browser gefunden"));
    }
    // macOS: dieselbe Idee, nur andere Pfade. Findet sich kein Chrome/Edge,
    // uebernimmt `open` den Standardbrowser - Hauptsache, das Meeting geht auf.
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ];
        for exe in candidates.iter() {
            if std::path::Path::new(exe).exists()
                && std::process::Command::new(exe)
                    .arg(format!("--app={}", url))
                    .arg("--window-size=1280,800")
                    .spawn()
                    .is_ok()
            {
                return Ok(());
            }
        }
        if std::process::Command::new("open").arg(url).spawn().is_ok() {
            return Ok(());
        }
        return Err(anyhow!("Kein Browser gefunden"));
    }

    // Linux und der Rest: Chrome-artige im Pfad suchen, sonst xdg-open.
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for exe in [
            "google-chrome",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
        ] {
            if std::process::Command::new(exe)
                .arg(format!("--app={}", url))
                .arg("--window-size=1280,800")
                .spawn()
                .is_ok()
            {
                return Ok(());
            }
        }
        if std::process::Command::new("xdg-open").arg(url).spawn().is_ok() {
            return Ok(());
        }
        return Err(anyhow!("Kein Browser gefunden"));
    }

    #[cfg(not(any(windows, unix)))]
    {
        let _ = url;
        Err(anyhow!("Auf diesem System nicht unterstuetzt"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_url_has_room_and_pass() {
        let u = join_url("482-913-770", "abc23xyz");
        assert!(u.contains("?room=482-913-770"), "{}", u);
        assert!(u.ends_with("&pass=abc23xyz"), "{}", u);
    }

    #[test]
    fn url_encoding_only_touches_what_it_must() {
        assert_eq!(urlenc("482-913-770"), "482-913-770");
        assert_eq!(urlenc("a b"), "a%20b");
        assert_eq!(urlenc("k&d"), "k%26d");
    }

    #[test]
    fn invite_contains_everything_to_join() {
        let m = Meeting {
            id: "111-222-333".into(),
            titel: "Testrunde".into(),
            passwort: "geheim22".into(),
            termin_text: String::new(),
        };
        let t = invite(&m);
        assert!(t.contains("Testrunde"));
        assert!(t.contains("111-222-333"));
        assert!(t.contains("geheim22"));
        assert!(t.contains("room=111-222-333"));
    }

    #[test]
    fn meeting_is_read_from_the_servers_answer() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"id":"1-2-3","titel":"T","passwort":"p","termin":null,"termin_text":"","erstellt":1}"#,
        )
        .unwrap();
        let m = from_json(&v);
        assert_eq!(m.id, "1-2-3");
        assert_eq!(m.passwort, "p");
    }

    #[test]
    fn base_can_be_pointed_somewhere_else() {
        // Standard bleibt der eigene Server
        assert!(base().starts_with("https://"));
    }
}
