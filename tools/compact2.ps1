$ErrorActionPreference = 'Stop'
$p = 'C:\Users\Admin\Projects\FreeViewer\src\main.rs'
$enc = New-Object System.Text.UTF8Encoding($false)
$src = [System.IO.File]::ReadAllText($p, $enc)

$pairs = @(
  # Karten dichter
  @('egui::Margin::same(12)',           'egui::Margin::same(10)'),
  @('style.spacing.item_spacing = egui::vec2(7.0, 5.0);', 'style.spacing.item_spacing = egui::vec2(6.0, 4.0);'),
  # Kopfzeile: Riesen-ID und Marke etwas kleiner
  @('.size(31.0).strong().color(TEXT)', '.size(27.0).strong().color(TEXT)'),
  @('RichText::new("FreeViewer").size(19.0).strong()', 'RichText::new("FreeViewer").size(17.0).strong()'),
  # Knopfhoehen
  @('.min_size(egui::vec2(0.0, 26.0))',   '.min_size(egui::vec2(0.0, 24.0))'),
  @('.min_size(egui::vec2(200.0, 33.0))', '.min_size(egui::vec2(200.0, 31.0))'),
  # Abschnittsluft
  @('    ui.add_space(4.0);
}', '    ui.add_space(3.0);
}')
)
foreach ($pair in $pairs) {
  $n = ([regex]::Matches($src, [regex]::Escape($pair[0]))).Count
  if ($n -eq 0) { Write-Output ("FEHLT: " + $pair[0]) } else { Write-Output ("$n x " + ($pair[0] -replace "`n",'\n')) }
  $src = $src.Replace($pair[0], $pair[1])
}

# grosse Luecken (>=8) noch einmal um ein Viertel kuerzen
$src = [regex]::Replace($src, 'add_space\((\d+(?:\.\d+)?)\)', {
  param($m)
  $v = [double]$m.Groups[1].Value
  if ($v -lt 8) { return $m.Value }
  $n = [Math]::Max(6, [Math]::Round($v * 0.75))
  "add_space($n.0)"
})

[System.IO.File]::WriteAllText($p, $src, $enc)
Write-Output 'geschrieben'
