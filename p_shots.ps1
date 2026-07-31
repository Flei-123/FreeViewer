# Bilder der Oberflaeche - mit EIGENER Konfiguration (FV_CONFIG), damit
# Justins echte Einstellungen und Passwoerter unberuehrt bleiben.
# ACHTUNG: der Parameter darf NICHT $args heissen, das ist in PowerShell
# belegt - dann kommen die Zusatzargumente nie beim Programm an.
$ErrorActionPreference = 'Stop'
$root = 'C:\Users\Admin\Projects\FreeViewer'
$exe = "$root\target\release\freeviewer.exe"
$shots = "$root\shots"
New-Item -ItemType Directory -Force -Path $shots | Out-Null

$pwjson = '[{"label":"Handy","pw":"beispiel-abc"},{"label":"Papa","pw":"beispiel-xyz"}]'
foreach ($v in @('hell', 'dunkel')) {
  $dir = "$root\shotcfg_$v"
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  Set-Content -Path "$dir\appearance.json" -Value ('{"preset":"' + $v + '","scale":1.0,"radius":10,"lang":"de","mic_on":false,"snd_on":false,"mic_dev":"","spk_dev":""}') -Encoding ascii
  Set-Content -Path "$dir\passwords.json" -Value $pwjson -Encoding ascii
}

function Shot($cfg, $name, $extra) {
  $env:FV_CONFIG = "$root\shotcfg_$cfg"
  $out = "$shots\$name.jpg"
  if (Test-Path $out) { Remove-Item $out -Force }
  $all = @('--shot', $out) + $extra
  $p = Start-Process -FilePath $exe -ArgumentList $all -PassThru -WindowStyle Normal
  $wait = 0
  while (-not (Test-Path $out) -and $wait -lt 25) { Start-Sleep -Milliseconds 500; $wait += 0.5 }
  Start-Sleep -Milliseconds 900
  if (-not $p.HasExited) { $p.Kill() }
  if (Test-Path $out) { "ok   $name   [" + ($all -join ' ') + "]" } else { "MISS $name" }
}

Shot 'hell'   'h1_start'   @('--view', 'start', '--demo')
Shot 'hell'   'h2_access'  @('--view', 'settings', '--tab', 'access', '--demo')
Shot 'hell'   'h3_look'    @('--view', 'settings', '--tab', 'look', '--demo')
Shot 'hell'   'h4_devices' @('--view', 'devices', '--demo')
Shot 'hell'   'h5_audio'   @('--view', 'settings', '--tab', 'audio', '--demo')
Shot 'dunkel' 'd1_start'   @('--view', 'start', '--demo')
Shot 'dunkel' 'd2_audio'   @('--view', 'settings', '--tab', 'audio', '--demo')
Shot 'dunkel' 'd3_access'  @('--view', 'settings', '--tab', 'access', '--demo')
Shot 'dunkel' 'd4_look'    @('--view', 'settings', '--tab', 'look', '--demo')
$env:FV_CONFIG = ''
Write-Output 'fertig'
