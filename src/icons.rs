//! The icon set. Every symbol is an inline SVG (24x24, stroke based, white),
//! rasterized by egui_extras/resvg and tinted to the colour we need - the same
//! set of shapes FreeMeet uses on the web, so both programs look related.
//!
//! Add a symbol: one entry in ICONS, then `icons::show(ui, "name", 18.0, col)`.

/// name -> SVG source
pub const ICONS: &[(&str, &str)] = &[
    (
        "home",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 10.5 12 3l9 7.5"/><path d="M5.5 9.5V20h13V9.5"/><path d="M9.5 20v-6h5v6"/></svg>"##,
    ),
    (
        "devices",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="4" width="13" height="9.5" rx="1.6"/><path d="M6 17h6"/><rect x="17" y="9" width="4.5" height="11" rx="1.4"/></svg>"##,
    ),
    (
        "monitor",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="12" rx="1.8"/><path d="M9 20h6M12 16v4"/></svg>"##,
    ),
    (
        "laptop",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="5" width="16" height="10" rx="1.6"/><path d="M2 18.5h20"/></svg>"##,
    ),
    (
        "chat",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M20 12.5c0 3.6-3.6 6.5-8 6.5-1 0-2-.15-2.9-.42L5 20.5l1.2-3.1C4.85 16.2 4 14.45 4 12.5 4 8.9 7.6 6 12 6s8 2.9 8 6.5z"/></svg>"##,
    ),
    (
        "settings",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.03 1.56V21a2 2 0 1 1-4 0v-.09A1.7 1.7 0 0 0 8.9 19.3a1.7 1.7 0 0 0-1.87.35l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.7 1.7 0 0 0 4.7 15a1.7 1.7 0 0 0-1.56-1.03H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.7 8.9a1.7 1.7 0 0 0-.35-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.7 1.7 0 0 0 9 4.7a1.7 1.7 0 0 0 1.03-1.56V3a2 2 0 1 1 4 0v.09A1.7 1.7 0 0 0 15 4.7a1.7 1.7 0 0 0 1.87-.35l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.7 1.7 0 0 0 19.3 9v.09c.66.28 1.09.92 1.09 1.65"/></svg>"##,
    ),
    (
        "palette",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3a9 9 0 1 0 0 18c1.2 0 1.8-.9 1.8-1.9 0-1.6 1.1-2.1 2.4-2.1H18a3 3 0 0 0 3-3 9 9 0 0 0-9-9z"/><circle cx="8" cy="10" r="1.1"/><circle cx="12" cy="7.5" r="1.1"/><circle cx="15.8" cy="10" r="1.1"/></svg>"##,
    ),
    (
        "search",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/></svg>"##,
    ),
    (
        "mic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="2.5" width="6" height="11" rx="3"/><path d="M5.5 11.5a6.5 6.5 0 0 0 13 0"/><path d="M12 18v3.5M8.5 21.5h7"/></svg>"##,
    ),
    (
        "mic-off",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 5.5V5a3 3 0 0 0-6 0v5.5"/><path d="M5.5 11.5a6.5 6.5 0 0 0 10.2 5.3"/><path d="M12 18v3.5M8.5 21.5h7"/><path d="M3 3l18 18"/></svg>"##,
    ),
    (
        "sound",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9.5h3.5L12 5.5v13L7.5 14.5H4z"/><path d="M16 9.5a4 4 0 0 1 0 5"/><path d="M18.5 7a7 7 0 0 1 0 10"/></svg>"##,
    ),
    (
        "sound-off",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9.5h3.5L12 5.5v13L7.5 14.5H4z"/><path d="M16 10l5 4M21 10l-5 4"/></svg>"##,
    ),
    (
        "files",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 6.5A1.5 1.5 0 0 1 5 5h4l2 2.5h6.5A1.5 1.5 0 0 1 19 9v8.5A1.5 1.5 0 0 1 17.5 19H5a1.5 1.5 0 0 1-1.5-1.5z"/></svg>"##,
    ),
    (
        "keyboard",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="6" width="19" height="12" rx="2"/><path d="M6 10h.01M9.5 10h.01M13 10h.01M16.5 10h.01M7.5 14h9"/></svg>"##,
    ),
    (
        "gamepad",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M7.5 8h9a4.5 4.5 0 0 1 4.3 5.8l-.7 2.4A2.6 2.6 0 0 1 16 17.4L14.5 15.5h-5L8 17.4a2.6 2.6 0 0 1-4.1-1.2l-.7-2.4A4.5 4.5 0 0 1 7.5 8z"/><path d="M7 11.7v2M6 12.7h2M16.5 12h.01M15 13.5h.01"/></svg>"##,
    ),
    (
        "trash",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6.5h16"/><path d="M9.5 6.5V4.8A1.3 1.3 0 0 1 10.8 3.5h2.4a1.3 1.3 0 0 1 1.3 1.3v1.7"/><path d="M6.5 6.5 7.4 19a1.6 1.6 0 0 0 1.6 1.5h6a1.6 1.6 0 0 0 1.6-1.5l.9-12.5"/><path d="M10.5 10v6.5M13.5 10v6.5"/></svg>"##,
    ),
    (
        "plus",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14M5 12h14"/></svg>"##,
    ),
    (
        "dots",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff" stroke="none"><circle cx="12" cy="5.5" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="12" cy="18.5" r="1.6"/></svg>"##,
    ),
    (
        "chevron-down",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"##,
    ),
    (
        "chevron-right",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 6 6 6-6 6"/></svg>"##,
    ),
    (
        "copy",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="11" height="11" rx="1.6"/><path d="M15 6.5V5.6A1.6 1.6 0 0 0 13.4 4H5.6A1.6 1.6 0 0 0 4 5.6v7.8A1.6 1.6 0 0 0 5.6 15h.9"/></svg>"##,
    ),
    (
        "refresh",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M20 12a8 8 0 1 1-2.4-5.7"/><path d="M20 4v4.5h-4.5"/></svg>"##,
    ),
    (
        "connect",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 12h9"/><path d="m10 8.5 3.5 3.5L10 15.5"/><path d="M15.5 5.5H18A2.5 2.5 0 0 1 20.5 8v8a2.5 2.5 0 0 1-2.5 2.5h-2.5"/></svg>"##,
    ),
    (
        "shield",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l7 3v5.5c0 4.3-2.9 7.6-7 9.5-4.1-1.9-7-5.2-7-9.5V6z"/><path d="m9 12 2 2 4-4"/></svg>"##,
    ),
    (
        "star",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m12 4 2.5 5.1 5.6.8-4 4 .9 5.6-5-2.7-5 2.7.9-5.6-4-4 5.6-.8z"/></svg>"##,
    ),
    (
        "eye",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 12S6 6.5 12 6.5 21.5 12 21.5 12 18 17.5 12 17.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.8"/></svg>"##,
    ),
];

