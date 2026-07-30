$ErrorActionPreference = 'Stop'
$root = 'C:\Users\Admin\Projects\FreeViewer'
$enc = New-Object System.Text.UTF8Encoding($false)
$script:fail = 0
function RepOne([string]$text, [string]$old, [string]$new, [string]$what) {
  $pat = [regex]::Escape($old) -replace '\\r\\n', '\r?\n' -replace '\\n', '\r?\n'
  $ms = [regex]::Matches($text, $pat)
  if ($ms.Count -ne 1) { Write-Host ("FEHLER {0}: {1} Treffer" -f $what, $ms.Count); $script:fail++; return $text }
  Write-Host ("ok " + $what)
  return $text.Remove($ms[0].Index, $ms[0].Length).Insert($ms[0].Index, $new)
}

# ===================================================== theme.rs: Widgets
$p = Join-Path $root 'src\theme.rs'
$t = [System.IO.File]::ReadAllText($p, $enc)
$t = RepOne $t '    let r: u8 = a.radius;' @'
    let r: u8 = a.radius;
    // Kleine Bedienelemente (Kaestchen, Schieber, Knoepfe) bleiben eckig -
    // mit der grossen Rundung wuerde aus einem Kaestchen ein Punkt.
    let rw: u8 = a.radius.min(4);
'@ 'theme: kleine Rundung rw'
$t = $t.Replace('        v.widgets.inactive.corner_radius = r.into();', '        v.widgets.inactive.corner_radius = rw.into();')
$t = $t.Replace('        v.widgets.hovered.corner_radius = r.into();', '        v.widgets.hovered.corner_radius = rw.into();')
$t = $t.Replace('        v.widgets.active.corner_radius = r.into();', '        v.widgets.active.corner_radius = rw.into();')
$t = RepOne $t '        v.widgets.open.weak_bg_fill = p.row_sel;' @'
        v.widgets.open.weak_bg_fill = p.row_sel;
        // Kaestchen und Schieber: Flaeche sichtbar, Haken/Griff im Akzent
        v.widgets.noninteractive.corner_radius = rw.into();
        v.widgets.inactive.bg_fill = if p.dark {
            p.card_hi
        } else {
            egui::Color32::from_rgb(0xdf, 0xe4, 0xee)
        };
        v.widgets.hovered.bg_fill = p.accent.gamma_multiply(0.45);
        v.widgets.active.bg_fill = p.accent;
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.6, p.text);
        v.widgets.hovered.fg_stroke = egui::Stroke::new(1.8, p.text);
        v.widgets.active.fg_stroke = egui::Stroke::new(1.8, p.accent);
        v.slider_trailing_fill = true;
'@ 'theme: Kaestchen und Schieber'
# aus (schon gepatcht): WriteAllText $t

# ===================================================== main.rs
$p = Join-Path $root 'src\main.rs'
$m = [System.IO.File]::ReadAllText($p, $enc)

# Rail im hellen Modus: klare weisse Symbole, gewaehlt = weisses Feld
$m = RepOne $m '                        let fg = if p.dark {
                            if sel {
                                p.accent
                            } else {
                                p.muted
                            }
                        } else if sel {
                            p.on_accent
                        } else {
                            p.on_accent.gamma_multiply(0.72)
                        };' @'
                        let fg = if p.dark {
                            if sel {
                                p.accent
                            } else {
                                p.muted
                            }
                        } else if sel {
                            p.accent
                        } else {
                            egui::Color32::WHITE
                        };
'@ 'main: Rail-Symbole hell'
$m = RepOne $m '                        let fill = if sel {
                            if p.dark {
                                p.accent.gamma_multiply(0.16)
                            } else {
                                egui::Color32::from_white_alpha(46)
                            }
                        } else {
                            egui::Color32::TRANSPARENT
                        };' @'
                        let fill = if sel {
                            if p.dark {
                                p.accent.gamma_multiply(0.16)
                            } else {
                                egui::Color32::WHITE
                            }
                        } else {
                            egui::Color32::TRANSPARENT
                        };
'@ 'main: Rail-Feld hell'

# Zwischenablage-Schalter in "Zugriff"
$m = RepOne $m '            ui.add_space(4.0);
            let mut keep = self.pw_fixed;' @'
            ui.add_space(4.0);
            let mut clip = self.shared.clip_on.load(Ordering::Relaxed);
            if ui
                .checkbox(&mut clip, i18n::t("set.clip"))
                .on_hover_text(i18n::t("set.clip_tip"))
                .changed()
            {
                self.shared.clip_on.store(clip, Ordering::Relaxed);
                ident::set_clipboard(clip);
            }
            ui.add_space(4.0);
            let mut keep = self.pw_fixed;
