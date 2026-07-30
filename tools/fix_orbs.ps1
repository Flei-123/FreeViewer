$enc = New-Object System.Text.UTF8Encoding($false)
$p = 'C:\Users\Admin\Projects\FreeViewer\src\theme.rs'
$s = [System.IO.File]::ReadAllText($p, $enc)
if ($s -match 'orbs: (true|false)') { Write-Output 'schon gesetzt'; exit 0 }
$want = @('false', 'false', 'true', 'false')   # HELL, DUNKEL, NAVY, GRUEN
$i = 0
$s = [regex]::Replace($s, '(?m)^(?<ind>[ ]*)dark: (?<v>true|false),\r?$', {
  param($m)
  $line = $m.Value
  if ($script:i -lt 4) {
    $add = $m.Groups['ind'].Value + 'orbs: ' + $script:want[$script:i] + ','
    $script:i++
    return ($line + "`r`n" + $add)
  }
  return $line
})
[System.IO.File]::WriteAllText($p, $s, $enc)
Write-Output ("orbs in " + $i + " Vorlagen gesetzt")
