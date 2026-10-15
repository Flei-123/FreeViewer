//! Brand in ONE place. Whoever wants a second product (e.g. "X-Remote"
//! for another company) builds with environment variables set:
//!
//! ```sh
//! FV_BRAND_NAME="X-Remote" FV_BRAND_EXE=x-remote.exe \
//! FV_BRAND_DIR="X-Remote" cargo build --release
//! ```
//!
//! Without these variables everything stays FreeViewer. Protocol, relay and IDs
//! are the same for all brands - every build can talk to every other.
//! (Later the GitHub action builds this as a matrix with two exe files.)

/// Display name everywhere: window, tray, start menu, service.
pub const NAME: &str = match option_env!("FV_BRAND_NAME") {
    Some(s) => s,
    None => "FreeViewer",
};

/// Dateiname des Programms.
pub const EXE: &str = match option_env!("FV_BRAND_EXE") {
    Some(s) => s,
    None => "freeviewer.exe",
};

/// Folder name in "Program Files", ProgramData and AppData.
pub const DIR: &str = match option_env!("FV_BRAND_DIR") {
    Some(s) => s,
    None => "FreeViewer",
};

/// Herausgeber (steht in Apps & Features).
pub const PUBLISHER: &str = match option_env!("FV_BRAND_PUBLISHER") {
    Some(s) => s,
    None => "FleiTec",
};

/// Short name of the brand - this way the relay knows which file a
/// setup link should load ("freeviewer" or "xoffi").
pub const SLUG: &str = match option_env!("FV_BRAND_SLUG") {
    Some(s) => s,
    None => "freeviewer",
};

/// Public address of the brand - the setup links point to it.
pub const WEB: &str = match option_env!("FV_BRAND_WEB") {
    Some(s) => s,
    None => "https://freeviewer.fleitec.com",
};

/// Own update feed per brand - a Xoffi build must never "update"
/// to FreeViewer.
pub const FEED: &str = match option_env!("FV_BRAND_FEED") {
    Some(s) => s,
    None => "https://freeviewer.fleitec.com/fv/version",
};

/// Registry branch of the brand (identity backup, uninstall entry
/// is stored below it).
pub fn reg_key() -> String {
    format!(r"SOFTWARE\{}", DIR)
}
