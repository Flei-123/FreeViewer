//! Marke an EINEM Ort. Wer ein zweites Produkt will (z. B. "Xoffi Remote"
//! fuer eine andere Firma), baut mit gesetzten Umgebungsvariablen:
//!
//! ```sh
//! FV_BRAND_NAME="Xoffi Remote" FV_BRAND_EXE=xoffi-remote.exe \
//! FV_BRAND_DIR="Xoffi Remote" cargo build --release
//! ```
//!
//! Ohne diese Variablen bleibt alles FreeViewer. Protokoll, Relay und IDs
//! sind bei allen Marken gleich - jeder Build kann mit jedem reden.
//! (Spaeter baut das die GitHub-Action als Matrix mit zwei Exe-Dateien.)

/// Anzeigename ueberall: Fenster, Tray, Startmenue, Dienst.
pub const NAME: &str = match option_env!("FV_BRAND_NAME") {
    Some(s) => s,
    None => "FreeViewer",
};

/// Dateiname des Programms.
pub const EXE: &str = match option_env!("FV_BRAND_EXE") {
    Some(s) => s,
    None => "freeviewer.exe",
};

/// Ordnername in "Programme", ProgramData und AppData.
pub const DIR: &str = match option_env!("FV_BRAND_DIR") {
    Some(s) => s,
    None => "FreeViewer",
};

/// Herausgeber (steht in Apps & Features).
pub const PUBLISHER: &str = match option_env!("FV_BRAND_PUBLISHER") {
    Some(s) => s,
    None => "FleiTec",
};

/// Kurzname der Marke - das Relay weiss so, welche Datei ein
/// Einrichtungs-Link laden soll ("freeviewer" oder "xoffi").
pub const SLUG: &str = match option_env!("FV_BRAND_SLUG") {
    Some(s) => s,
    None => "freeviewer",
};

/// Oeffentliche Adresse der Marke - darauf zeigen die Einrichtungs-Links.
pub const WEB: &str = match option_env!("FV_BRAND_WEB") {
    Some(s) => s,
    None => "https://freeviewer.fleitec.com",
};

/// Eigener Update-Feed pro Marke - ein Xoffi-Build darf sich nie zum
/// FreeViewer "aktualisieren".
pub const FEED: &str = match option_env!("FV_BRAND_FEED") {
    Some(s) => s,
    None => "https://freeviewer.fleitec.com/fv/version",
};

/// Registry-Zweig der Marke (Identitaets-Sicherung, Deinstallations-Eintrag
/// wird darunter abgelegt).
pub fn reg_key() -> String {
    format!(r"SOFTWARE\{}", DIR)
}
