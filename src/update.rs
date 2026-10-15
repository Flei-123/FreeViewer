//! Self update: ask the relay for the newest published build, verify it and
//! replace the running binary.
//!
//! Windows cannot overwrite a running .exe, but it can *rename* it. So the
//! sequence is: download -> check SHA-256 -> rename running exe to ".old" ->
//! move the fresh one into its place -> start it -> exit. The next start
//! deletes the ".old" leftover.

use crate::lockutil::LockExt;
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

/// Version of the package for `platform`. The Mac package has its own
/// `mac_version` (r28): a Windows-only publish moves `version` but must neither
/// offer an old Mac zip as "new" nor invalidate `mac_sig`, which signs
/// `mac_version`. Feeds without `mac_version` (legacy) fall back to `version`.
pub fn feed_version(v: &serde_json::Value, platform: &str) -> String {
    let get = |k: &str| -> String {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    if platform == "mac" {
        let mv = get("mac_version");
        if !mv.is_empty() {
            return mv;
        }
    }
    get("version")
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
    // Each platform has its own package: "url" is Windows, "mac_url"
    // is the Mac. Never pull the wrong file again - a Windows exe would
    // otherwise destroy a Mac during the swap.
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
        version: feed_version(&v, PLATFORM),
        sha256: hash,
        url,
        notes: s("notes"),
        size,
        sig,
    };
    // If the package for the own platform is missing: no update, no error -
    // just offer nothing.
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

/// Folder for the fresh file.
///
/// Preferably right next to the running exe - then the swap is a
/// simple rename. But if the installation is in "C:\Program Files",
/// a normal user may not write there (os error 5). Then we fall back
/// to the temp folder; the swap happens later with elevated rights.
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

/// Is this a "you are not allowed" error?
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
    // Sanity check: a Mac must never get a Windows exe (and
    // vice versa) - otherwise the update eats itself.
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

/// Puts `fresh` in the place of `target`. Windows cannot overwrite a running
/// exe, but it can rename it - after that the spot is free.
/// It copies, not renames: the fresh file may be on a different
/// drive.
pub fn swap_into(fresh: &Path, target: &Path) -> Result<()> {
    let old = target.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(target, &old).map_err(|e| anyhow!("Umbenennen fehlgeschlagen: {}", e))?;
    if let Err(e) = std::fs::copy(fresh, target) {
        // restore the running installation, otherwise everything is broken
        let _ = std::fs::rename(&old, target);
        return Err(anyhow!("Ersetzen fehlgeschlagen: {}", e));
    }
    let _ = std::fs::remove_file(fresh);
    Ok(())
}

/// Cleans up whatever an earlier update left behind (at every start).
///
/// Previously only ".old" and ".new" were deleted. If the old file could not
/// be removed (because it was still running), leftovers like "freeviewer.old2" stayed forever in the
/// program folder - so now everything next to the
/// exe with the same name and an update extension is cleaned up.
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
/// If FreeViewer is in a protected folder ("C:\Program Files"), a short helper
/// with administrator rights does the swap - started from the
/// FRESH file, so that old versions can take this path too.
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

/// Starts the fresh exe with administrator rights so that it puts itself
/// in the right place.
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
    // SAFETY: `exe` and `params` are NUL-terminated UTF-16 buffers alive during the call; the verb is a static string.
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

/// `freeviewer --apply-update <fresh> <target> <pid>`
///
/// Runs with administrator rights: swaps the file, restarts the service
/// and brings the window back as a normal user (via Explorer, so that
/// the app does not permanently keep elevated rights).
pub fn apply_update(fresh: &Path, target: &Path, pid: u32) -> Result<()> {
    // give the caller a moment to exit
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
    shared.xfer.lock_ok().is_some()
        || shared.connected.load(Ordering::Relaxed)
        || shared.connecting.load(Ordering::Relaxed)
}

/// Background thread: checks now and every few hours for a new version.
///
/// `darf_einspielen` = false means: only look and report. That is
/// the normal case once the service is installed - it runs as SYSTEM,
/// may write to "Program Files" and needs NO prompt for it. Previously
/// the UI AND the agent both tried the swap themselves; both ended up at
/// "access denied", raised the administrator prompt and tried
/// again every minute - hence the many prompts.
pub fn watcher(shared: Arc<Shared>, darf_einspielen: bool) {
    cleanup();
    std::thread::spawn(move || loop {
        // After a failed attempt (e.g. prompt declined) it is not
        // tried again in this round.
        let mut gescheitert = false;
        match check() {
            Ok(rel) => {
                if newer(&rel.version, VERSION) {
                    *shared.update.lock_ok() = Some(rel.clone());
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
                    *shared.update.lock_ok() = None;
                    shared.set_update_status(format!("Aktuell (v{})", VERSION));
                }
            }
            Err(e) => shared.set_update_status(format!("Update-Pruefung: {}", e)),
        }
        // Check again later. If an update is waiting for a session
        // to end, it is checked every minute - but only if this process
        // is allowed to apply it at all and it has not just failed.
        for _ in 0..(EVERY.as_secs() / 60) {
            std::thread::sleep(Duration::from_secs(60));
            if !darf_einspielen || gescheitert {
                continue;
            }
            let pending = shared.update.lock_ok().clone();
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

/// File with which the UI asks the service to update immediately
/// (button press instead of waiting for the half hour).
#[cfg(windows)]
fn trigger_pfad() -> Option<std::path::PathBuf> {
    Some(crate::ident::machine_config_dir()?.join("update-now.flag"))
}

/// As long as this file exists, an update is running - the UI
/// then shows the loading window.
#[cfg(windows)]
pub fn updating_flag() -> Option<std::path::PathBuf> {
    Some(crate::ident::machine_config_dir()?.join("updating.flag"))
}

/// Marker with which a VISIBLE UI says: "I am running".
///
/// The service sits in session 0 and cannot see the user's windows;
/// the existing mutex check is limited to "Local\\" and thus
/// per session. A file with a timestamp is the only way that works
/// across session boundaries.
#[cfg(windows)]
pub fn gui_marke() -> Option<std::path::PathBuf> {
    Some(crate::ident::machine_config_dir()?.join("gui-laeuft.flag"))
}

/// After the update: this marker asks the agent to open the UI
/// again.
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

/// Runs in the visible UI: keeps the marker fresh.
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

/// Was a visible UI open in the last 90 seconds?
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

/// Runs in the agent (the user's session): if a window was open
/// before the update, it is opened again now.
///
/// WHY here and not in the service: the service runs as SYSTEM in session 0.
/// A process started from there ends up on a desktop nobody
/// sees. The agent already sits on the user desktop - it only needs
/// a normal `spawn` for this.
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
        // Without arguments = completely normal window, like a double click.
        let _ = std::process::Command::new(exe).spawn();
        crate::service::log("Oberflaeche nach dem Update wieder geoeffnet");
    }
}

/// "Install update" button with an installed service: only leave the request,
/// the service does the rest (as SYSTEM, without prompt).
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

/// The updater of the service. Runs as SYSTEM, so it may write directly to
/// "Program Files" - without any administrator prompt. After the swap
/// ALL FreeViewer processes are ended and the service restarts; the
/// agent on the user desktop comes back with the fresh version.
#[cfg(windows)]
pub fn service_watcher() {
    cleanup();
    if let Some(p) = updating_flag() {
        let _ = std::fs::remove_file(p);
    }
    std::thread::spawn(move || {
        let mut naechste_runde = std::time::Instant::now();
        loop {
            // Quick request from the UI ("Install update")?
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
                // The button counts as an explicit request - even if
                // automatic mode is off.
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
                            // Was a window open? Then it should come back after the
                            // restart. Without this marker the screen was
                            // simply empty after every update - the service
                            // came back, the WINDOW did not.
                            if gui_war_offen() {
                                if let Some(p) = gui_neustart_marke() {
                                    let _ = std::fs::write(&p, b"1");
                                }
                                crate::service::log("Oberflaeche war offen - wird neu geoeffnet");
                            }
                            // Start the restart chain. Try three times: the
                            // service needs a moment until Windows lists it as
                            // STOPPED; a start that is too early fails
                            // with "is being stopped". A later attempt
                            // on an already running service does no harm.
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
                            // End UI and agent - but NOT ourselves.
                            // The service ends in an orderly way right below;
                            // a self-kill counts as a crash on Windows
                            // and uses up the safety net.
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
                            // Now exit cleanly: supervise() returns,
                            // the service reports STOPPED properly.
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
    fn feed_version_uses_mac_version_only_for_mac() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"version":"0.29.4","mac_version":"0.26.10"}"#,
        )
        .unwrap();
        assert_eq!(feed_version(&v, "win"), "0.29.4");
        assert_eq!(feed_version(&v, "mac"), "0.26.10");
        // legacy feed without mac_version: both platforms use "version"
        let legacy: serde_json::Value = serde_json::from_str(r#"{"version":"0.29.4"}"#).unwrap();
        assert_eq!(feed_version(&legacy, "mac"), "0.29.4");
        let empty: serde_json::Value =
            serde_json::from_str(r#"{"version":"0.29.4","mac_version":""}"#).unwrap();
        assert_eq!(feed_version(&empty, "mac"), "0.29.4");
    }

    #[test]
    fn mac_signature_binds_mac_version_not_windows_version() {
        // A Windows publish changes "version" only: the Mac signature over
        // mac_version must stay valid, one over the Windows version must not.
        let m_old = signed_message("mac", "0.26.10", ABC, 3);
        let m_win = signed_message("mac", "0.29.4", ABC, 3);
        assert_ne!(m_old, m_win);
        let v: serde_json::Value = serde_json::from_str(
            r#"{"version":"0.29.5","mac_version":"0.26.10"}"#,
        )
        .unwrap();
        let signed_for = signed_message("mac", &feed_version(&v, "mac"), ABC, 3);
        assert_eq!(signed_for, m_old);
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
