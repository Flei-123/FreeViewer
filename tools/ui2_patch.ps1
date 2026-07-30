$ErrorActionPreference = 'Stop'
$root = 'C:\Users\Admin\Projects\FreeViewer'
$enc = New-Object System.Text.UTF8Encoding($false)
$fail = 0

function Read-Src($rel) { [System.IO.File]::ReadAllText((Join-Path $root $rel), $enc) }
function Write-Src($rel, $text) { [System.IO.File]::WriteAllText((Join-Path $root $rel), $text, $enc) }
function Rep($text, $old, $new, $what) {
  $n = ([regex]::Matches($text, [regex]::Escape($old))).Count
  if ($n -ne 1) { Write-Output ("FEHLER {0}: Anker {1}x" -f $what, $n); $script:fail++; return $text }
  Write-Output ("ok " + $what)
  return $text.Replace($old, $new)
}

# =========================================================== Cargo: DWM
$c = Read-Src 'Cargo.toml'
if ($c -notmatch 'Win32_Graphics_Dwm') {
  $c = Rep $c '    "Win32_Graphics_Dxgi",' "    `"Win32_Graphics_Dwm`",`n    `"Win32_Graphics_Dxgi`"," 'Cargo: Win32_Graphics_Dwm'
  Write-Src 'Cargo.toml' $c
}

# =========================================================== theme.rs
$t = Read-Src 'src/theme.rs'
$t = Rep $t '    /// dark palettes paint the soft background orbs, light ones do not
    pub dark: bool,' '    /// dark palettes paint the soft background orbs, light ones do not
    pub dark: bool,
    /// Farbnebel im Hintergrund? Der FleiLauncher-Look will eine ruhige Flaeche.
    pub orbs: bool,' 'theme: Feld orbs'
$t = $t.Replace("    text: rgb(0x18, 0x20, 0x30),`n    dark: false,", "    text: rgb(0x18, 0x20, 0x30),`n    dark: false,`n    orbs: false,")
$t = $t.Replace("    text: rgb(0xe8, 0xeb, 0xf3),`n    dark: true,", "    text: rgb(0xe8, 0xeb, 0xf3),`n    dark: true,`n    orbs: false,")
$t = $t.Replace("    text: rgb(0xe7, 0xeb, 0xf3),`n    dark: true,", "    text: rgb(0xe7, 0xeb, 0xf3),`n    dark: true,`n    orbs: true,")
$t = $t.Replace("    text: rgb(0xe7, 0xeb, 0xf0),`n    dark: true,", "    text: rgb(0xe7, 0xeb, 0xf0),`n    dark: true,`n    orbs: false,")
if (([regex]::Matches($t, 'orbs: ')).Count -ne 5) { Write-Output ('FEHLER theme: orbs nur ' + ([regex]::Matches($t, 'orbs: ')).Count + 'x'); $fail++ } else { Write-Output 'ok theme: orbs in allen Vorlagen' }

$t = Rep $t '    /// 0 .. 16
    pub radius: u8,
}' '    /// 0 .. 16
    pub radius: u8,
    /// Sprachkuerzel: "de" oder "en"
    pub lang: String,
}' 'theme: Feld lang'
$t = Rep $t '            scale: 1.0,
            radius: 10,
        }' '            scale: 1.0,
            radius: 10,
            lang: "de".to_string(),
        }' 'theme: lang Default'
$t = Rep $t '            if let Some(r) = v.get("radius").and_then(|x| x.as_u64()) {
                a.radius = r.min(16) as u8;
            }' '            if let Some(r) = v.get("radius").and_then(|x| x.as_u64()) {
                a.radius = r.min(16) as u8;
            }
            if let Some(l) = v.get("lang").and_then(|x| x.as_str()) {
                if crate::i18n::LANGS.iter().any(|(c, _)| *c == l) {
                    a.lang = l.to_string();
                }
            }' 'theme: lang laden'
$t = Rep $t '    v.insert("radius".into(), serde_json::Value::from(a.radius as u64));' '    v.insert("radius".into(), serde_json::Value::from(a.radius as u64));
    v.insert("lang".into(), serde_json::Value::from(a.lang.clone()));' 'theme: lang speichern'
