//! Die Titelleiste von Windows in der Farbe des Programms.
//!
//! Windows 11 lässt die Leiste über DwmSetWindowAttribute einfärben
//! (DWMWA_CAPTION_COLOR / _TEXT_COLOR / _BORDER_COLOR). Auf Windows 10 gibt
//! es das nicht - dort schlägt der Aufruf still fehl und die Leiste bleibt
//! wie sie ist, was in Ordnung ist.

#[cfg(windows)]
fn colorref(c: egui::Color32) -> u32 {
    // COLORREF ist 0x00bbggrr
    (c.b() as u32) << 16 | (c.g() as u32) << 8 | c.r() as u32
}

/// Setzt Leiste, Schrift und Rand auf die Farben der Palette.
#[cfg(windows)]
pub fn paint_caption(caption: egui::Color32, text: egui::Color32, border: egui::Color32, dark: bool) {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
        DWMWA_USE_IMMERSIVE_DARK_MODE,
    };
    unsafe {
        let hwnd = match crate::tray::main_window() {
            Some(h) => h,
            None => return,
        };
        let d = BOOL::from(dark);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &d as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<BOOL>() as u32,
        );
        for (attr, col) in [
            (DWMWA_CAPTION_COLOR, colorref(caption)),
            (DWMWA_TEXT_COLOR, colorref(text)),
            (DWMWA_BORDER_COLOR, colorref(border)),
        ] {
            let _ = DwmSetWindowAttribute(
                hwnd,
                attr,
                &col as *const _ as *const std::ffi::c_void,
                4,
            );
        }
    }
}

#[cfg(not(windows))]
pub fn paint_caption(
    _caption: egui::Color32,
    _text: egui::Color32,
    _border: egui::Color32,
    _dark: bool,
) {
}

/// Farben aus der aktiven Palette nehmen.
pub fn paint_from_theme() {
    let p = crate::theme::palette();
    paint_caption(p.card, p.text, p.line, p.dark);
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn colorref_swaps_red_and_blue() {
        // reines Rot (255,0,0) muss als 0x0000ff ankommen
        assert_eq!(super::colorref(egui::Color32::from_rgb(255, 0, 0)), 0x0000ff);
        assert_eq!(super::colorref(egui::Color32::from_rgb(0, 0, 255)), 0xff0000);
        assert_eq!(
            super::colorref(egui::Color32::from_rgb(0x1b, 0x1e, 0x26)),
            0x261e1b
        );
    }
}
