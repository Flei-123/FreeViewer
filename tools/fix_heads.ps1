$enc = New-Object System.Text.UTF8Encoding($false)
$fix = @(
  @('C:\Users\Admin\Projects\FreeViewer\src\theme.rs', '//! Look of the window'),
  @('C:\Users\Admin\Projects\FreeViewer\src\audio.rs', '//! Voice for a running session'),
  @('C:\Users\Admin\Projects\FreeViewer\Cargo.toml', '[package]')
)
foreach ($f in $fix) {
  $s = [System.IO.File]::ReadAllText($f[0], $enc)
  $i = $s.IndexOf($f[1])
  if ($i -lt 0) { Write-Host ("KEIN ANKER: " + $f[0]); continue }
  if ($i -eq 0) { Write-Host ("schon sauber: " + $f[0]); continue }
  $s = $s.Substring($i)
  [System.IO.File]::WriteAllText($f[0], $s, $enc)
  Write-Host ("bereinigt (" + $i + " Zeichen Muell entfernt): " + $f[0])
}