'@ 'main: Zwischenablage-Schalter'

# Hinweis zur Verschluesselung auf der Info-Seite
$m = RepOne $m '            info_row(ui, i18n::t("set.relay"), &self.shared.relay_url);' @'
            info_row(ui, i18n::t("set.relay"), &self.shared.relay_url);
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new(i18n::t("set.e2e_note"))
                    .size(11.0)
                    .color(theme::muted()),
            );
'@ 'main: Hinweis Ende zu Ende'
[System.IO.File]::WriteAllText($p, $m, $enc)

# ===================================================== shared.rs: clip_on
$p = Join-Path $root 'src\shared.rs'
$s = [System.IO.File]::ReadAllText($p, $enc)
$s = RepOne $s '    pub stats: Mutex<Stats>,' @'
    /// Zwischenablage in beide Richtungen abgleichen?
    pub clip_on: AtomicBool,
    pub stats: Mutex<Stats>,
'@ 'shared: clip_on'
$s = RepOne $s '            stats: Mutex::new(Stats::default()),' @'
            clip_on: AtomicBool::new(crate::ident::clipboard_enabled()),
            stats: Mutex::new(Stats::default()),
'@ 'shared: clip_on init'
# aus (schon gepatcht): WriteAllText $s

# ===================================================== ident.rs: Schalterdatei
$p = Join-Path $root 'src\ident.rs'
$i = [System.IO.File]::ReadAllText($p, $enc)
if ($i -notmatch 'fn clipboard_enabled') {
  $i = $i + @'

/// Zwischenablage teilen? Aus, wenn die Datei "noclip" im Ordner liegt.
pub fn clipboard_enabled() -> bool {
    !config_dir().join("noclip").exists()
}

pub fn set_clipboard(on: bool) {
    let f = config_dir().join("noclip");
    if on {
        let _ = std::fs::remove_file(f);
    } else {
        let _ = std::fs::create_dir_all(config_dir());
        let _ = std::fs::write(f, b"1");
    }
}
'@
  Write-Host 'ok ident: clipboard_enabled'
} else { Write-Host 'ident: schon da' }
# aus (schon gepatcht): WriteAllText $i

# ===================================================== clip.rs: Schalter achten
$p = Join-Path $root 'src\clip.rs'
$c = [System.IO.File]::ReadAllText($p, $enc)
if ($c -match 'clip_on') { Write-Host 'clip.rs: schon verdrahtet' }
else {
  $c2 = [regex]::Replace($c, '(?m)^(\s*)(if let Some\(text\) = )', '$1if !shared.clip_on.load(std::sync::atomic::Ordering::Relaxed) { std::thread::sleep(std::time::Duration::from_millis(600)); continue; }
$1$2', 1)
  if ($c2 -eq $c) { Write-Host 'HINWEIS clip.rs: kein Anker, Schalter wirkt nur in der Oberflaeche' } else { Write-Host 'ok clip.rs' ; $c = $c2 }
  # aus (schon gepatcht): WriteAllText $c
}

# ===================================================== i18n
$p = Join-Path $root 'src\i18n.rs'
$s = [System.IO.File]::ReadAllText($p, $enc)
$s = RepOne $s '    // Update und Rueckmeldung' @'
    ("set.clip", "Zwischenablage teilen", "Share clipboard"),
    (
        "set.clip_tip",
        "Kopierter Text gilt auf beiden Rechnern - in beide Richtungen, nur Text.",
        "Copied text works on both machines - both ways, text only.",
    ),
    (
        "set.e2e_note",
        "Bild, Ton, Tastatur und Dateien laufen verschlüsselt (AES-256-GCM) direkt zwischen den beiden Rechnern. Der Relay leitet nur weiter und kann nichts mitlesen; nichts wird dort gespeichert.",
        "Picture, sound, keyboard and files run encrypted (AES-256-GCM) straight between the two machines. The relay only forwards and cannot read anything; nothing is stored there.",
    ),
    // Update und Rueckmeldung
'@ 'i18n: Zwischenablage + Hinweis'
# aus (schon gepatcht): WriteAllText $s

if ($script:fail -gt 0) { Write-Host ("ABBRUCH: " + $script:fail); exit 1 }
Write-Host 'r4 fertig'
