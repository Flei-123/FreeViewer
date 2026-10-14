# Verifies the password sealing (DPAPI, secstore.rs) and the ProgramData ACL (r73/r74)
# on a REAL Windows PC. Works only in its own temp folder (and one throw-away
# folder under C:\ProgramData in phase acl), installs nothing, changes nothing
# that exists, and removes everything at the end (tasks, temp user, folders).
#
# Build the test exe on the server (no Windows needed):
#   cd tools/win-verify/secstore-testexe
#   cargo test --locked --target x86_64-pc-windows-gnu --no-run
#   -> target/x86_64-pc-windows-gnu/debug/deps/fv_secstore_check-<hash>.exe
#
# Usage (PowerShell):
#   .\secstore-check.ps1 -Exe C:\path\fv-secstore-tests.exe                  # 1: unit tests (normal user)
#   .\secstore-check.ps1 -Exe ... -Phase xacct [-TempUser]                   # 2: ELEVATED shell
#        SYSTEM writes a sealed file (scheduled task); the current user (and
#        with -TempUser a throw-away standard user) tries to open it
#   .\secstore-check.ps1 -Exe ... -Phase acl                                 # 3: ELEVATED shell
#        applies the r74 folder ACL to C:\ProgramData\FvAclCheck-<n> and probes
#        it as a throw-away standard user
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [ValidateSet('unit', 'xacct', 'acl')][string]$Phase = 'unit',
  [switch]$TempUser
)
$ErrorActionPreference = 'Stop'
$work = Join-Path $env:TEMP 'fv-secstore-check'
$task = 'FvSecstoreCheck'
$tmpUser = 'fvchk' + (Get-Random -Maximum 9999)
$acl = $null

