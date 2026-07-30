$enc = New-Object System.Text.UTF8Encoding($false)
$root = 'C:\Users\Admin\Projects\FreeViewer'

# 1) doppeltes "mod icons;" entfernen
$p = Join-Path $root 'src\main.rs'
$s = [System.IO.File]::ReadAllText($p, $enc)
$n = ([regex]::Matches($s, '(?m)^mod icons;\r?$')).Count
if ($n -gt 1) {
  $s = [regex]::Replace($s, '(?m)^mod icons;\r?\n', '', 1)
  Write-Host ("mod icons: " + $n + " -> " + ([regex]::Matches($s, '(?m)^mod icons;\r?$')).Count)
}

# 2) Slider: Text vor dem mutablen Borrow bilden
$old = @'
            ui.add(
                egui::Slider::new(&mut a.scale, 0.85..=1.35)
                    .show_value(false)
                    .text(format!("{:.0} %", a.scale * 100.0)),
            );
'@
$new = @'
            let pct = format!("{:.0} %", a.scale * 100.0);
            ui.add(
                egui::Slider::new(&mut a.scale, 0.85..=1.35)
                    .show_value(false)
                    .text(pct),
            );
'@
$pat = [regex]::Escape($old) -replace '\\r\\n', '\r?\n' -replace '\\n', '\r?\n'
$ms = [regex]::Matches($s, $pat)
if ($ms.Count -eq 1) {
  $s = $s.Remove($ms[0].Index, $ms[0].Length).Insert($ms[0].Index, $new)
  Write-Host 'Slider-Borrow behoben'
} else { Write-Host ("Slider-Anker " + $ms.Count + "x") }
[System.IO.File]::WriteAllText($p, $s, $enc)

# 3) tray: main_hwnd nach oben durchreichen
$tp = Join-Path $root 'src\tray.rs'
$t = [System.IO.File]::ReadAllText($tp, $enc)
if ($t -notmatch 'pub fn main_window\(') {
  $anchor = @'
#[cfg(windows)]
pub fn show_window() {
'@
  $add = @'
/// Fenstergriff des Hauptfensters - fuer die Titelleistenfarbe.
#[cfg(windows)]
pub fn main_window() -> Option<windows::Win32::Foundation::HWND> {
    unsafe { imp::main_hwnd() }
}

#[cfg(not(windows))]
pub fn main_window() -> Option<()> {
    None
}

#[cfg(windows)]
pub fn show_window() {
'@
  $pat2 = [regex]::Escape($anchor) -replace '\\r\\n', '\r?\n' -replace '\\n', '\r?\n'
  $ms2 = [regex]::Matches($t, $pat2)
  if ($ms2.Count -eq 1) {
    $t = $t.Remove($ms2[0].Index, $ms2[0].Length).Insert($ms2[0].Index, $add)
    [System.IO.File]::WriteAllText($tp, $t, $enc)
    Write-Host 'tray::main_window ergaenzt'
  } else { Write-Host ("tray-Anker " + $ms2.Count + "x") }
}

# 4) chrome.rs nutzt main_window
$cp = Join-Path $root 'src\chrome.rs'
$c = [System.IO.File]::ReadAllText($cp, $enc)
$c = $c.Replace('        let hwnd = match crate::tray::main_hwnd() {', '        let hwnd = match crate::tray::main_window() {')
[System.IO.File]::WriteAllText($cp, $c, $enc)
Write-Host 'chrome.rs auf main_window umgestellt'
