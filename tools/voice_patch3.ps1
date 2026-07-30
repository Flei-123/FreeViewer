$ErrorActionPreference = 'Stop'
$enc = New-Object System.Text.UTF8Encoding($false)

# 1) FV_AUDIO_MIC=1 schaltet das Mikrofon ohne Oberflaeche ein (fuer Messungen)
$p = 'C:\Users\Admin\Projects\FreeViewer\src\audio.rs'
$s = [System.IO.File]::ReadAllText($p, $enc)
$old = '            mic: AtomicBool::new(false),'
$new = '            mic: AtomicBool::new(std::env::var("FV_AUDIO_MIC").as_deref() == Ok("1")),'
if (([regex]::Matches($s, [regex]::Escape($old))).Count -ne 1) { Write-Output 'ABBRUCH mic-Anker'; exit 1 }
$s = $s.Replace($old, $new)
[System.IO.File]::WriteAllText($p, $s, $enc)
Write-Output 'audio.rs: FV_AUDIO_MIC'

# 2) Version 0.14.0
$c = 'C:\Users\Admin\Projects\FreeViewer\Cargo.toml'
$t = [System.IO.File]::ReadAllText($c, $enc)
if (([regex]::Matches($t, [regex]::Escape('version = "0.13.1"'))).Count -ne 1) { Write-Output 'ABBRUCH version'; exit 1 }
$t = $t.Replace('version = "0.13.1"', 'version = "0.14.0"')
[System.IO.File]::WriteAllText($c, $t, $enc)
Write-Output 'Cargo.toml: 0.14.0'
