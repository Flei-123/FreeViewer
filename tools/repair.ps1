$ErrorActionPreference = 'Stop'
$root = 'C:\Users\Admin\Projects\FreeViewer'
$enc = New-Object System.Text.UTF8Encoding($false)

# 1) main.rs aus dem letzten Commit zurueckholen
Push-Location $root
& git checkout -- src/main.rs
Pop-Location
$len = (Get-Item (Join-Path $root 'src\main.rs')).Length
Write-Host ("main.rs aus Git: " + $len + " Bytes")

# 2) die drei Patchskripte so umbauen, dass sie NUR main.rs schreiben
foreach ($f in 'r3_patch.ps1', 'r4_patch.ps1', 'r5_patch.ps1') {
  $p = Join-Path $root ('tools\' + $f)
  $s = [System.IO.File]::ReadAllText($p, $enc)
  foreach ($v in '$v', '$s', '$t', '$i', '$c', '$a') {
    $s = $s.Replace('[System.IO.File]::WriteAllText($p, ' + $v + ', $enc)', '# aus (schon gepatcht): WriteAllText ' + $v)
  }
  # r5: Here-String-Ende muss am Zeilenanfang stehen
  $s = $s.Replace("    }'@ 'main: Tonstandards speichern'", "    }`r`n'@ 'main: Tonstandards speichern'")
  [System.IO.File]::WriteAllText($p, $s, $enc)
  Write-Host ("umgebaut: " + $f)
}
