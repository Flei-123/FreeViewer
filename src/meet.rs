//! FreeViewer Meet - conferences.
//!
//! The media server (SFU) is already in place and serves the browser at
//! meet.fleitec.com. FreeViewer is now the umbrella: here you create a
//! meeting, see what is running and join. The video-and-audio window is
//! the same UI as in the browser, just without an address bar - exactly the
//! route Zoom and Teams take. Whoever wants to take over the other's MOUSE
//! needs the client anyway, and it is already there.
//!
//! Server interface (axum):
//!   POST   /api/meeting        {titel, termin, termin_text}  -> Meeting
//!   GET    /api/meetings                                     -> Liste ohne Passwort
//!   GET    /api/meeting/{id}                                 -> Meeting
//! Join in the browser:  <BASE>/?room=<id>&pass=<password>

use anyhow::{anyhow, Result};

/// Overridable with FV_MEET, so you can work against a test server.
pub fn base() -> String {
    std::env::var("FV_MEET").unwrap_or_else(|_| "https://meet.fleitec.com".to_string())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meeting {
    /// Anzeige-ID im Zoom-Stil: "482-913-770"
    pub id: String,
    pub titel: String,
    /// Empty in the list - it is deliberately not handed out.
    pub passphrase: String,
    pub termin_text: String,
    /// End-to-end key as text (43 characters), empty = none.
    ///
    /// IMPORTANT: It NEVER goes to the server. It is created here in the program and
    /// travels only in the fragment of the link (behind the #). Browsers never
    /// send the fragment along - that is exactly why that is the
    /// right place for it.
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
        passphrase: s("passwort"),
        termin_text: s("termin_text"),
        // Only from the OWN list; the key never comes from the server.
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

/// Creates a meeting and returns ID + password.
pub fn create(titel: &str, e2e: bool) -> Result<Meeting> {
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
    // End-to-end is a DELIBERATE choice when creating, not automatic:
    // an encrypted meeting rejects everyone whose program cannot do it
    // yet. If every new meeting were encrypted, older
    // versions would suddenly be standing at the door.
    //
    // The key is rolled HERE, not on the server. It travels on only in the
    // fragment of the link - the server never gets to see it.
    m.e2e = key_for(e2e);
    Ok(m)
}

/// A fresh key - or none at all.
///
/// Separate function so the decision can be tested: without the tick
/// NOTHING may be created, otherwise the invitation link silently carries a
/// key and the meeting rejects older versions.
pub fn key_for(e2e: bool) -> String {
    if e2e {
        crate::meete2e::Key::neu().als_text()
    } else {
        String::new()
    }
}

/// Where the OWN meetings are stored.
fn own_file() -> std::path::PathBuf {
    crate::ident::config_dir().join("meine-meetings.json")
}

fn write_own(liste: &[Meeting]) {
    let arr: Vec<serde_json::Value> = liste
        .iter()
        .map(|x| {
            serde_json::json!({
                "id": x.id,
                "titel": x.titel,
                "passwort": x.passphrase,
                "termin_text": x.termin_text,
            })
        })
        .collect();
    let p = own_file();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, serde_json::to_vec_pretty(&arr).unwrap_or_default());
}

/// The own meetings (created or joined), newest first.
///
/// WHY not from the server anymore: it used to deliver ALL meetings with
/// number and title to anyone who asked - without login. So every
/// stranger could see who was meeting for what. An overview is not worth that.
/// Now each machine remembers its own; whether one is still running,
/// the server still answers - but you have to already know the
/// number for that.
pub fn eigene() -> Vec<Meeting> {
    let text = std::fs::read_to_string(own_file()).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    v.as_array()
        .map(|a| a.iter().map(from_json).collect())
        .unwrap_or_default()
}

/// Add a meeting to the own list (or refresh it).
pub fn merken(m: &Meeting) {
    if m.id.trim().is_empty() {
        return;
    }
    let mut liste = eigene();
    liste.retain(|x| x.id != m.id);
    liste.insert(0, m.clone());
    let hoechstens: Vec<Meeting> = liste.into_iter().take(12).collect();
    write_own(&hoechstens);
}

/// Remove a meeting from the own list.
pub fn vergessen(id: &str) {
    let mut liste = eigene();
    liste.retain(|x| x.id != id);
    write_own(&liste);
}

/// The own meetings that are REALLY still running. The server is asked
/// for each number individually - expired ones drop out of the list.
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

/// Does the ID exist? (The server answers without a password.)
pub fn exists(id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() {
        return false;
    }
    get_json(&format!("{}/api/meeting/{}", base(), id))
        .map(|v| !from_json(&v).id.is_empty())
        .unwrap_or(false)
}

/// The address used to join the meeting.
pub fn join_url(id: &str, pass: &str) -> String {
    format!(
        "{}/?room={}&pass={}",
        base(),
        urlenc(id.trim()),
        urlenc(pass.trim())
    )
}

/// Attach the end-to-end key to a link - ALWAYS as a fragment.
///
/// Everything behind the # stays in the browser: it is in no request, in
/// no server log and in no history at the provider. That is exactly
/// why the server cannot know the key - and exactly why
/// it must NEVER wander into the query (?...).
pub fn with_key(url: &str, e2e: &str) -> String {
    let k = e2e.trim();
    if k.is_empty() {
        return url.to_string();
    }
    format!("{}#k={}", url, k)
}

/// Get the key from a link (fragment OR "k=" in it).
pub fn key_from_link(link: &str) -> Option<String> {
    let teil = link.split('#').nth(1)?;
    for kv in teil.split('&') {
        if let Some(v) = kv.strip_prefix("k=") {
            let v = v.trim();
            if crate::meete2e::Key::aus_text(v).is_some() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Like join_url, but with the own FreeViewer ID.
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
        m.passphrase,
        with_key(&join_url(&m.id, &m.passphrase), &m.e2e)
    )
}

/// Encode only the characters that can occur in ID and password at all
/// - an own dependency is not worth it for that.
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

/// Appends `app=1` to the meeting address as long as it is missing.
///
/// For invitation links the Meet page tries to start the installed client
/// via the freeviewer:// scheme. This window here IS already
/// the client - without the marker the page inside it would trigger the scheme again
/// and open itself in a circle.
fn mit_app_merkmal(url: &str) -> String {
    if url.contains("app=") {
        return url.to_string();
    }
    let trenn = if url.contains('?') { "&" } else { "?" };
    format!("{}{}app=1", url, trenn)
}

/// Opens the meeting in its own window without an address bar.
///
/// Edge and Chrome can do exactly that with `--app=`: a bare window that looks
/// like a program. If neither exists, we use the normal
/// browser - the main thing is that the user ends up in the meeting.
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
        // If all else fails the default browser
        let ok = std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn()
            .is_ok();
        if ok {
            return Ok(());
        }
        return Err(anyhow!("Kein Browser gefunden"));
    }
    // macOS: same idea, only different paths. If no Chrome/Edge is found,
    // `open` takes the default browser - the main thing is that the meeting opens.
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

    // Linux and the rest: look for Chrome-like in the path, otherwise xdg-open.
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
            passphrase: "geheim22".into(),
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
        assert_eq!(m.passphrase, "p");
    }

    #[test]
    fn app_merkmal_wird_genau_einmal_angehaengt() {
        let u = mit_app_merkmal("https://meet.fleitec.com/?room=1-2-3&pass=x");
        assert!(u.ends_with("&app=1"), "{}", u);
        // Already marked? Then the address stays untouched.
        let m = mit_app_merkmal(&u);
        assert_eq!(m, u);
        // With no parameter at all the address gets a question mark.
        assert_eq!(mit_app_merkmal("https://x.test/"), "https://x.test/?app=1");
    }

    #[test]
    fn base_can_be_pointed_somewhere_else() {
        // Default stays the own server
        assert!(base().starts_with("https://"));
    }
}

