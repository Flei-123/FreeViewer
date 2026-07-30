//! Look of the window: four ready made palettes, an accent colour, text size
//! and corner radius - the same idea as the "Darstellung" page in the FleiTec
//! cockpit, only native.
//!
//! Everything is stored in <config>/appearance.json, so the choice survives a
//! restart and an update.

use egui::Color32;
use std::sync::RwLock;

#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    /// window background
    pub bg: Color32,
    /// cards, panels, table rows
    pub card: Color32,
    /// slightly lifted surface (hover, secondary buttons)
    pub card_hi: Color32,
    /// selected row
    pub row_sel: Color32,
    /// text fields
    pub field: Color32,
    /// hair lines and borders
    pub line: Color32,
    pub accent: Color32,
    /// text/icon colour ON the accent
    pub on_accent: Color32,
    pub violet: Color32,
    pub green: Color32,
    pub muted: Color32,
    pub text: Color32,
    /// dark palettes paint the soft background orbs, light ones do not
    pub dark: bool,
    /// Farbnebel im Hintergrund? Der FleiLauncher-Look will eine ruhige Flaeche.
    pub orbs: bool,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Bright, like TeamViewer.
pub const HELL: Palette = Palette {
    bg: rgb(0xf4, 0xf6, 0xfb),
    card: rgb(0xff, 0xff, 0xff),
    card_hi: rgb(0xef, 0xf2, 0xf9),
    row_sel: rgb(0xe4, 0xed, 0xff),
    field: rgb(0xff, 0xff, 0xff),
    line: rgb(0xdd, 0xe3, 0xee),
    accent: rgb(0x0b, 0x5c, 0xff),
    on_accent: rgb(0xff, 0xff, 0xff),
    violet: rgb(0x7c, 0x3a, 0xed),
    green: rgb(0x14, 0x9d, 0x52),
    muted: rgb(0x67, 0x70, 0x85),
    text: rgb(0x18, 0x20, 0x30),
    dark: false,
    orbs: false,
};

/// TeamViewer, but at night.
pub const DUNKEL: Palette = Palette {
    bg: rgb(0x13, 0x15, 0x1b),
    card: rgb(0x1b, 0x1e, 0x26),
    card_hi: rgb(0x23, 0x27, 0x31),
    row_sel: rgb(0x2b, 0x31, 0x40),
    field: rgb(0x15, 0x18, 0x1f),
    line: rgb(0x2d, 0x32, 0x3d),
    accent: rgb(0x2f, 0x86, 0xff),
    on_accent: rgb(0xff, 0xff, 0xff),
    violet: rgb(0x8b, 0x5c, 0xf6),
    green: rgb(0x22, 0xc5, 0x5e),
    muted: rgb(0x9a, 0xa3, 0xb5),
    text: rgb(0xe8, 0xeb, 0xf3),
    dark: true,
    orbs: false,
};

/// The FleiTec house style (what 0.13 looked like).
pub const NAVY: Palette = Palette {
    bg: rgb(0x07, 0x09, 0x0f),
    card: rgb(0x0e, 0x12, 0x20),
    card_hi: rgb(0x12, 0x17, 0x28),
    row_sel: rgb(0x16, 0x1e, 0x36),
    field: rgb(0x0a, 0x0e, 0x1a),
    line: rgb(0x1e, 0x25, 0x38),
    accent: rgb(0x38, 0xbd, 0xf8),
    on_accent: rgb(0x07, 0x09, 0x0f),
    violet: rgb(0x8b, 0x5c, 0xf6),
    green: rgb(0x22, 0xc5, 0x5e),
    muted: rgb(0x8b, 0x95, 0xab),
    text: rgb(0xe7, 0xeb, 0xf3),
    dark: true,
    orbs: true,
};

