$ErrorActionPreference = 'Stop'
$root = 'C:\Users\Admin\Projects\FreeViewer'
$enc = New-Object System.Text.UTF8Encoding($false)
$mainPath = Join-Path $root 'src\main.rs'
$s = [System.IO.File]::ReadAllText($mainPath, $enc)
$fail = 0

function Once($text, $anchor) {
  ([regex]::Matches($text, [regex]::Escape($anchor))).Count -eq 1
}

# ---------------------------------------------------------------- Cargo.toml
$cp = Join-Path $root 'Cargo.toml'
$c = [System.IO.File]::ReadAllText($cp, $enc)
if ($c -notmatch 'egui_extras') {
  $c = $c.Replace('egui = "0.33"', "egui = `"0.33`"`negui_extras = { version = `"0.33`", features = [`"svg`"] }")
  [System.IO.File]::WriteAllText($cp, $c, $enc)
  Write-Output 'Cargo.toml: egui_extras + svg'
}

# ------------------------------------------------------------------ Module
foreach ($pair in @(@('mod ident;', "mod icons;`nmod ident;"), @('mod tray;', "mod theme;`nmod tray;"))) {
  if (Once $s $pair[0]) { $s = $s.Replace($pair[0], $pair[1]); Write-Output ("Modul: " + $pair[1].Replace("`n", ' + ')) }
  else { Write-Output ("FEHLER Modul-Anker: " + $pair[0]); $fail++ }
}

# ------------------------------------------------- Farbkonstanten -> Palette
$constBlock = [regex]::Match($s, '(?s)// Hausstil von fleitec.*?const TEXT: egui::Color32 = egui::Color32::from_rgb\(0xe7, 0xeb, 0xf3\);\r?\n')
if ($constBlock.Success) {
  $s = $s.Remove($constBlock.Index, $constBlock.Length)
  Write-Output 'Farbkonstanten entfernt'
} else { Write-Output 'FEHLER: Farbblock nicht gefunden'; $fail++ }

foreach ($n in 'CARD_HI','ROW_SEL','CARD','FIELD','LINE','ACCENT','VIOLET','GREEN','MUTED','TEXT','BG') {
  $fn = 'theme::' + $n.ToLower() + '()'
  $before = ([regex]::Matches($s, "\b$n\b")).Count
  $s = [regex]::Replace($s, "\b$n\b", $fn)
  Write-Output ("Farbe {0} -> {1} ({2}x)" -f $n, $fn, $before)
}

# ------------------------------------------------------------- install_theme
$it = [regex]::Match($s, '(?s)fn install_theme\(ctx: &egui::Context\) \{.*?\r?\n\}\r?\n')
if ($it.Success) {
  $new = @'
fn install_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    // SVG-Symbole: der Bildlader von egui_extras rastert sie in der Groesse,
    // in der sie gebraucht werden.
    egui_extras::install_image_loaders(ctx);
    theme::apply(ctx, &theme::load());
}
'@
  $s = $s.Remove($it.Index, $it.Length).Insert($it.Index, $new)
  Write-Output 'install_theme ersetzt'
} else { Write-Output 'FEHLER: install_theme nicht gefunden'; $fail++ }

# ------------------------------------------------------------------ View
if (Once $s "enum View {") {
  $s = $s.Replace(@'
enum View {
    Start,
    Devices,
    Settings,
}
'@, @'
enum View {
    Start,
    Devices,
    Settings,
    Appearance,
}
'@)
  Write-Output 'View::Appearance'
} else { Write-Output 'FEHLER: View-Enum'; $fail++ }

# ------------------------------------------------------------ App-Feld look
if (Once $s '    shot_n: u32,') {
  $s = $s.Replace('    shot_n: u32,', "    /// Gewaehltes Aussehen (Vorlage, Akzent, Groesse, Rundung).`n    look: theme::Appearance,`n    shot_n: u32,")
  Write-Output 'Feld look'
} else { Write-Output 'FEHLER: shot_n-Feld'; $fail++ }
if (Once $s '            shot_n: 0,') {
  $s = $s.Replace('            shot_n: 0,', "            look: theme::load(),`n            shot_n: 0,")
  Write-Output 'look initialisiert'
} else { Write-Output 'FEHLER: shot_n-Init'; $fail++ }

# --------------------------------------------------------------- home_ui
$hu = [regex]::Match($s, '(?s)    fn home_ui\(&mut self, ui: &mut egui::Ui\) \{.*?\r?\n    \}\r?\n')
if ($hu.Success) {
  $new = @'
    fn home_ui(&mut self, ui: &mut egui::Ui) {
        match self.view {
            View::Start => self.start_view(ui),
            View::Devices => self.devices_view(ui),
            View::Settings => self.settings_view(ui),
            View::Appearance => self.appearance_view(ui),
        }
    }
'@
  $s = $s.Remove($hu.Index, $hu.Length).Insert($hu.Index, $new)
  Write-Output 'home_ui ohne top_bar'
} else { Write-Output 'FEHLER: home_ui'; $fail++ }

# ------------------------------------------------- Schale in update() setzen
$old = @'
            paint_background(ctx);
            egui::CentralPanel::default()
'@
if (Once $s $old) {
  $s = $s.Replace($old, @'
            paint_background(ctx);
            self.rail(ctx);
            self.header(ctx);
            egui::CentralPanel::default()
'@)
  Write-Output 'Schale (rail + header) eingehaengt'
} else { Write-Output 'FEHLER: update-Schale'; $fail++ }

# ----------------------------------------- devices_view + neue Seiten ersetzen
$start = $s.IndexOf('    fn devices_view(&mut self, ui: &mut egui::Ui) {')
$end = $s.IndexOf('    fn settings_view(&mut self, ui: &mut egui::Ui) {')
if ($start -lt 0 -or $end -lt 0 -or $end -le $start) {
  Write-Output 'FEHLER: devices_view-Bereich'; $fail++
} else {
  $newViews = Get-Content -Raw -Encoding UTF8 (Join-Path $root 'tools\new_views.rs.txt')
  $s = $s.Remove($start, $end - $start).Insert($start, $newViews)
  Write-Output 'devices_view ersetzt, rail/header/appearance eingefuegt'
}

if ($fail -gt 0) { Write-Output "ABBRUCH: $fail Fehler - Datei NICHT geschrieben"; exit 1 }
[System.IO.File]::WriteAllText($mainPath, $s, $enc)
Write-Output 'main.rs geschrieben'