# etwas deutlichere Hover-/Druck-Zustaende
$t = Rep $t '        v.widgets.hovered.weak_bg_fill = p.row_sel;' '        v.widgets.hovered.weak_bg_fill = if p.dark {
            p.accent.gamma_multiply(0.20)
        } else {
            p.accent.gamma_multiply(0.12)
        };
        v.widgets.hovered.expansion = 1.0;' 'theme: Hover-Fuellung'
$t = Rep $t '        v.widgets.active.weak_bg_fill = p.row_sel;' '        v.widgets.active.weak_bg_fill = p.accent.gamma_multiply(0.30);' 'theme: Druck-Fuellung'
Write-Src 'src/theme.rs' $t

# =========================================================== audio.rs
$a = Read-Src 'src/audio.rs'
$a = Rep $a '/// `freeviewer --audiodev`' '/// Klarnamen der beiden Geraete, fuer die Einstellungen.
pub fn device_names() -> (String, String) {
    let host = cpal::default_host();
    let mic = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_else(|| "-".to_string());
    let spk = host
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_else(|| "-".to_string());
    (mic, spk)
}

/// `freeviewer --audiodev`' 'audio: device_names'
Write-Src 'src/audio.rs' $a

# =========================================================== main.rs
$m = Read-Src 'src/main.rs'

# Module
$m = Rep $m 'mod ident;' "mod i18n;`nmod icons;`nmod ident;" 'main: mod i18n'
$m = $m.Replace("mod icons;`nmod i18n;`nmod icons;", "mod i18n;`nmod icons;")   # falls icons doppelt
$m = Rep $m 'mod clip;' "mod chrome;`nmod clip;" 'main: mod chrome'

# View ohne Appearance
$m = Rep $m 'enum View {
    Start,
    Devices,
    Settings,
    Appearance,
}' 'enum View {
    Start,
    Devices,
    Settings,
}

/// Bereiche innerhalb der Einstellungen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Access,
    Audio,
    Look,
    About,
}' 'main: View + SettingsTab'
$m = Rep $m '                    Some("appearance") => View::Appearance,' '                    Some("settings2") => View::Settings,' 'main: shot-Ansicht'
$m = Rep $m '            View::Appearance => self.appearance_view(ui),
' '' 'main: home_ui ohne Appearance'

# App-Felder
$m = Rep $m '    look: theme::Appearance,' '    look: theme::Appearance,
    /// Gewaehlter Bereich in den Einstellungen.
    stab: SettingsTab,
    /// Wann die Titelleiste zuletzt eingefaerbt wurde.
    caption_tick: std::time::Instant,' 'main: Felder stab/caption_tick'
$m = Rep $m '            look: theme::load(),' '            look: theme::load(),
            stab: SettingsTab::General,
            caption_tick: std::time::Instant::now() - Duration::from_secs(9),' 'main: Felder init'

# Ueberschriften auch im hellen Modus lesbar
$m = Rep $m '            .color(egui::Color32::from_rgb(0xd4, 0xdc, 0xea)),' '            .color(theme::text()),' 'main: section-Farbe'

# Farbnebel nur wo die Vorlage ihn will
$m = Rep $m '    // zwei weiche Farborbs (viele Kreise mit wenig Deckkraft = Verlauf)' '    if !theme::palette().orbs {
        return;
    }
    // zwei weiche Farborbs (viele Kreise mit wenig Deckkraft = Verlauf)' 'main: Orbs abschaltbar'

# Sprache + Titelleiste beim Start
$m = Rep $m '    theme::apply(ctx, &theme::load());' '    let look = theme::load();
    i18n::set_lang(&look.lang);
    theme::apply(ctx, &look);' 'main: Sprache beim Start'

# Titelleiste regelmaessig nachziehen
$m = Rep $m '        self.tray_ui(ctx);' '        if self.caption_tick.elapsed() > Duration::from_secs(2) {
            self.caption_tick = std::time::Instant::now();
            chrome::paint_from_theme();
        }
        self.tray_ui(ctx);' 'main: Titelleiste nachziehen'