// ------------------------------------------------- eigener Meeting-Modus --

/// A participant, as the Meet server currently reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Participant {
    pub name: String,
    pub host: bool,
    pub mikro_aus: bool,
    pub kamera_aus: bool,
    pub hand: bool,
    /// Offers that others may control its machine (FreeViewer ID).
    pub fv: bool,
}

fn tn_aus_json(v: &serde_json::Value) -> Vec<Participant> {
    let arr = v.as_array().cloned().unwrap_or_default();
    let truthy = |x: &serde_json::Value, k: &str| {
        x.get(k).and_then(|b| b.as_bool()).unwrap_or(false)
    };
    arr.iter()
        .map(|x| Participant {
            name: x
                .get("name")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string(),
            host: truthy(x, "host"),
            mikro_aus: truthy(x, "audio_muted"),
            kamera_aus: truthy(x, "video_muted"),
            hand: truthy(x, "hand"),
            fv: truthy(x, "fv"),
        })
        .collect()
}

/// Who is currently in the room? The password belongs to the query - the ID
/// alone must not reveal anything (same rule as for deleting).
pub fn participant(id: &str, pass: &str) -> Result<Vec<Participant>> {
    let url = format!(
        "{}/api/meeting/{}/teilnehmer?pass={}",
        base(),
        urlenc(id.trim()),
        urlenc(pass.trim())
    );
    let v = get_json(&url)?;
    Ok(tn_aus_json(&v))
}

/// Microphones and cameras of this machine as (microphones, cameras).
///
/// The names are reported by the system; the browser calls them almost the same, so
/// the Meet page finds a wanted device again by its name.
pub fn geraete() -> (Vec<String>, Vec<String>) {
    (crate::audio::input_devices(), kameras())
}

/// Cameras of the machine. Windows asks the device manager (PnP), Linux
/// reads the names of the Video4Linux nodes, macOS the system report.
/// Everything runs locally - no packet goes onto the network for it.
pub fn kameras() -> Vec<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without a window - otherwise a console flashes up on every query.
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

