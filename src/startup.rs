//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use super::*;

/// The host already runs in its own threads - here we only wait and
/// log so that the process stays alive.
pub(crate) fn ohne_fenster_weiterlaufen(shared: &Arc<Shared>) -> ! {
    capture::log_line("Oberflaeche geht nicht - laufe als Host ohne Fenster weiter");
    let mut zuletzt = String::new();
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let id = shared.my_id.lock().unwrap().clone();
        let status = shared.host_status.lock().unwrap().clone();
        let zeile = format!("ohne Fenster: ID {} | {}", id, status);
        if zeile != zuletzt {
            capture::log_line(&zeile);
            println!("{}", zeile);
            zuletzt = zeile;
        }
    }
}

/// Has at least one frame been drawn yet? The watchdog depends on this:
/// without this sign of life the UI has not started.
pub(crate) static BILD_GEZEICHNET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Are all attempts (OpenGL, OpenGL without acceleration, DirectX) through?
/// Then the main path reports the error - the watchdog stays out.
pub(crate) static ANLAEUFE_FERTIG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Why the start went wrong - readable for humans.
///
/// The most common case is a machine without usable graphics output:
/// Windows Server with "Microsoft Basic Display Adapter", an RDP session
/// without graphics redirection, or a machine without a graphics driver. There
/// the OpenGL context the UI needs fails.
pub(crate) fn start_fehler_text(e: &str) -> String {
    let low = e.to_lowercase();
    let grund = if low.contains("opengl")
        || low.contains("glutin")
        || low.contains("wgl")
        || low.contains("egl")
        || low.contains("no available")
        || low.contains("context")
        || low.contains("pixel format")
        || low.contains("surface")
    {
        "Dieser Rechner stellt keine brauchbare Grafikausgabe bereit.\n\n         Das passiert auf Servern ohne Grafiktreiber (Microsoft Basic Display          Adapter) und in Remotedesktop-Sitzungen ohne Grafikweiterleitung.\n\n         Was hilft:\n         \u{2022} Grafiktreiber des Servers installieren\n         \u{2022} direkt an der Konsole des Servers anmelden statt ueber          Remotedesktop\n         \u{2022} in einer virtuellen Maschine die Grafikbeschleunigung          einschalten"
    } else {
        "Der Start wurde vom Betriebssystem abgewiesen."
    };
    format!(
        "{} konnte kein Fenster oeffnen.\n\n{}\n\n\
         DER RECHNER BLEIBT TROTZDEM ERREICHBAR: {} laeuft im Hintergrund \
         weiter, man kann sich also von aussen daraufschalten.\n\n\
         Damit das auch nach einem Neustart und vor der Anmeldung gilt, als \
         Administrator einmal aufrufen:\n    {} --install-service\n\n\
         Technische Meldung:\n{}\n\nAlle Einzelheiten stehen in:\n{}",
        crate::brand::NAME,
        grund,
        crate::brand::NAME,
        crate::brand::EXE,
        e,
        ident::config_dir().join("start.log").display()
    )
}

/// Writes the error to the log and shows it as a window.
pub(crate) fn start_fehler_melden(e: &str) {
    let text = start_fehler_text(e);
    let pfad = ident::config_dir().join("start.log");
    let _ = std::fs::create_dir_all(ident::config_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&pfad)
    {
        use std::io::Write;
        let _ = writeln!(
            f,
            "[{}] Start fehlgeschlagen ({} {}): {}",
            chrono_zeit(),
            crate::brand::NAME,
            update::VERSION,
            e
        );
    }
    eprintln!("{}", text);
    fehlerfenster(&text);
}

/// Date and time without an extra crate - enough for a log line.
pub(crate) fn chrono_zeit() -> String {
    let jetzt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix {}", jetzt)
}

#[cfg(windows)]
pub(crate) fn fehlerfenster(text: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };
    let mut t: Vec<u16> = text.encode_utf16().collect();
    t.push(0);
    let mut titel: Vec<u16> = format!("{} - Start fehlgeschlagen", crate::brand::NAME)
        .encode_utf16()
        .collect();
    titel.push(0);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(t.as_ptr()),
            PCWSTR(titel.as_ptr()),
            MB_OK | MB_ICONERROR | MB_TOPMOST | MB_SETFOREGROUND,
        );
    }
}

#[cfg(not(windows))]
pub(crate) fn fehlerfenster(_text: &str) {}
