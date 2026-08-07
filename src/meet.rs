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
    /// Ende-zu-Ende-Schluessel als Text (43 Zeichen), leer = keiner.
    ///
    /// WICHTIG: Der geht NIE an den Server. Er entsteht hier im Programm und
    /// reist nur im Fragment des Links (hinter dem #). Browser schicken das
    /// Fragment grundsaetzlich nicht mit - deshalb ist genau dort der
    /// richtige Platz dafuer.
    pub e2e: String,
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
        // Nur aus der EIGENEN Liste; vom Server kommt der Schluessel nie.
        e2e: s("e2e"),
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
    let mut m = from_json(&v);
    if m.id.is_empty() {
        return Err(anyhow!("Server hat keine Meeting-ID geliefert"));
    }
    // Ende-zu-Ende-Schluessel HIER wuerfeln, nicht am Server. Er geht nur
    // im Fragment des Links weiter - der Server bekommt ihn nie zu sehen.
    m.e2e = crate::meete2e::Schluessel::neu().als_text();
    Ok(m)
}

/// Wo die EIGENEN Meetings liegen.
fn eigene_datei() -> std::path::PathBuf {
    crate::ident::config_dir().join("meine-meetings.json")
}

fn eigene_schreiben(liste: &[Meeting]) {
    let arr: Vec<serde_json::Value> = liste
        .iter()
        .map(|x| {
            serde_json::json!({
                "id": x.id,
                "titel": x.titel,
                "passwort": x.passwort,
                "termin_text": x.termin_text,
            })
        })
        .collect();
    let p = eigene_datei();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, serde_json::to_vec_pretty(&arr).unwrap_or_default());
}

/// Die eigenen Meetings (angelegt oder beigetreten), neueste zuerst.
///
/// WARUM nicht mehr vom Server: der lieferte frueher ALLE Meetings mit
/// Nummer und Titel an jeden, der fragte - ohne Anmeldung. Damit sah jeder
/// Fremde, wer sich gerade wozu trifft. Eine Uebersicht ist das nicht wert.
/// Jetzt merkt sich jeder Rechner seine eigenen; ob eines noch laeuft,
/// beantwortet der Server weiterhin - dafuer muss man die Nummer aber
/// bereits kennen.
pub fn eigene() -> Vec<Meeting> {
    let text = std::fs::read_to_string(eigene_datei()).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    v.as_array()
        .map(|a| a.iter().map(from_json).collect())
        .unwrap_or_default()
}

/// Ein Meeting in die eigene Liste aufnehmen (oder auffrischen).
pub fn merken(m: &Meeting) {
    if m.id.trim().is_empty() {
        return;
    }
    let mut liste = eigene();
    liste.retain(|x| x.id != m.id);
    liste.insert(0, m.clone());
    let hoechstens: Vec<Meeting> = liste.into_iter().take(12).collect();
    eigene_schreiben(&hoechstens);
}

/// Ein Meeting aus der eigenen Liste streichen.
pub fn vergessen(id: &str) {
    let mut liste = eigene();
    liste.retain(|x| x.id != id);
    eigene_schreiben(&liste);
}