function Run-AsTask([string]$name, [string]$runAs, [string]$pw, [string]$cmdFile) {
  # tasks run a small .cmd file: /TR is limited to 261 characters
  if ($pw) { schtasks /Create /TN $name /SC ONCE /ST 00:00 /RU $runAs /RP $pw /TR "`"$cmdFile`"" /F | Out-Null }
  else { schtasks /Create /TN $name /SC ONCE /ST 00:00 /RU $runAs /RL HIGHEST /TR "`"$cmdFile`"" /F | Out-Null }
  schtasks /Run /TN $name | Out-Null
}
function Cleanup {
  schtasks /Delete /TN $task /F 2>$null | Out-Null
  schtasks /Delete /TN "$task-u" /F 2>$null | Out-Null
  net user $tmpUser /delete 2>$null | Out-Null
  if ($acl) { Remove-Item -Recurse -Force $acl -ErrorAction SilentlyContinue }
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
function Need-Admin {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  if (-not (New-Object Security.Principal.WindowsPrincipal $id).IsInRole('Administrator')) { throw 'run in an ELEVATED shell' }
}

try {
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
  New-Item -ItemType Directory $work | Out-Null
  # everybody may use the temp folder (the throw-away user must run the exe from there)
  icacls $work /grant '*S-1-1-0:(OI)(CI)M' | Out-Null
  Copy-Item (Resolve-Path $Exe).Path "$work\t.exe"
  $exe = "$work\t.exe"

  if ($Phase -eq 'unit') {
    # DPAPI roundtrip, wrong entropy, tampering, plain->sealed migration, ACL command lines
    & $exe secstore:: machineacl:: --test-threads=1 2>&1 | Select-Object -Last 25
    if ($LASTEXITCODE -ne 0) { throw "unit tests failed ($LASTEXITCODE)" }
    Write-Host 'OK: unit tests passed'
    return
  }

  Need-Admin

  if ($Phase -eq 'acl') {
    $acl = 'C:\ProgramData\FvAclCheck-' + (Get-Random -Maximum 99999)
    $pw = 'Fv!' + [guid]::NewGuid().ToString('N').Substring(0, 12)
    $env:FV_ACL_DIR = $acl
    & $exe machineacl::tests::win_acl_apply --ignored --exact --nocapture 2>&1 | Select-Object -Last 6
    if ($LASTEXITCODE -ne 0) { throw 'icacls apply failed' }
    icacls $acl
    icacls "$acl\identity.txt"
    net user $tmpUser $pw /add | Out-Null
    Set-Content "$work\probe.cmd" @"
@echo off
cd /d $acl
type identity.txt >nul 2>&1 && (echo identity_read=OK) || (echo identity_read=DENIED)
echo x>>identity.txt 2>nul && (echo identity_write=ALLOWED-BAD) || (echo identity_write=DENIED-good)
del identity.txt >nul 2>&1
if exist identity.txt (echo identity_delete=DENIED-good) else (echo identity_delete=ALLOWED-BAD)
echo x>>password.txt 2>nul && (echo password_write=ALLOWED-good) || (echo password_write=DENIED-BAD)
echo y>newfile.txt 2>nul && (echo create_file=ALLOWED-good) || (echo create_file=DENIED-BAD)
del newfile.txt >nul 2>&1
if exist newfile.txt (echo delete_own_new=DENIED-BAD) else (echo delete_own_new=ALLOWED-good)
"@
    Set-Content "$work\run.cmd" "@echo off`r`ncall `"$work\probe.cmd`" > `"$work\acl.log`" 2>&1"
    Run-AsTask "$task-u" $tmpUser $pw "$work\run.cmd"
    for ($i = 0; $i -lt 30 -and -not (Test-Path "$work\acl.log"); $i++) { Start-Sleep 1 }
    Start-Sleep 4
    Get-Content "$work\acl.log"
    Write-Host 'EXPECTED: identity_read=OK, identity_write/delete DENIED, password_write/create_file/delete_own_new ALLOWED'
    return
  }

  # --- xacct: SYSTEM writes, user reads ---------------------------------
  $f = "$work\sealed.txt"
  Set-Content "$work\sys.cmd" "@echo off`r`nset FV_XACCT_FILE=$f`r`n`"$exe`" secstore::tests::win_xacct_write --ignored --exact --nocapture > `"$work\sys.log`" 2>&1"
  Run-AsTask $task 'SYSTEM' $null "$work\sys.cmd"
  for ($i = 0; $i -lt 30 -and -not (Test-Path $f); $i++) { Start-Sleep 1 }
  if (-not (Test-Path $f)) { Get-Content "$work\sys.log" -ErrorAction SilentlyContinue; throw 'SYSTEM did not write the file' }
  Write-Host ('sealed file written by SYSTEM: ' + (Get-Content $f -Raw).Substring(0, 12) + '...')
  $env:FV_XACCT_FILE = $f
  & $exe secstore::tests::win_xacct_read --ignored --exact --nocapture 2>&1 | Select-Object -Last 6
  if ($LASTEXITCODE -ne 0) { throw 'current user cannot open the SYSTEM-sealed value' }
  Write-Host 'OK: SYSTEM wrote, current user read (machine scope works)'

  if ($TempUser) {
    $pw = 'Fv!' + [guid]::NewGuid().ToString('N').Substring(0, 12)
    net user $tmpUser $pw /add | Out-Null   # standard user (Users group only)
    Set-Content "$work\usr.cmd" "@echo off`r`nset FV_XACCT_FILE=$f`r`n`"$exe`" secstore::tests::win_xacct_read --ignored --exact --nocapture > `"$work\usr.log`" 2>&1"
    Run-AsTask "$task-u" $tmpUser $pw "$work\usr.cmd"
    for ($i = 0; $i -lt 30 -and -not (Test-Path "$work\usr.log"); $i++) { Start-Sleep 1 }
    Start-Sleep 4
    Get-Content "$work\usr.log" | Select-Object -Last 6
    Write-Host 'EXPECTED (documented, r74): another local standard user can ALSO open a machine-scope value.'
  }
}
finally { Cleanup }