/// Like the FleiLauncher: near black with a green accent.
pub const GRUEN: Palette = Palette {
    bg: rgb(0x07, 0x09, 0x0f),
    card: rgb(0x0d, 0x11, 0x17),
    card_hi: rgb(0x12, 0x17, 0x1f),
    row_sel: rgb(0x16, 0x1d, 0x26),
    field: rgb(0x0a, 0x0d, 0x12),
    line: rgb(0x1d, 0x23, 0x2c),
    accent: rgb(0x1b, 0xd9, 0x6a),
    on_accent: rgb(0x07, 0x09, 0x0f),
    violet: rgb(0xa8, 0x55, 0xf7),
    green: rgb(0x1b, 0xd9, 0x6a),
    muted: rgb(0x8b, 0x95, 0x9f),
    text: rgb(0xe7, 0xeb, 0xf0),
    dark: true,
    orbs: false,
};

pub const PRESETS: [(&str, &str, Palette); 4] = [
    ("hell", "Hell", HELL),
    ("dunkel", "Dunkel", DUNKEL),
    ("navy", "FleiTec Navy", NAVY),
    ("gruen", "FleiLauncher", GRUEN),
];

/// The six accents the cockpit offers, plus "wie die Vorlage".
pub const ACCENTS: [(&str, Color32); 6] = [
    ("Blau", rgb(0x2f, 0x86, 0xff)),
    ("Himmel", rgb(0x38, 0xbd, 0xf8)),
    ("Gruen", rgb(0x1b, 0xd9, 0x6a)),
    ("Violett", rgb(0xa8, 0x55, 0xf7)),
    ("Orange", rgb(0xf5, 0x9e, 0x0b)),
    ("Rot", rgb(0xef, 0x44, 0x44)),
];

static ACTIVE: RwLock<Palette> = RwLock::new(DUNKEL);

pub fn palette() -> Palette {
    *ACTIVE.read().unwrap()
}

pub fn bg() -> Color32 {
    palette().bg
}
pub fn card() -> Color32 {
    palette().card
}
pub fn card_hi() -> Color32 {
    palette().card_hi
}
pub fn row_sel() -> Color32 {
    palette().row_sel
}
pub fn field() -> Color32 {
    palette().field
}
pub fn line() -> Color32 {
    palette().line
}
pub fn accent() -> Color32 {
    palette().accent
}
pub fn on_accent() -> Color32 {
    palette().on_accent
}
pub fn violet() -> Color32 {
    palette().violet
}
pub fn green() -> Color32 {
    palette().green
}
pub fn muted() -> Color32 {
    palette().muted
}
pub fn text() -> Color32 {
    palette().text
}
pub fn is_dark() -> bool {
    palette().dark
}

/// What the user picked.
#[derive(Clone, PartialEq)]
pub struct Appearance {
    /// key out of PRESETS
    pub preset: String,
    /// None = keep the accent of the preset
    pub accent: Option<[u8; 3]>,
    /// 0.85 .. 1.35
    pub scale: f32,
    /// 0 .. 16
    pub radius: u8,
    /// Sprachkuerzel: "de" oder "en"
    pub lang: String,
    /// Mikrofon beim Verbinden gleich an?
    pub mic_on: bool,
    /// Ton der anderen Seite beim Verbinden gleich an?
    pub snd_on: bool,
    /// Gewaehltes Mikrofon (None = Systemstandard)
    pub mic_dev: Option<String>,
    /// Gewaehlte Wiedergabe (None = Systemstandard)
    pub spk_dev: Option<String>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            preset: "dunkel".to_string(),
            accent: None,
            scale: 1.0,
            radius: 10,
            lang: "de".to_string(),
            mic_on: false,
            snd_on: false,
            mic_dev: None,
            spk_dev: None,
        }
    }
}

fn path() -> std::path::PathBuf {
    crate::ident::config_dir().join("appearance.json")
}

