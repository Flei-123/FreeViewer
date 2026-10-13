//! Self update: ask the relay for the newest published build, verify it and
//! replace the running binary.
//!
//! Windows cannot overwrite a running .exe, but it can *rename* it. So the
//! sequence is: download -> check SHA-256 -> rename running exe to ".old" ->
//! move the fresh one into its place -> start it -> exit. The next start
//! deletes the ".old" leftover.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::shared::Shared;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const FEED: &str = crate::brand::FEED;
/// How often a running instance looks for a new build.
const EVERY: Duration = Duration::from_secs(30 * 60);
const MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Ed25519 public key (64 hex chars) that signed update feeds must verify
/// against. Branded builds can override it with `FV_UPDATE_PUBKEY` at build
/// time. The private key lives only on the release machine (never in git);
/// see `tools/sign-release/` and `docs/UPDATE-SIGNING.md`.
pub const UPDATE_PUBKEY: &str = match option_env!("FV_UPDATE_PUBKEY") {
    Some(s) => s,
    None => "6fd3beb25185415730f99e3222f6ded7834e82339d6ba2902ecc8308469e97cd",
};

/// Migration switch (see `docs/UPDATE-SIGNING.md`).
/// * `false` (phase 1, default): a feed WITHOUT `sig` is still accepted (old
///   releases keep working), but a feed WITH a `sig` that does not verify is
///   always rejected.
/// * `true` (phase 2, build with `FV_REQUIRE_SIG=1`): unsigned feeds are
///   rejected too.
pub const REQUIRE_SIGNATURE: bool = match option_env!("FV_REQUIRE_SIG") {
    Some(s) => s.len() == 1 && s.as_bytes()[0] == b'1',
    None => false,
};

/// Platform label inside the signed message: the signature of the Windows
/// package can never be replayed for the Mac package (and vice versa).
#[cfg(target_os = "macos")]
const PLATFORM: &str = "mac";
#[cfg(not(target_os = "macos"))]
const PLATFORM: &str = "win";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Release {
    pub version: String,
    pub sha256: String,
    pub url: String,
    pub notes: String,
    pub size: u64,
    /// Hex Ed25519 signature over `signed_message` (empty = feed is unsigned).
    pub sig: String,
}

/// The exact bytes that are signed. Must match `tools/sign-release`:
/// `freeviewer-update-v1\n<platform>\n<version>\n<sha256 hex>\n<size>`.
/// The SHA-256 binds the package bytes, so signing the hash signs the file.
pub fn signed_message(platform: &str, version: &str, sha256: &str, size: u64) -> Vec<u8> {
    format!(
        "freeviewer-update-v1\n{}\n{}\n{}\n{}",
        platform,
        version,
        sha256.to_lowercase(),
        size
    )
    .into_bytes()
}

/// Verifies one Ed25519 signature. Any malformed input is an error.
pub fn verify_signature(pubkey_hex: &str, message: &[u8], sig_hex: &str) -> Result<()> {
    let pk: [u8; 32] = hex::decode(pubkey_hex)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| anyhow!("update public key is malformed"))?;
    let key = VerifyingKey::from_bytes(&pk).map_err(|_| anyhow!("update public key is invalid"))?;
    let sig: [u8; 64] = hex::decode(sig_hex)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| anyhow!("update signature is malformed"))?;
    key.verify(message, &Signature::from_bytes(&sig))
        .map_err(|_| anyhow!("update signature is invalid"))
}

/// Policy: signed feeds must verify; unsigned feeds only pass while
/// `require` is false (migration phase 1).
pub fn check_release_signature(
    rel: &Release,
    platform: &str,
    pubkey_hex: &str,
    require: bool,
) -> Result<()> {
    if rel.sig.is_empty() {
        return if require {
            Err(anyhow!("update feed is not signed - update refused"))
        } else {
            Ok(())
        };
    }
    let msg = signed_message(platform, &rel.version, &rel.sha256, rel.size);
    verify_signature(pubkey_hex, &msg, &rel.sig)
}