/// Die eigenen Meetings, die WIRKLICH noch laufen. Der Server wird je
/// Nummer einzeln gefragt - abgelaufene fliegen aus der Liste.
pub fn list() -> Result<Vec<Meeting>> {
    let mut aus = Vec::new();
    for m in eigene() {
        if exists(&m.id) {
            aus.push(m);
        } else {
            vergessen(&m.id);
        }
    }
    Ok(aus)
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

/// Den Ende-zu-Ende-Schluessel an einen Link haengen - IMMER als Fragment.
///
/// Alles hinter dem # bleibt im Browser: es steht in keiner Anfrage, in
/// keinem Server-Protokoll und in keinem Verlauf beim Anbieter. Genau
/// deshalb kann der Server den Schluessel nicht kennen - und genau deshalb
/// darf er auch NIE in die Abfrage (?...) wandern.
pub fn mit_schluessel(url: &str, e2e: &str) -> String {
    let k = e2e.trim();
    if k.is_empty() {
        return url.to_string();
    }
    format!("{}#k={}", url, k)
}

/// Den Schluessel aus einem Link holen (Fragment ODER "k=" darin).
pub fn schluessel_aus_link(link: &str) -> Option<String> {
    let teil = link.split('#').nth(1)?;
    for kv in teil.split('&') {
        if let Some(v) = kv.strip_prefix("k=") {
            let v = v.trim();
            if crate::meete2e::Schluessel::aus_text(v).is_some() {
                return Some(v.to_string());
            }
        }
    }
    None
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
        format!("{} Meet", crate::brand::NAME)
    } else {
        m.titel.trim().to_string()
    };
    format!(
        "{}\nMeeting-ID: {}\nPasswort: {}\n{}",
        titel,
        m.id,
        m.passwort,
        mit_schluessel(&join_url(&m.id, &m.passwort), &m.e2e)
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

/// Haengt `app=1` an die Meeting-Adresse, solange es noch fehlt.
///
/// Die Meet-Seite versucht bei Einladungslinks, den installierten Client
/// ueber das freeviewer://-Schema zu starten. Dieses Fenster hier IST schon
/// der Client - ohne das Merkmal wuerde die Seite darin das Schema erneut
/// anstossen und sich selbst im Kreis oeffnen.
fn mit_app_merkmal(url: &str) -> String {
    if url.contains("app=") {
        return url.to_string();
    }
    let trenn = if url.contains('?') { "&" } else { "?" };
    format!("{}{}app=1", url, trenn)
}

/// Oeffnet das Meeting in einem eigenen Fenster ohne Adressleiste.
///
/// Edge und Chrome koennen mit `--app=` genau das: ein nacktes Fenster, das
/// wie ein Programm aussieht. Gibt es beide nicht, nehmen wir den normalen
/// Browser - Hauptsache, der Nutzer landet im Meeting.
pub fn open_window(url: &str) -> Result<()> {
    let markiert = mit_app_merkmal(url);
    let url = markiert.as_str();
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
            e2e: String::new(),
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
    fn app_merkmal_wird_genau_einmal_angehaengt() {
        let u = mit_app_merkmal("https://meet.fleitec.com/?room=1-2-3&pass=x");
        assert!(u.ends_with("&app=1"), "{}", u);
        // Schon markiert? Dann bleibt die Adresse unangetastet.
        let m = mit_app_merkmal(&u);
        assert_eq!(m, u);
        // Ganz ohne Parameter bekommt die Adresse ein Fragezeichen.
        assert_eq!(mit_app_merkmal("https://x.test/"), "https://x.test/?app=1");
    }

    #[test]
    fn base_can_be_pointed_somewhere_else() {
        // Standard bleibt der eigene Server
        assert!(base().starts_with("https://"));
    }
}

// ------------------------------------------------- eigener Meeting-Modus --

/// Ein Teilnehmer, wie ihn der Meet-Server gerade meldet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Teilnehmer {
    pub name: String,
    pub host: bool,
    pub mikro_aus: bool,
    pub kamera_aus: bool,
    pub hand: bool,
    /// Bietet an, dass andere seine Maschine steuern duerfen (FreeViewer-ID).
    pub fv: bool,
}

fn tn_aus_json(v: &serde_json::Value) -> Vec<Teilnehmer> {
    let arr = v.as_array().cloned().unwrap_or_default();
    let wahr = |x: &serde_json::Value, k: &str| {
        x.get(k).and_then(|b| b.as_bool()).unwrap_or(false)
    };
    arr.iter()
        .map(|x| Teilnehmer {
            name: x
                .get("name")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string(),
            host: wahr(x, "host"),
            mikro_aus: wahr(x, "audio_muted"),
            kamera_aus: wahr(x, "video_muted"),
            hand: wahr(x, "hand"),
            fv: wahr(x, "fv"),
        })
        .collect()
}

/// Wer sitzt gerade im Raum? Das Passwort gehoert zur Abfrage - die ID
/// allein darf nichts verraten (gleiche Regel wie beim Loeschen).
pub fn teilnehmer(id: &str, pass: &str) -> Result<Vec<Teilnehmer>> {
    let url = format!(
        "{}/api/meeting/{}/teilnehmer?pass={}",
        base(),
        urlenc(id.trim()),
        urlenc(pass.trim())
    );
    let v = get_json(&url)?;
    Ok(tn_aus_json(&v))
}

/// Mikrofone und Kameras dieses Rechners als (Mikrofone, Kameras).
///
/// Die Namen meldet das System; der Browser nennt sie fast gleich, darum
/// findet die Meet-Seite ein Wunschgeraet anhand seines Namens wieder.
pub fn geraete() -> (Vec<String>, Vec<String>) {
    (crate::audio::input_devices(), kameras())
}

/// Kameras des Rechners. Windows fragt die Geraeteverwaltung (PnP), Linux
/// liest die Namen der Video4Linux-Knoten, macOS den Systembericht.
/// Alles laeuft lokal - dafuer geht kein Paket ins Netz.
pub fn kameras() -> Vec<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Ohne Fenster - sonst blitzt bei jeder Abfrage eine Konsole auf.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let ps = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-PnpDevice -Class CAMERA -Status OK -ErrorAction SilentlyContinue | ForEach-Object { $_.FriendlyName }",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Ok(out) = ps {
            let text = String::from_utf8_lossy(&out.stdout);
            return text
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
        }
        return Vec::new();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut v = Vec::new();
        if let Ok(rd) = std::fs::read_dir("/sys/class/video4linux") {
            for e in rd.flatten() {
                if let Ok(name) = std::fs::read_to_string(e.path().join("name")) {
                    let name = name.trim().to_string();
                    if !name.is_empty() {
                        v.push(name);
                    }
                }
            }
        }
        v.sort();
        v.dedup();
        return v;
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("system_profiler")
            .arg("SPCameraDataType")
            .output();
        if let Ok(out) = out {
            let text = String::from_utf8_lossy(&out.stdout);
            return text
                .lines()
                .map(|l| l.trim())
                .filter(|l| l.ends_with(':') && l.len() > 2)
                .map(|l| l.trim_end_matches(':').to_string())
                .filter(|l| !l.starts_with("Camera") && !l.starts_with("Kamera"))
                .collect();
        }
        return Vec::new();
    }
    #[cfg(not(any(windows, unix)))]
    {
        Vec::new()
    }
}

