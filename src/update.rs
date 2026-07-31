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
use sha2::{Digest, Sha256};

use crate::shared::Shared;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const FEED: &str = "https://jarvis.fleitec.com/fv/version";
/// How often a running instance looks for a new build.
const EVERY: Duration = Duration::from_secs(30 * 60);
const MAX_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Release {
    pub version: String,
    pub sha256: String,
    pub url: String,
    pub notes: String,
    pub size: u64,
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
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    let s = |k: &str| -> String {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let rel = Release {
        version: s("version"),
        sha256: s("sha256").to_lowercase(),
        url: s("url"),
        notes: s("notes"),
        size: v.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
    };
    if rel.version.is_empty() || rel.url.is_empty() || rel.sha256.len() != 64 {
        return Err(anyhow!("unvollstaendige Release-Info"));
    }
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
    let bytes = fetch(&rel.url)?;
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

/// Removes the leftover of a previous update (called at every start).
pub fn cleanup() {
    if let Ok(cur) = std::env::current_exe() {
        let _ = std::fs::remove_file(cur.with_extension("old"));
        let _ = std::fs::remove_file(cur.with_extension("new"));
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

/// Background thread: looks for a new build now and every few hours.
pub fn watcher(shared: Arc<Shared>) {
    cleanup();
    std::thread::spawn(move || loop {
        match check() {
            Ok(rel) => {
                if newer(&rel.version, VERSION) {
                    *shared.update.lock().unwrap() = Some(rel.clone());
                    shared.set_update_status(format!(
                        "Update {} verfuegbar (dieser Stand: {})",
                        rel.version, VERSION
                    ));
                    if shared.auto_update.load(Ordering::Relaxed) && !busy(&shared) {
                        shared.set_update_status(format!("Installiere Update {} ...", rel.version));
                        match install(&rel) {
                            Ok(()) => {}
                            Err(e) => shared.set_update_status(format!("Update fehlgeschlagen: {}", e)),
                        }
                    }
                } else {
                    *shared.update.lock().unwrap() = None;
                    shared.set_update_status(format!("Aktuell (v{})", VERSION));
                }
            }
            Err(e) => shared.set_update_status(format!("Update-Pruefung: {}", e)),
        }
        // check again later, but look every minute whether a pending update can
        // finally be installed (session ended in the meantime)
        for _ in 0..(EVERY.as_secs() / 60) {
            std::thread::sleep(Duration::from_secs(60));
            let pending = shared.update.lock().unwrap().clone();
            if let Some(rel) = pending {
                if shared.auto_update.load(Ordering::Relaxed) && !busy(&shared) {
                    shared.set_update_status(format!("Installiere Update {} ...", rel.version));
                    if let Err(e) = install(&rel) {
                        shared.set_update_status(format!("Update fehlgeschlagen: {}", e));
                    }
                }
            }
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
}