# Sitzungsleiste: Sprache als Symbolschalter
$voice = [regex]::Match($m, '(?s)                ui\.separator\(\);\r?\n                \{\r?\n                    let v = self\.shared\.voice\.clone\(\);.*?\r?\n                \}\r?\n')
if ($voice.Success) {
  $new = "                ui.separator();`n                self.voice_buttons(ui, 16.0);`n"
  $m = $m.Remove($voice.Index, $voice.Length).Insert($voice.Index, $new)
  Write-Output 'ok main: Sitzungsleiste mit Symbolschaltern'
} else { Write-Output 'FEHLER main: Sprachblock der Sitzungsleiste'; $fail++ }

# Startseite/Geraete/Einstellungen komplett ersetzen
$start = $m.IndexOf('    fn start_view(&mut self, ui: &mut egui::Ui) {')
$end = $m.IndexOf('    fn knock_ui(&mut self, ctx: &egui::Context) {')
if ($start -lt 0 -or $end -le $start) { Write-Output 'FEHLER main: Bereich start_view..knock_ui'; $fail++ }
else {
  $newUi = [System.IO.File]::ReadAllText((Join-Path $root 'tools\new_ui.rs.txt'), $enc)
  $m = $m.Remove($start, $end - $start).Insert($start, $newUi)
  Write-Output 'ok main: Seiten neu (Start, Geraete, Einstellungen)'
}

# Hilfsfunktionen anhaengen
$helpers = @'

// ------------------------------------------------------------ kleine Helfer

/// Symbolknopf ohne Rahmen - Hover und Druck kommen aus der Palette.
fn icon_ghost(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    let r = ui
        .scope(|ui| {
            let v = ui.visuals_mut();
            v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
            v.widgets.active.bg_stroke = egui::Stroke::NONE;
            ui.add(
                egui::Button::image(icons::image(icon, 15.0, theme::muted()))
                    .corner_radius(7)
                    .min_size(egui::vec2(28.0, 24.0)),
            )
        })
        .inner;
    r.on_hover_text(tip)
}

/// Ein- und ausschaltbarer Symbolknopf (Mikrofon, Ton).
fn icon_toggle(
    ui: &mut egui::Ui,
    icon: &str,
    on: bool,
    size: f32,
    tip: &str,
) -> egui::Response {
    let col = if on { theme::accent() } else { theme::muted() };
    let r = ui
        .scope(|ui| {
            let v = ui.visuals_mut();
            v.widgets.inactive.weak_bg_fill = if on {
                theme::accent().gamma_multiply(0.16)
            } else {
                egui::Color32::TRANSPARENT
            };
            v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
            v.widgets.active.bg_stroke = egui::Stroke::NONE;
            ui.add(
                egui::Button::image(icons::image(icon, size, col))
                    .corner_radius(8)
                    .min_size(egui::vec2(size + 14.0, size + 11.0)),
            )
        })
        .inner;
    r.on_hover_text(tip)
}

/// Zwei kleine Pegelbalken: raus und rein.
fn level_bar(ui: &mut egui::Ui, out: u32, inn: u32) {
    let p = theme::palette();
    for (val, col) in [(out, p.accent), (inn, p.green)] {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(46.0, 6.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 3.0, p.card_hi);
        let w = rect.width() * (val.min(100) as f32 / 100.0);
        if w > 0.5 {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(w, rect.height())),
                3.0,
                col,
            );
        }
        ui.add_space(4.0);
    }
}

/// Monitor oder Laptop - je nachdem, wie das Geraet heisst.
fn device_symbol(label: &str) -> &'static str {
    let l = label.to_lowercase();
    if l.contains("laptop") || l.contains("book") || l.contains("note") {
        "laptop"
    } else {
        "monitor"
    }
}

/// Offline steht grau da, online in der normalen Schrift bzw. gruen.
fn row_color(online: bool, is_text: bool) -> egui::Color32 {
    let p = theme::palette();
    if online {
        if is_text {
            p.text
        } else {
            p.green
        }
    } else if is_text {
        p.muted.gamma_multiply(0.75)
    } else {
        p.muted.gamma_multiply(0.55)
    }
}
'@
$m = $m + $helpers
Write-Output 'ok main: Helfer angehaengt'

if ($fail -gt 0) { Write-Output "ABBRUCH: $fail Fehler - main.rs NICHT geschrieben"; exit 1 }
Write-Src 'src/main.rs' $m
Write-Output 'main.rs geschrieben'