/// Join address with everything the own meeting window can set:
/// own FreeViewer ID, wanted devices and start state of
/// microphone and camera. The Meet page reads the values and sets
/// up its preparation screen accordingly.
pub fn join_url_ex(
    id: &str,
    pass: &str,
    fvid: Option<&str>,
    mic: Option<&str>,
    cam: Option<&str>,
    is_muted: bool,
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
    if is_muted {
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
    fn join_url_ex_carries_devices_and_state() {
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
        // Missing fields are not an error - older servers deliver less.
        let leer: serde_json::Value = serde_json::from_str(r#"[{"name":"N"}]"#).unwrap();
        let t = tn_aus_json(&leer);
        assert_eq!(t.len(), 1);
        assert!(!t[0].host && !t[0].mikro_aus && !t[0].fv);
    }
}

#[cfg(test)]
mod e2e_link_tests {
    use super::*;

    /// The key MUST be in the fragment - anything else ends up in the
    /// server's logs and would thus no longer be end-to-end.
    #[test]
    fn key_follows_the_hash() {
        let k = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
        let u = with_key(&join_url("1-2-3", "pw"), k);
        let (vorn, hinten) = u.split_once('#').expect("kein Fragment");
        assert!(!vorn.contains(k), "Schluessel steht in der Abfrage: {}", vorn);
        assert_eq!(hinten, format!("k={}", k));
    }

    #[test]
    fn without_key_link_is_unchanged() {
        let u = join_url("1-2-3", "pw");
        assert_eq!(with_key(&u, ""), u);
        assert_eq!(with_key(&u, "   "), u);
    }

    #[test]
    fn key_comes_back_from_the_link() {
        let k = crate::meete2e::Key::neu().als_text();
        let u = with_key(&join_url("1-2-3", "pw"), &k);
        assert_eq!(key_from_link(&u).as_deref(), Some(k.as_str()));
    }

    /// Nonsense in the fragment must NOT pass as a key - otherwise
    /// the client decrypts with garbage and shows garbled video.
    #[test]
    fn unsinn_im_fragment_wird_abgewiesen() {
        assert!(key_from_link("https://x/?room=1#k=zukurz").is_none());
        assert!(key_from_link("https://x/?room=1#k=").is_none());
        assert!(key_from_link("https://x/?room=1").is_none());
        assert!(key_from_link("https://x/?room=1#anderes=1").is_none());
    }

    /// WITHOUT the tick when creating, no key is created - and thus
    /// the invitation carries none either. Otherwise every new meeting would be silently
    /// encrypted and older versions would be standing at the door.
    #[test]
    fn without_tick_no_key() {
        assert_eq!(key_for(false), "");
        let m = Meeting {
            id: "482-913-770".into(),
            titel: "Test".into(),
            passphrase: "geheim".into(),
            termin_text: String::new(),
            e2e: key_for(false),
        };
        let text = invite(&m);
        assert!(!text.contains("#k="), "Schluessel ohne Haken:\n{}", text);
        assert!(key_from_link(&text).is_none());
    }

    /// LIVE against the real server - runs only on request:
    ///     cargo test -- --ignored anlegen_mit_und_ohne_haken --nocapture
    /// Proves on a real meeting that the tick really decides about the
    /// key in the invitation link.
    #[test]
    #[ignore]
    fn anlegen_mit_und_ohne_haken() {
        let ohne = create("Probe ohne Haken", false).expect("Server nicht erreichbar");
        assert!(ohne.e2e.is_empty(), "Schluessel ohne Haken: {}", ohne.e2e);
        let l1 = invite(&ohne);
        assert!(!l1.contains("#k="), "{}", l1);
        println!("OHNE Haken: {}", l1.lines().last().unwrap_or(""));

        let mit = create("Probe mit Haken", true).expect("Server nicht erreichbar");
        assert!(crate::meete2e::Key::aus_text(&mit.e2e).is_some());
        let l2 = invite(&mit);
        assert!(l2.contains("#k="), "{}", l2);
        println!("MIT Haken:  {}", l2.lines().last().unwrap_or(""));
        println!("ROOM_WITHOUT={} PASS_WITHOUT={}", ohne.id, ohne.passphrase);
        println!("ROOM_WITH={} PASS_WITH={} KEY={}", mit.id, mit.passphrase, mit.e2e);
    }

    /// WITH the tick a valid key is created - and a different one every time.
    #[test]
    fn with_tick_a_fresh_key() {
        let a = key_for(true);
        let b = key_for(true);
        assert!(crate::meete2e::Key::aus_text(&a).is_some());
        assert_ne!(a, b, "zweimal derselbe Schluessel");
    }

    /// The invitation contains the link WITH the key.
    #[test]
    fn invite_carries_the_key() {
        let m = Meeting {
            id: "482-913-770".into(),
            titel: "Test".into(),
            passphrase: "geheim".into(),
            termin_text: String::new(),
            e2e: crate::meete2e::Key::neu().als_text(),
        };
        let text = invite(&m);
        assert!(text.contains("#k="), "no key in the invitation:\n{}", text);
        assert_eq!(key_from_link(&text).as_deref(), Some(m.e2e.as_str()));
    }
}
