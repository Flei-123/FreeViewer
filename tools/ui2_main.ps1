$ErrorActionPreference = 'Stop'
$root = 'C:\Users\Admin\Projects\FreeViewer'
$enc = New-Object System.Text.UTF8Encoding($false)
$mp = Join-Path $root 'src\main.rs'
$m = [System.IO.File]::ReadAllText($mp, $enc)
$fail = 0

# Anker zeilenendenunabhaengig ersetzen
function RepRx([string]$text, [string]$old, [string]$new, [string]$what) {
  $pat = [regex]::Escape($old) -replace '\\r\\n', '\r?\n' -replace '\\n', '\r?\n'
  $ms = [regex]::Matches($text, $pat)
  if ($ms.Count -ne 1) { Write-Host ("FEHLER {0}: {1} Treffer" -f $what, $ms.Count); $script:fail++; return $text }
  Write-Host ("ok " + $what)
  return [regex]::Replace($text, $pat, [System.Text.RegularExpressions.Regex]::Escape($new).Replace('\','\\') -replace '.*', '$0') -replace '', ''
}

# einfacher: direkter Index-Ersatz
function RepOne([string]$text, [string]$old, [string]$new, [string]$what) {
  $pat = [regex]::Escape($old) -replace '\\r\\n', '\r?\n' -replace '\\n', '\r?\n'
  $ms = [regex]::Matches($text, $pat)
  if ($ms.Count -ne 1) { Write-Host ("FEHLER {0}: {1} Treffer" -f $what, $ms.Count); $script:fail++; return $text }
  Write-Host ("ok " + $what)
  return $text.Remove($ms[0].Index, $ms[0].Length).Insert($ms[0].Index, $new)
}

$m = RepOne $m 'mod ident;' "mod i18n;`r`nmod icons;`r`nmod ident;" 'mod i18n'
$m = $m.Replace("mod i18n;`r`nmod icons;`r`nmod ident;`r`nmod icons;", "mod i18n;`r`nmod icons;`r`nmod ident;")
$m = $m.Replace("mod icons;`r`nmod i18n;`r`nmod icons;`r`nmod ident;", "mod i18n;`r`nmod icons;`r`nmod ident;")
$m = RepOne $m 'mod clip;' "mod chrome;`r`nmod clip;" 'mod chrome'

$m = RepOne $m @'
enum View {
    Start,
    Devices,
    Settings,
    Appearance,
}
'@ @'
enum View {
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
}
'@ 'View + SettingsTab'

$m = RepOne $m '                    Some("appearance") => View::Appearance,' '                    Some("settings2") => View::Settings,' 'shot-Ansicht'
$m = RepOne $m '            View::Appearance => self.appearance_view(ui),' '' 'home_ui ohne Appearance'
$m = RepOne $m '    look: theme::Appearance,' @'
    look: theme::Appearance,
    /// Gewaehlter Bereich in den Einstellungen.
    stab: SettingsTab,
    /// Wann die Titelleiste zuletzt eingefaerbt wurde.
    caption_tick: std::time::Instant,
'@ 'Felder stab/caption_tick'
$m = RepOne $m '            look: theme::load(),' @'
            look: theme::load(),
            stab: SettingsTab::General,
            caption_tick: std::time::Instant::now() - Duration::from_secs(9),
'@ 'Felder init'
$m = RepOne $m '            .color(egui::Color32::from_rgb(0xd4, 0xdc, 0xea)),' '            .color(theme::text()),' 'section-Farbe'
$m = RepOne $m '    // zwei weiche Farborbs (viele Kreise mit wenig Deckkraft = Verlauf)' @'
    if !theme::palette().orbs {
        return;
    }
    // zwei weiche Farborbs (viele Kreise mit wenig Deckkraft = Verlauf)
'@ 'Orbs abschaltbar'
$m = RepOne $m '    theme::apply(ctx, &theme::load());' @'
    let look = theme::load();
    i18n::set_lang(&look.lang);
    theme::apply(ctx, &look);
'@ 'Sprache beim Start'
$m = RepOne $m '        self.tray_ui(ctx);' @'
        if self.caption_tick.elapsed() > Duration::from_secs(2) {
            self.caption_tick = std::time::Instant::now();
            chrome::paint_from_theme();
        }
        self.tray_ui(ctx);
'@ 'Titelleiste nachziehen'

# Sitzungsleiste: Sprachblock -> Symbolschalter
$voice = [regex]::Match($m, '(?s)                ui\.separator\(\);\r?\n                \{\r?\n                    let v = self\.shared\.voice\.clone\(\);.*?\r?\n                \}\r?\n')
if ($voice.Success) {
  $m = $m.Remove($voice.Index, $voice.Length).Insert($voice.Index, "                ui.separator();`r`n                self.voice_buttons(ui, 16.0);`r`n")
  Write-Output 'ok Sitzungsleiste'
} else { Write-Output 'FEHLER Sitzungsleiste'; $fail++ }

# Seiten ersetzen
$start = $m.IndexOf('    fn start_view(&mut self, ui: &mut egui::Ui) {')
$end = $m.IndexOf('    fn knock_ui(&mut self, ctx: &egui::Context) {')
if ($start -lt 0 -or $end -le $start) { Write-Output 'FEHLER Bereich start_view..knock_ui'; $fail++ }
else {
  $newUi = [System.IO.File]::ReadAllText((Join-Path $root 'tools\new_ui.rs.txt'), $enc)
  $m = $m.Remove($start, $end - $start).Insert($start, $newUi)
  Write-Output ('ok Seiten ersetzt (' + ($end - $start) + ' Zeichen raus, ' + $newUi.Length + ' rein)')
}

# Helfer anhaengen
if ($m -notmatch 'fn icon_ghost') {
  $m = $m + [System.IO.File]::ReadAllText((Join-Path $root 'tools\helpers.rs.txt'), $enc)
  Write-Output 'ok Helfer angehaengt'
}

if ($fail -gt 0) { Write-Output "ABBRUCH: $fail Fehler"; exit 1 }
[System.IO.File]::WriteAllText($mp, $m, $enc)
Write-Output 'main.rs geschrieben'