pub fn load() -> Appearance {
    let mut a = Appearance::default();
    if let Ok(raw) = std::fs::read_to_string(path()) {
        // von Hand geschriebene Dateien haben oft ein BOM davor
        let s = raw.trim_start_matches('\u{feff}').to_string();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
            if let Some(p) = v.get("preset").and_then(|x| x.as_str()) {
                if PRESETS.iter().any(|(k, _, _)| *k == p) {
                    a.preset = p.to_string();
                }
            }
            if let Some(arr) = v.get("accent").and_then(|x| x.as_array()) {
                if arr.len() == 3 {
                    let mut c = [0u8; 3];
                    for (i, x) in arr.iter().enumerate() {
                        c[i] = x.as_u64().unwrap_or(0).min(255) as u8;
                    }
                    a.accent = Some(c);
                }
            }
            if let Some(s) = v.get("scale").and_then(|x| x.as_f64()) {
                a.scale = (s as f32).clamp(0.85, 1.35);
            }
            if let Some(r) = v.get("radius").and_then(|x| x.as_u64()) {
                a.radius = r.min(16) as u8;
            }
            if let Some(b) = v.get("mic_on").and_then(|x| x.as_bool()) {
                a.mic_on = b;
            }
            if let Some(b) = v.get("snd_on").and_then(|x| x.as_bool()) {
                a.snd_on = b;
            }
            if let Some(s) = v.get("mic_dev").and_then(|x| x.as_str()) {
                if !s.is_empty() {
                    a.mic_dev = Some(s.to_string());
                }
            }
            if let Some(s) = v.get("spk_dev").and_then(|x| x.as_str()) {
                if !s.is_empty() {
                    a.spk_dev = Some(s.to_string());
                }
            }
            if let Some(l) = v.get("lang").and_then(|x| x.as_str()) {
                if crate::i18n::LANGS.iter().any(|(c, _)| *c == l) {
                    a.lang = l.to_string();
                }
            }
        }
    }
    a
}

pub fn save(a: &Appearance) {
    let mut v = serde_json::Map::new();
    v.insert("preset".into(), serde_json::Value::from(a.preset.clone()));
    if let Some(c) = a.accent {
        v.insert(
            "accent".into(),
            serde_json::Value::from(vec![c[0] as u64, c[1] as u64, c[2] as u64]),
        );
    }
    v.insert("scale".into(), serde_json::Value::from(a.scale as f64));
    v.insert("radius".into(), serde_json::Value::from(a.radius as u64));
    v.insert("lang".into(), serde_json::Value::from(a.lang.clone()));
    v.insert("mic_on".into(), serde_json::Value::from(a.mic_on));
    v.insert("snd_on".into(), serde_json::Value::from(a.snd_on));
    v.insert(
        "mic_dev".into(),
        serde_json::Value::from(a.mic_dev.clone().unwrap_or_default()),
    );
    v.insert(
        "spk_dev".into(),
        serde_json::Value::from(a.spk_dev.clone().unwrap_or_default()),
    );
    let _ = std::fs::create_dir_all(crate::ident::config_dir());
    let _ = std::fs::write(
        path(),
        serde_json::to_string_pretty(&serde_json::Value::Object(v)).unwrap_or_default(),
    );
}

/// Builds the palette the settings describe.
pub fn resolve(a: &Appearance) -> Palette {
    let mut p = PRESETS
        .iter()
        .find(|(k, _, _)| *k == a.preset)
        .map(|(_, _, p)| *p)
        .unwrap_or(DUNKEL);
    if let Some(c) = a.accent {
        p.accent = Color32::from_rgb(c[0], c[1], c[2]);
        // keep the label on the accent readable
        let lum = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
        p.on_accent = if lum > 150.0 {
            Color32::from_rgb(0x10, 0x14, 0x1c)
        } else {
            Color32::WHITE
        };
    }
    p
}