/// "0.6.1" is newer than "0.6", "0.10.0" is newer than "0.9.9".
pub fn newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let a = parse(remote);
    let b = parse(local);
    for i in 0..a.len().max(b.len()) {
        let x = *a.get(i).unwrap_or(&0);
        let y = *b.get(i).unwrap_or(&0);
        if x != y {
            return x > y;
        }
    }
    false
}

fn fetch(url: &str) -> Result<Vec<u8>> {
    let mut resp = ureq::get(url)
        .call()
        .map_err(|e| anyhow!("{}: {}", url, e))?;
    let body = resp
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .map_err(|e| anyhow!("Download: {}", e))?;
    Ok(body)
}

/// Asks the relay what the newest build is.
pub fn check() -> Result<Release> {
    let body = fetch(FEED)?;
    parse_feed(&body)
}

/// Parses + verifies a feed document (split from `check` so tests can feed it).
pub fn parse_feed(body: &[u8]) -> Result<Release> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let s = |k: &str| -> String {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    // Jede Plattform hat ihr eigenes Paket: "url" ist Windows, "mac_url"
    // der Mac. Nie wieder die falsche Datei ziehen - eine Windows-Exe wuerde
    // einen Mac sonst beim Tausch zerstoeren.
    #[cfg(target_os = "macos")]
    let (url, hash, size, sig) = (
        s("mac_url"),
        s("mac_sha256").to_lowercase(),
        v.get("mac_size").and_then(|x| x.as_u64()).unwrap_or(0),
        s("mac_sig").to_lowercase(),
    );
    #[cfg(not(target_os = "macos"))]
    let (url, hash, size, sig) = (
        s("url"),
        s("sha256").to_lowercase(),
        v.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
        s("sig").to_lowercase(),
    );
    let rel = Release {
        version: s("version"),
        sha256: hash,
        url,
        notes: s("notes"),
        size,
        sig,
    };
    // Fehlt das Paket der eigenen Plattform: kein Update, kein Fehler -
    // einfach nichts anbieten.
    if rel.url.is_empty() {
        return Ok(Release {
            version: VERSION.to_string(),
            ..rel
        });
    }
    if rel.version.is_empty() || rel.sha256.len() != 64 {
        return Err(anyhow!("unvollstaendige Release-Info"));
    }
    check_release_signature(&rel, PLATFORM, UPDATE_PUBKEY, REQUIRE_SIGNATURE)?;
    Ok(rel)
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// Ordner fuer die frische Datei.
///
/// Am liebsten direkt neben der laufenden Exe - dann ist der Tausch ein
/// simples Umbenennen. Liegt die Installation aber in "C:\Program Files",
/// darf ein normaler Nutzer dort nicht schreiben (os error 5). Dann weichen
/// wir in den Temp-Ordner aus; getauscht wird spaeter mit Rechten.
fn staging() -> PathBuf {
    if let Ok(cur) = std::env::current_exe() {
        if let Some(dir) = cur.parent() {
            let probe = dir.join(".fv-write-test");
            if std::fs::write(&probe, b"x").is_ok() {
                let _ = std::fs::remove_file(&probe);
                return dir.to_path_buf();
            }
        }
    }
    let d = std::env::temp_dir().join("freeviewer-update");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Ist das ein "darfst du nicht"-Fehler?
fn denied(e: &anyhow::Error) -> bool {
    let s = e.to_string();
    s.contains("os error 5") || s.contains("Zugriff verweigert") || s.contains("Access is denied")
}

/// Downloads the build and writes it into the staging folder.
pub fn download(rel: &Release) -> Result<PathBuf> {
    // Defence in depth: the release may come from stored state, not straight
    // from `check`. Refuse BEFORE downloading anything.
    check_release_signature(rel, PLATFORM, UPDATE_PUBKEY, REQUIRE_SIGNATURE)?;
    let bytes = fetch(&rel.url)?;
    // Bauchweh-Pruefung: ein Mac darf nie eine Windows-Exe bekommen (und
    // umgekehrt) - sonst frisst sich das Update selbst.
    #[cfg(target_os = "macos")]
    if bytes.starts_with(b"MZ") {
        return Err(anyhow!("Windows-Paket auf einem Mac - Update abgebrochen"));
    }
    #[cfg(windows)]
    if !bytes.starts_with(b"MZ") {
        return Err(anyhow!("kein Windows-Paket - Update abgebrochen"));
    }
    if rel.size != 0 && bytes.len() as u64 != rel.size {
        return Err(anyhow!(
            "Groesse passt nicht ({} statt {})",
            bytes.len(),
            rel.size
        ));
    }
    let got = sha256_hex(&bytes);
    if got != rel.sha256 {
        return Err(anyhow!("Pruefsumme falsch"));
    }
    let tmp = staging().join(format!("freeviewer-{}.exe", rel.version));
    let _ = std::fs::remove_file(&tmp);
    std::fs::write(&tmp, &bytes)?;
    Ok(tmp)
}

/// Puts the downloaded file in the place of the running one.
pub fn swap(fresh: &Path) -> Result<PathBuf> {
    let cur = std::env::current_exe()?;
    swap_into(fresh, &cur)?;
    Ok(cur)
}

/// Legt `fresh` an die Stelle von `target`. Windows kann eine laufende Exe
/// nicht ueberschreiben, aber umbenennen - danach ist der Platz frei.
/// Kopiert wird, nicht umbenannt: die frische Datei kann auf einem anderen
/// Laufwerk liegen.
pub fn swap_into(fresh: &Path, target: &Path) -> Result<()> {
    let old = target.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(target, &old).map_err(|e| anyhow!("Umbenennen fehlgeschlagen: {}", e))?;
    if let Err(e) = std::fs::copy(fresh, target) {
        // die laufende Installation wieder herstellen, sonst ist alles kaputt
        let _ = std::fs::rename(&old, target);
        return Err(anyhow!("Ersetzen fehlgeschlagen: {}", e));
    }
    let _ = std::fs::remove_file(fresh);
    Ok(())
}

/// Raeumt weg, was ein frueheres Update liegen gelassen hat (bei jedem Start).
///
/// Frueher wurden nur ".old" und ".new" geloescht. Konnte die alte Datei nicht
/// weg (weil sie noch lief), blieben Reste wie "freeviewer.old2" fuer immer im
/// Programmordner liegen - deshalb wird jetzt alles aufgeraeumt, was neben der
/// Exe mit demselben Namen und einer Update-Endung steht.
pub fn cleanup() {
    let Ok(cur) = std::env::current_exe() else {
        return;
    };
    let _ = std::fs::remove_file(cur.with_extension("old"));
    let _ = std::fs::remove_file(cur.with_extension("new"));
    let (Some(dir), Some(stem)) = (cur.parent(), cur.file_stem()) else {
        return;
    };
    let stem = stem.to_string_lossy().to_lowercase();
    let Ok(eintraege) = std::fs::read_dir(dir) else {
        return;
    };
    for e in eintraege.flatten() {
        let p = e.path();
        if p == cur {
            continue;
        }
        let gleicher_name = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase() == stem)
            .unwrap_or(false);
        let update_rest = p
            .extension()
            .map(|x| {
                let x = x.to_string_lossy().to_lowercase();
                x == "new" || x.starts_with("old")
            })
            .unwrap_or(false);
        if gleicher_name && update_rest {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Starts the given binary and ends this process.
pub fn restart_into(exe: &Path) -> ! {
    let mut cmd = std::process::Command::new(exe);
    for a in std::env::args().skip(1) {
        cmd.arg(a);
    }
    let _ = cmd.spawn();
    std::thread::sleep(Duration::from_millis(200));
    std::process::exit(0);
}

/// Download + verify + swap + restart.
///
/// Liegt FreeViewer in einem geschuetzten Ordner ("C:\Program Files"), macht
/// den Tausch ein kurzer Helfer mit Administrator-Rechten - gestartet aus der
/// FRISCHEN Datei, damit auch alte Staende diesen Weg nehmen koennen.
pub fn install(rel: &Release) -> Result<()> {
    let fresh = download(rel)?;
    let target = std::env::current_exe()?;
    match swap_into(&fresh, &target) {
        Ok(()) => restart_into(&target),
        Err(e) if denied(&e) => {
            elevated_swap(&fresh, &target)?;
            std::thread::sleep(Duration::from_millis(300));
            std::process::exit(0);
        }
        Err(e) => Err(e),
    }
}

/// Startet die frische Exe mit Administrator-Rechten, damit sie sich selbst
/// an die richtige Stelle legt.
#[cfg(windows)]
pub fn elevated_swap(fresh: &Path, target: &Path) -> Result<()> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let exe = wide(&fresh.to_string_lossy());
    let args = format!(
        "--apply-update \"{}\" \"{}\" {}",
        fresh.display(),
        target.display(),
        std::process::id()
    );
    let params = wide(&args);
    let r = unsafe {
        ShellExecuteW(
            HWND::default(),
            w!("runas"),
            PCWSTR(exe.as_ptr()),
            PCWSTR(params.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(anyhow!(
            "FreeViewer liegt in einem geschuetzten Ordner - fuer das Update werden Administrator-Rechte gebraucht"
        ))
    }
}

#[cfg(not(windows))]
pub fn elevated_swap(_fresh: &Path, _target: &Path) -> Result<()> {
    Err(anyhow!("nur unter Windows"))
}

/// `freeviewer --apply-update <frisch> <ziel> <pid>`
///
/// Laeuft mit Administrator-Rechten: tauscht die Datei, startet den Dienst neu
/// und holt das Fenster als normaler Nutzer zurueck (ueber den Explorer, damit
/// die App nicht dauerhaft erhoehte Rechte behaelt).
pub fn apply_update(fresh: &Path, target: &Path, pid: u32) -> Result<()> {
    // dem Aufrufer einen Moment geben, sich zu beenden
    std::thread::sleep(Duration::from_millis(600));
    let _ = pid;
    let mut last: Option<anyhow::Error> = None;
    for _ in 0..10 {
        match swap_into(fresh, target) {
            Ok(()) => {
                last = None;
                break;
            }
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(400));
            }
        }
    }
    if let Some(e) = last {
        return Err(e);
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("sc")
            .args(["stop", crate::service::SERVICE_NAME])
            .output();
        std::thread::sleep(Duration::from_millis(800));
        let _ = std::process::Command::new("sc")
            .args(["start", crate::service::SERVICE_NAME])
            .output();
        let _ = std::process::Command::new("explorer.exe")
            .arg(target.as_os_str())
            .spawn();
    }
    Ok(())
}

/// True while a session is running - never interrupt that for an update.
fn busy(shared: &Arc<Shared>) -> bool {
    shared.xfer.lock().unwrap().is_some()
        || shared.connected.load(Ordering::Relaxed)
        || shared.connecting.load(Ordering::Relaxed)
}

/// Hintergrund-Faden: sucht jetzt und alle paar Stunden nach einem neuen Stand.
///
/// `darf_einspielen` = false heisst: nur nachsehen und Bescheid sagen. Das ist
/// der Normalfall, sobald der Dienst installiert ist - der laeuft als SYSTEM,
/// darf nach "Programme" schreiben und braucht dafuer KEINE Nachfrage. Frueher
/// versuchten Oberflaeche UND Agent den Tausch selbst; beide landeten bei
/// "Zugriff verweigert", riefen die Administrator-Abfrage auf und probierten
/// es jede Minute erneut - daher die vielen Nachfragen.
pub fn watcher(shared: Arc<Shared>, darf_einspielen: bool) {
    cleanup();
    std::thread::spawn(move || loop {
        // Nach einem gescheiterten Versuch (z. B. Nachfrage abgelehnt) wird in
        // dieser Runde nicht noch einmal angesetzt.
        let mut gescheitert = false;
        match check() {
            Ok(rel) => {
                if newer(&rel.version, VERSION) {
                    *shared.update.lock().unwrap() = Some(rel.clone());
                    if darf_einspielen {
                        shared.set_update_status(format!(
                            "Update {} verfuegbar (dieser Stand: {})",
                            rel.version, VERSION
                        ));
                        if shared.auto_update.load(Ordering::Relaxed) && !busy(&shared) {
                            shared.set_update_status(format!(
                                "Installiere Update {} ...",
                                rel.version
                            ));
                            if let Err(e) = install(&rel) {
                                gescheitert = true;
                                shared.set_update_status(format!("Update fehlgeschlagen: {}", e));
                            }
                        }
                    } else {
                        shared.set_update_status(format!(
                            "Update {} wird vom Dienst eingespielt (dieser Stand: {})",
                            rel.version, VERSION
                        ));
                    }
                } else {
                    *shared.update.lock().unwrap() = None;
                    shared.set_update_status(format!("Aktuell (v{})", VERSION));
                }
            }
            Err(e) => shared.set_update_status(format!("Update-Pruefung: {}", e)),
        }
        // Spaeter neu nachsehen. Wartet ein Update darauf, dass eine Sitzung
        // endet, wird minuetlich geprueft - aber nur, wenn dieser Prozess
        // ueberhaupt einspielen darf und es nicht gerade gescheitert ist.
        for _ in 0..(EVERY.as_secs() / 60) {
            std::thread::sleep(Duration::from_secs(60));
            if !darf_einspielen || gescheitert {
                continue;
            }
            let pending = shared.update.lock().unwrap().clone();
            if let Some(rel) = pending {
                if shared.auto_update.load(Ordering::Relaxed) && !busy(&shared) {
                    shared.set_update_status(format!("Installiere Update {} ...", rel.version));
                    if let Err(e) = install(&rel) {
                        gescheitert = true;
                        shared.set_update_status(format!("Update fehlgeschlagen: {}", e));
                    }
                }
            }
        }
    });
}

/// Datei, mit der die Oberflaeche den Dienst bittet, sofort zu aktualisieren
/// (Knopfdruck statt Warten auf die halbe Stunde).
#[cfg(windows)]
fn trigger_pfad() -> Option<std::path::PathBuf> {
    Some(crate::ident::machine_config_dir()?.join("update-now.flag"))
}

/// Solange diese Datei steht, laeuft gerade ein Update - die Oberflaeche
/// zeigt dann das Lade-Fenster.
#[cfg(windows)]
pub fn updating_flag() -> Option<std::path::PathBuf> {
    Some(crate::ident::machine_config_dir()?.join("updating.flag"))
}

/// Marke, mit der eine SICHTBARE Oberflaeche sagt: "ich laufe gerade".
///
/// Der Dienst sitzt in Sitzung 0 und kann die Fenster des Benutzers nicht
/// sehen; die vorhandene Mutex-Pruefung ist auf "Local\\" begrenzt und damit
/// pro Sitzung. Eine Datei mit Zeitstempel ist der einzige Weg, der ueber
/// Sitzungsgrenzen funktioniert.
#[cfg(windows)]
pub fn gui_marke() -> Option<std::path::PathBuf> {
    Some(crate::ident::machine_config_dir()?.join("gui-laeuft.flag"))
}

/// Nach dem Update: diese Marke bittet den Agenten, die Oberflaeche wieder
/// zu oeffnen.
#[cfg(windows)]
pub fn gui_neustart_marke() -> Option<std::path::PathBuf> {
    Some(crate::ident::machine_config_dir()?.join("gui-neustart.flag"))
}

#[cfg(not(windows))]
pub fn gui_marke() -> Option<std::path::PathBuf> {
    None
}
#[cfg(not(windows))]
pub fn gui_neustart_marke() -> Option<std::path::PathBuf> {
    None
}

/// Laeuft in der sichtbaren Oberflaeche: haelt die Marke frisch.
pub fn gui_marke_pflegen() {
    #[cfg(windows)]
    std::thread::spawn(|| loop {
        if let Some(p) = gui_marke() {
            let _ = std::fs::write(&p, now_text().as_bytes());
        }
        std::thread::sleep(Duration::from_secs(15));
    });
}

fn now_text() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

/// War in den letzten 90 Sekunden eine sichtbare Oberflaeche offen?
pub fn gui_war_offen() -> bool {
    let Some(p) = gui_marke() else { return false };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return false;
    };
    let Ok(dann) = text.trim().parse::<u64>() else {
        return false;
    };
    let jetzt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    jetzt.saturating_sub(dann) <= 90
}

/// Laeuft im Agenten (Sitzung des Benutzers): stand vor dem Update ein
/// Fenster offen, wird es jetzt wieder geoeffnet.
///
/// WARUM hier und nicht im Dienst: der Dienst laeuft als SYSTEM in Sitzung 0.
/// Ein von dort gestarteter Prozess landet auf einem Desktop, den niemand
/// sieht. Der Agent sitzt schon im Benutzer-Desktop - er braucht dafuer nur
/// ein normales `spawn`.
pub fn gui_wieder_oeffnen() {
    #[cfg(windows)]
    {
        let Some(p) = gui_neustart_marke() else { return };
        if !p.exists() {
            return;
        }
        let _ = std::fs::remove_file(&p);
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        // Ohne Argumente = ganz normales Fenster, wie ein Doppelklick.
        let _ = std::process::Command::new(exe).spawn();
        crate::service::log("Oberflaeche nach dem Update wieder geoeffnet");
    }
}

/// Knopf "Update installieren" bei installiertem Dienst: nur die Bitte
/// hinterlegen, der Dienst macht den Rest (als SYSTEM, ohne Nachfrage).
#[cfg(windows)]
pub fn bitte_dienst() {
    if let Some(p) = trigger_pfad() {
        let _ = std::fs::write(p, b"1");
    }
}

#[cfg(not(windows))]
pub fn bitte_dienst() {}

#[cfg(not(windows))]
pub fn updating_flag() -> Option<std::path::PathBuf> {
    None
}

/// Der Updater des Dienstes. Laeuft als SYSTEM, darf also direkt nach
/// "Programme" schreiben - ohne jede Administrator-Nachfrage. Nach dem Tausch
/// werden ALLE FreeViewer-Prozesse beendet und der Dienst startet neu; der
/// Agent im Benutzer-Desktop kommt mit dem frischen Stand wieder.
#[cfg(windows)]
pub fn service_watcher() {
    cleanup();
    if let Some(p) = updating_flag() {
        let _ = std::fs::remove_file(p);
    }
    std::thread::spawn(move || {
        let mut naechste_runde = std::time::Instant::now();
        loop {
            // Schneller Wunsch von der Oberflaeche ("Update installieren")?
            let mut sofort = false;
            if let Some(p) = trigger_pfad() {
                if p.exists() {
                    let _ = std::fs::remove_file(&p);
                    sofort = true;
                }
            }
            if !sofort && std::time::Instant::now() < naechste_runde {
                std::thread::sleep(Duration::from_secs(10));
                continue;
            }
            naechste_runde = std::time::Instant::now() + EVERY;
            let einspielen = |rel: &Release| -> bool {
                // Der Knopf zaehlt als ausdruecklicher Wunsch - auch wenn die
                // Automatik aus ist.
                sofort || crate::ident::auto_update_enabled()
            };
            if let Ok(rel) = check() {
                if newer(&rel.version, VERSION) && einspielen(&rel) {
                    crate::service::log(&format!("Update {} gefunden", rel.version));
                    if let Some(p) = updating_flag() {
                        let _ = std::fs::write(&p, b"1");
                    }
                    let ergebnis = download(&rel).and_then(|fresh| {
                        let ziel = std::env::current_exe()?;
                        swap_into(&fresh, &ziel)
                    });
                    match ergebnis {
                        Ok(()) => {
                            crate::service::log("Update eingespielt - alle Teile starten neu");
                            // Stand ein Fenster offen? Dann soll es nach dem
                            // Neustart wiederkommen. Ohne diese Marke war der
                            // Bildschirm nach jedem Update einfach leer - der
                            // Dienst kam zurueck, das FENSTER nicht.
                            if gui_war_offen() {
                                if let Some(p) = gui_neustart_marke() {
                                    let _ = std::fs::write(&p, b"1");
                                }
                                crate::service::log("Oberflaeche war offen - wird neu geoeffnet");
                            }
                            // Neustart-Kette anwerfen. Dreimal versuchen: der
                            // Dienst braucht einen Moment, bis Windows ihn als
                            // GESTOPPT fuehrt; ein zu frueher Start scheitert
                            // mit "wird gerade beendet". Ein spaeterer Versuch
                            // auf einen schon laufenden Dienst schadet nicht.
                            let name = crate::service::SERVICE_NAME;
                            let _ = std::process::Command::new("cmd")
                                .args([
                                    "/c",
                                    &format!(
                                        "ping -n 4 127.0.0.1 >nul & sc start {n} >nul 2>&1 &                                          ping -n 6 127.0.0.1 >nul & sc start {n} >nul 2>&1 &                                          ping -n 11 127.0.0.1 >nul & sc start {n} >nul 2>&1",
                                        n = name
                                    ),
                                ])
                                .spawn();
                            // Oberflaeche und Agent beenden - aber NICHT uns
                            // selbst. Der Dienst geht gleich darunter geordnet
                            // zu Ende; ein Selbstabschuss zaehlt bei Windows
                            // als Absturz und verbraucht das Sicherheitsnetz.
                            let eigen = std::process::id();
                            let _ = std::process::Command::new("taskkill")
                                .args([
                                    "/f",
                                    "/im",
                                    crate::brand::EXE,
                                    "/fi",
                                    &format!("PID ne {}", eigen),
                                ])
                                .output();
                            // Jetzt sauber aussteigen: supervise() kehrt
                            // zurueck, der Dienst meldet ordentlich GESTOPPT.
                            crate::service::stop_anfordern();
                            return;
                        }
                        Err(e) => {
                            crate::service::log(&format!("Update fehlgeschlagen: {}", e));
                            if let Some(p) = updating_flag() {
                                let _ = std::fs::remove_file(p);
                            }
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(10));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(newer("0.6.0", "0.5.9"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.5.0", "0.5.0"));
        assert!(!newer("0.4.9", "0.5.0"));
        assert!(newer("0.5.1", "0.5"));
        assert!(!newer("kaputt", "0.5.0"));
    }

    #[test]
    fn hash_is_the_usual_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    // --- signature verification ---

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    // Test-only key: seed = 32 x 0x07. Vectors made by tools/sign-release.
    const TEST_PUB: &str = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c";
    const TEST_SIG_WIN: &str = "abbaaec85acdd2df47bb2882565f2e94594eedc921d3862f3f77be0b2973768060b04b5c5f0a4123714f178d74e102157d54db55874bbf54efe3dd4e58d13b0c";
    const TEST_SIG_MAC: &str = "fcbd04a4a82685c848f8cf5543a213bb8a1847344a49f7ab23b51dcabfc19d52e2c71a34543c3575adb72733987e8648f1c13b5a76865f08b519e7410e80f20a";

    fn signed_rel() -> Release {
        Release {
            version: "1.2.3".into(),
            sha256: ABC.into(),
            url: "https://example.invalid/x.exe".into(),
            notes: String::new(),
            size: 3,
            sig: TEST_SIG_WIN.into(),
        }
    }

    #[test]
    fn python_signature_verifies() {
        // cross-language check: signed by tools/sign-release (Python)
        let r = signed_rel();
        check_release_signature(&r, "win", TEST_PUB, true).unwrap();
        let mac = Release {
            sig: TEST_SIG_MAC.into(),
            ..r
        };
        check_release_signature(&mac, "mac", TEST_PUB, true).unwrap();
    }

    #[test]
    fn rust_signed_roundtrip() {
        use ed25519_dalek::{Signer, SigningKey};
        let sk = SigningKey::from_bytes(&[9u8; 32]);
        let pk = hex::encode(sk.verifying_key().to_bytes());
        let msg = signed_message("win", "0.30.0", ABC, 3);
        let r = Release {
            sig: hex::encode(sk.sign(&msg).to_bytes()),
            ..signed_rel()
        };
        let r = Release {
            version: "0.30.0".into(),
            ..r
        };
        check_release_signature(&r, "win", &pk, true).unwrap();
        // the production key must not accept it
        assert!(check_release_signature(&r, "win", UPDATE_PUBKEY, true).is_err());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let r = signed_rel();
        let other = "6fd3beb25185415730f99e3222f6ded7834e82339d6ba2902ecc8308469e97cd";
        assert!(check_release_signature(&r, "win", other, false).is_err());
    }

    #[test]
    fn tampered_fields_are_rejected() {
        let base = signed_rel();
        let mut v = base.clone();
        v.version = "1.2.4".into();
        let mut h = base.clone();
        h.sha256 = sha256_hex(b"evil");
        let mut z = base.clone();
        z.size = 4;
        for r in [v, h, z] {
            // even in the lenient phase a bad signature is never accepted
            assert!(check_release_signature(&r, "win", TEST_PUB, false).is_err());
            assert!(check_release_signature(&r, "win", TEST_PUB, true).is_err());
        }
        // upper-case hash in the feed is the same hash
        let mut up = base.clone();
        up.sha256 = ABC.to_uppercase();
        check_release_signature(&up, "win", TEST_PUB, true).unwrap();
    }

    #[test]
    fn platform_swap_is_rejected() {
        // the Windows signature must not unlock the Mac package
        assert!(check_release_signature(&signed_rel(), "mac", TEST_PUB, false).is_err());
    }

    #[test]
    fn flipped_signature_bit_is_rejected() {
        let mut r = signed_rel();
        let mut raw = hex::decode(&r.sig).unwrap();
        raw[10] ^= 1;
        r.sig = hex::encode(raw);
        assert!(check_release_signature(&r, "win", TEST_PUB, false).is_err());
    }

    #[test]
    fn malformed_inputs_are_errors_not_panics() {
        for bad in ["zz", "abcd", &"0".repeat(128), &"0".repeat(130)] {
            let r = Release {
                sig: bad.into(),
                ..signed_rel()
            };
            assert!(check_release_signature(&r, "win", TEST_PUB, false).is_err());
        }
        assert!(verify_signature("not-hex", b"x", TEST_SIG_WIN).is_err());
        assert!(verify_signature("abcd", b"x", TEST_SIG_WIN).is_err());
    }

    #[test]
    fn unsigned_feed_follows_migration_switch() {
        let r = Release {
            sig: String::new(),
            ..signed_rel()
        };
        // phase 1: legacy feed still works
        check_release_signature(&r, "win", TEST_PUB, false).unwrap();
        // phase 2: refused
        assert!(check_release_signature(&r, "win", TEST_PUB, true).is_err());
    }

    #[test]
    fn built_in_key_is_a_valid_ed25519_key() {
        let raw: [u8; 32] = hex::decode(UPDATE_PUBKEY).unwrap().try_into().unwrap();
        VerifyingKey::from_bytes(&raw).unwrap();
    }

    #[test]
    fn parse_feed_handles_signed_and_unsigned() {
        // unsigned (old relay): accepted while REQUIRE_SIGNATURE is false
        let unsigned = format!(
            r#"{{"version":"9.9.9","url":"https://x.invalid/a.exe","sha256":"{ABC}","size":3}}"#
        );
        let r = parse_feed(unsigned.as_bytes());
        assert_eq!(r.is_ok(), !REQUIRE_SIGNATURE);
        // signed with a key that is not the built-in one: always refused
        let bad = format!(
            r#"{{"version":"1.2.3","url":"https://x.invalid/a.exe","sha256":"{ABC}","size":3,"sig":"{TEST_SIG_WIN}","mac_url":"https://x.invalid/m","mac_sha256":"{ABC}","mac_size":3,"mac_sig":"{TEST_SIG_MAC}"}}"#
        );
        assert!(parse_feed(bad.as_bytes()).is_err());
        // no package for this platform: not an error, nothing offered
        let none = parse_feed(br#"{"version":"9.9.9"}"#).unwrap();
        assert!(none.url.is_empty());
        // garbage
        assert!(parse_feed(b"not json").is_err());
    }
}