fn source(name: &str) -> &'static str {
    ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
        .unwrap_or(ICONS[0].1)
}

/// The icon as an egui image, already tinted.
pub fn image(name: &str, size: f32, color: egui::Color32) -> egui::Image<'static> {
    let uri = ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, _)| *n)
        .unwrap_or("home");
    egui::Image::from_bytes(
        format!("bytes://fv-icon-{}.svg", uri),
        source(name).as_bytes(),
    )
    .fit_to_exact_size(egui::vec2(size, size))
    .tint(color)
}

/// Draw an icon inline (no interaction).
pub fn show(ui: &mut egui::Ui, name: &str, size: f32, color: egui::Color32) {
    ui.add(image(name, size, color));
}

/// A flat icon button, e.g. in a table row.
pub fn button(ui: &mut egui::Ui, name: &str, size: f32, color: egui::Color32) -> egui::Response {
    ui.add(
        egui::Button::image(image(name, size, color))
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE)
            .min_size(egui::vec2(size + 10.0, size + 8.0)),
    )
}

/// Icon + text button used in the toolbars.
pub fn text_button(
    ui: &mut egui::Ui,
    name: &str,
    label: &str,
    on: bool,
) -> egui::Response {
    let col = if on {
        crate::theme::accent()
    } else {
        crate::theme::muted()
    };
    let r = ui.add(
        egui::Button::image_and_text(
            image(name, 16.0, col),
            egui::RichText::new(label).size(12.5).color(if on {
                crate::theme::text()
            } else {
                crate::theme::muted()
            }),
        )
        .fill(if on {
            crate::theme::accent().gamma_multiply(0.16)
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE),
    );
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_wellformed_svg() {
        for (n, s) in ICONS.iter() {
            assert!(s.starts_with("<svg"), "{} faengt nicht mit <svg an", n);
            assert!(s.ends_with("</svg>"), "{} endet nicht mit </svg>", n);
            assert!(s.contains("viewBox=\"0 0 24 24\""), "{} ohne viewBox", n);
        }
    }

    #[test]
    fn names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (n, _) in ICONS.iter() {
            assert!(seen.insert(*n), "Symbol {} doppelt", n);
        }
    }

    #[test]
    fn unknown_name_gives_a_shape_instead_of_panicking() {
        assert!(source("gibtsnicht").starts_with("<svg"));
    }
}