/// Beitritts-Adresse mit allem, was das eigene Meeting-Fenster einstellen
/// kann: eigene FreeViewer-ID, Wunschgeraete und Startzustand von
/// Mikrofon und Kamera. Die Meet-Seite liest die Werte aus und stellt
/// ihren Vorbereitungsbildschirm danach.
pub fn join_url_ex(
    id: &str,
    pass: &str,
    fvid: Option<&str>,
    mic: Option<&str>,
    cam: Option<&str>,
    stumm: bool,
    ohne_video: bool,
) -> String {
    let mut u = join_url(id, pass);
    if let Some(f) = fvid {
        let ziffern: String = f.chars().filter(|c| c.is_ascii_digit()).take(12).collect();
        if !ziffern.is_empty() {
            u = format!("{}&fv={}", u, ziffern);
        }
    }
    if let Some(m) = mic {
        if !m.trim().is_empty() {
            u = format!("{}&mic={}", u, urlenc(m.trim()));
        }
    }
    if let Some(c) = cam {
        if !c.trim().is_empty() {
            u = format!("{}&cam={}", u, urlenc(c.trim()));
        }
    }
    if stumm {
        u = format!("{}&mute=1", u);
    }
    if ohne_video {
        u = format!("{}&novideo=1", u);
    }
    u
}

#[cfg(test)]
mod tests_meetwin {
    use super::*;

    #[test]
    fn join_url_ex_traegt_geraete_und_zustand() {
        let u = join_url_ex(
            "1-2-3",
            "pw",
            Some("123 456 789"),
            Some("Mikrofon (Realtek)"),
            Some("HD Cam"),
            true,
            true,
        );
        assert!(u.contains("room=1-2-3"), "{}", u);
        assert!(u.contains("&pass=pw"), "{}", u);
        assert!(u.contains("&fv=123456789"), "{}", u);
        assert!(u.contains("&mic=Mikrofon%20%28Realtek%29"), "{}", u);
        assert!(u.contains("&cam=HD%20Cam"), "{}", u);
        assert!(u.contains("&mute=1"), "{}", u);
        assert!(u.ends_with("&novideo=1"), "{}", u);
    }

