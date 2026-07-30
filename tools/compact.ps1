$ErrorActionPreference = 'Stop'
$p = 'C:\Users\Admin\Projects\FreeViewer\src\main.rs'
$enc = New-Object System.Text.UTF8Encoding($false)
$src = [System.IO.File]::ReadAllText($p, $enc)
[System.IO.File]::WriteAllText("$p.bak_pad", $src, $enc)

# 1) alle ui.add_space(N) auf 60% zusammenziehen (Werte < 2 bleiben)
$src = [regex]::Replace($src, 'add_space\((\d+(?:\.\d+)?)\)', {
  param($m)
  $v = [double]$m.Groups[1].Value
  if ($v -lt 2) { return $m.Value }
  $n = [Math]::Max(2, [Math]::Round($v * 0.6))
  "add_space($n.0)"
})

# 2) feste Abstaende / Innenraender
$pairs = @(
  @('egui::Margin::symmetric(26, 18)', 'egui::Margin::symmetric(16, 10)'),
  @('egui::Margin::symmetric(10, 6)',  'egui::Margin::symmetric(8, 4)'),
  @('egui::Margin::symmetric(10, 7)',  'egui::Margin::symmetric(8, 5)'),
  @('egui::Margin::symmetric(12, 9)',  'egui::Margin::symmetric(11, 6)'),
  @('egui::Margin::symmetric(0, 3)',   'egui::Margin::symmetric(0, 2)'),
  @('egui::Margin::symmetric(12, 4)',  'egui::Margin::symmetric(10, 3)'),
  @('egui::Margin::same(18)',          'egui::Margin::same(12)'),
  @('.corner_radius(16)',              '.corner_radius(13)'),
  @('style.spacing.item_spacing = egui::vec2(8.0, 8.0);',   'style.spacing.item_spacing = egui::vec2(7.0, 5.0);'),
  @('style.spacing.button_padding = egui::vec2(14.0, 8.0);','style.spacing.button_padding = egui::vec2(11.0, 5.0);'),
  @('style.spacing.interact_size.y = 28.0;',                'style.spacing.interact_size.y = 24.0;'),
  @('style.spacing.window_margin = egui::Margin::same(16);','style.spacing.window_margin = egui::Margin::same(12);'),
  @('.min_size(egui::vec2(0.0, 30.0))',        '.min_size(egui::vec2(0.0, 26.0))'),
  @('.min_size(egui::vec2(210.0, 38.0))',      '.min_size(egui::vec2(200.0, 33.0))'),
  @('ghost(egui::vec2(210.0, 34.0)',           'ghost(egui::vec2(200.0, 30.0)'),
  @('ghost(egui::vec2(detail_w - 56.0, 32.0)', 'ghost(egui::vec2(detail_w - 56.0, 28.0)')
)
foreach ($pair in $pairs) {
  $n = ([regex]::Matches($src, [regex]::Escape($pair[0]))).Count
  if ($n -eq 0) { Write-Output ("FEHLT: " + $pair[0]) }
  else { Write-Output ("$n x " + $pair[0] + "  ->  " + $pair[1]) }
  $src = $src.Replace($pair[0], $pair[1])
}

[System.IO.File]::WriteAllText($p, $src, $enc)
Write-Output 'geschrieben'