/// Puts the palette in place and rebuilds the egui style around it.
pub fn apply(ctx: &egui::Context, a: &Appearance) {
    crate::audio::set_defaults(a.mic_on, a.snd_on);
    crate::audio::set_devices(a.mic_dev.clone(), a.spk_dev.clone());
    let p = resolve(a);
    *ACTIVE.write().unwrap() = p;

    let mut style = (*ctx.style()).clone();
    {
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        let s = a.scale;
        style.text_styles = [
            (TextStyle::Heading, FontId::new(20.0 * s, Proportional)),
            (TextStyle::Body, FontId::new(14.0 * s, Proportional)),
            (TextStyle::Button, FontId::new(14.0 * s, Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(14.0 * s, egui::FontFamily::Monospace),
            ),
            (TextStyle::Small, FontId::new(11.5 * s, Proportional)),
        ]
        .into();
    }
    let r: u8 = a.radius;
    // Kleine Bedienelemente (Kaestchen, Schieber, Knoepfe) bleiben eckig -
    // mit der grossen Rundung wuerde aus einem Kaestchen ein Punkt.
    let rw: u8 = a.radius.min(4);
    {
        let v = &mut style.visuals;
        v.dark_mode = p.dark;
        v.panel_fill = p.bg;
        v.window_fill = p.card;
        v.extreme_bg_color = p.field;
        v.faint_bg_color = p.card_hi;
        v.override_text_color = Some(p.text);
        v.hyperlink_color = p.accent;
        v.selection.bg_fill = p.accent.gamma_multiply(0.30);
        v.selection.stroke = egui::Stroke::new(1.0, p.text);
        v.window_stroke = egui::Stroke::new(1.0, p.line);
        v.window_corner_radius = (r + 4).into();
        v.widgets.noninteractive.bg_fill = p.card;
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.line);
        v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, p.text);
        v.widgets.inactive.weak_bg_fill = p.card_hi;
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, p.line);
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, p.text);
        v.widgets.inactive.corner_radius = rw.into();
        v.widgets.hovered.weak_bg_fill = if p.dark {
            p.accent.gamma_multiply(0.20)
        } else {
            p.accent.gamma_multiply(0.12)
        };
        v.widgets.hovered.expansion = 1.0;
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, p.accent.gamma_multiply(0.7));
        v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, p.text);
        v.widgets.hovered.corner_radius = rw.into();
        v.widgets.active.weak_bg_fill = p.accent.gamma_multiply(0.30);
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0, p.accent);
        v.widgets.active.fg_stroke = egui::Stroke::new(1.0, p.text);
        v.widgets.active.corner_radius = rw.into();
        v.widgets.open.weak_bg_fill = p.row_sel;
        // Kaestchen und Schieber: Flaeche sichtbar, Haken/Griff im Akzent
        v.widgets.noninteractive.corner_radius = rw.into();
        v.widgets.inactive.bg_fill = if p.dark {
            p.card_hi
        } else {
            // Griff des Schiebers und Fuellungen: deutlich vom Weiss abgesetzt
            egui::Color32::from_rgb(0xc4, 0xcd, 0xdd)
        };
        v.widgets.hovered.bg_fill = p.accent.gamma_multiply(0.45);
        v.widgets.active.bg_fill = p.accent;
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.6, p.text);
        v.widgets.hovered.fg_stroke = egui::Stroke::new(1.8, p.text);
        v.widgets.active.fg_stroke = egui::Stroke::new(1.8, p.accent);
        v.slider_trailing_fill = true;
    }
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(11.0, 5.0);
    style.spacing.interact_size.y = 24.0;
    style.spacing.window_margin = egui::Margin::same(12);
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_resolves() {
        for (k, _, _) in PRESETS.iter() {
            let a = Appearance {
                preset: k.to_string(),
                ..Default::default()
            };
            let p = resolve(&a);
            assert!(p.text != p.bg, "Vorlage {} unlesbar", k);
        }
    }

    #[test]
    fn unknown_preset_falls_back() {
        let a = Appearance {
            preset: "gibtsnicht".into(),
            ..Default::default()
        };
        assert_eq!(resolve(&a).bg, DUNKEL.bg);
    }

    #[test]
    fn own_accent_wins_and_stays_readable() {
        let a = Appearance {
            accent: Some([255, 255, 255]),
            ..Default::default()
        };
        let p = resolve(&a);
        assert_eq!(p.accent, Color32::WHITE);
        assert!(p.on_accent != Color32::WHITE, "Text auf Weiss waere unsichtbar");
    }

    #[test]
    fn light_palette_is_actually_light() {
        assert!(!HELL.dark);
        assert!(HELL.bg.r() > 200 && HELL.text.r() < 80);
    }
}