    #[test]
    fn join_url_ex_laesst_leeres_weg() {
        let u = join_url_ex("1-2-3", "pw", None, None, Some("  "), false, false);
        assert_eq!(u, join_url("1-2-3", "pw"));
    }

    #[test]
    fn teilnehmerliste_wird_gelesen() {
        let v: serde_json::Value = serde_json::from_str(
            r#"[
              {"name":"Justin","host":true,"audio_muted":false,"video_muted":true,"hand":false,"fv":true},
              {"name":"Gast 7","host":false,"audio_muted":true,"video_muted":false,"hand":true,"fv":false}
            ]"#,
        )
        .unwrap();
        let t = tn_aus_json(&v);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].name, "Justin");
        assert!(t[0].host && t[0].kamera_aus && t[0].fv && !t[0].mikro_aus);
        assert_eq!(t[1].name, "Gast 7");
        assert!(t[1].mikro_aus && t[1].hand && !t[1].host);
        // Fehlende Felder sind kein Fehler - aeltere Server liefern weniger.
        let leer: serde_json::Value = serde_json::from_str(r#"[{"name":"N"}]"#).unwrap();
        let t = tn_aus_json(&leer);
        assert_eq!(t.len(), 1);
        assert!(!t[0].host && !t[0].mikro_aus && !t[0].fv);
    }
}

#[cfg(test)]
mod e2e_link_tests {
    use super::*;

    /// Der Schluessel MUSS im Fragment stehen - alles andere landet in den
    /// Protokollen des Servers und waere damit kein Ende-zu-Ende mehr.
    #[test]
    fn der_schluessel_steht_hinter_der_raute() {
        let k = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
        let u = mit_schluessel(&join_url("1-2-3", "pw"), k);
        let (vorn, hinten) = u.split_once('#').expect("kein Fragment");
        assert!(!vorn.contains(k), "Schluessel steht in der Abfrage: {}", vorn);
        assert_eq!(hinten, format!("k={}", k));
    }

    #[test]
    fn ohne_schluessel_bleibt_der_link_unveraendert() {
        let u = join_url("1-2-3", "pw");
        assert_eq!(mit_schluessel(&u, ""), u);
        assert_eq!(mit_schluessel(&u, "   "), u);
    }

    #[test]
    fn schluessel_kommt_aus_dem_link_zurueck() {
        let k = crate::meete2e::Schluessel::neu().als_text();
        let u = mit_schluessel(&join_url("1-2-3", "pw"), &k);
        assert_eq!(schluessel_aus_link(&u).as_deref(), Some(k.as_str()));
    }

    /// Unsinn im Fragment darf NICHT als Schluessel durchgehen - sonst
    /// entschluesselt der Client mit Muell und zeigt Bildsalat.
    #[test]
    fn unsinn_im_fragment_wird_abgewiesen() {
        assert!(schluessel_aus_link("https://x/?room=1#k=zukurz").is_none());
        assert!(schluessel_aus_link("https://x/?room=1#k=").is_none());
        assert!(schluessel_aus_link("https://x/?room=1").is_none());
        assert!(schluessel_aus_link("https://x/?room=1#anderes=1").is_none());
    }

    /// Die Einladung enthaelt den Link MIT Schluessel.
    #[test]
    fn die_einladung_traegt_den_schluessel() {
        let m = Meeting {
            id: "482-913-770".into(),
            titel: "Test".into(),
            passwort: "geheim".into(),
            termin_text: String::new(),
            e2e: crate::meete2e::Schluessel::neu().als_text(),
        };
        let text = invite(&m);
        assert!(text.contains("#k="), "kein Schluessel in der Einladung:\n{}", text);
        assert_eq!(schluessel_aus_link(&text).as_deref(), Some(m.e2e.as_str()));
    }
}
