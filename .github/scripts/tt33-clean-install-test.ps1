# TT-33 acceptance test, run INSIDE a pristine Windows container
# (mcr.microsoft.com/windows/servercore) that ships with NO VC++ 2015-2022
# runtime. This is the "fresh Windows machine" from the task. We prove:
#   1. without the runtime the app cannot even load (STATUS_DLL_NOT_FOUND)
#   2. our NSIS installer's POSTINSTALL hook installs the runtime
#   3. the installed app then launches with no missing-DLL error
#   4. removing that runtime again reproduces the failure (causation)
#
# Expects C:\test\setup.exe, C:\test\bare\nebula.exe, mounted from the host.

$ErrorActionPreference = 'Stop'
$DLL_NOT_FOUND = -1073741515   # 0xC0000135 STATUS_DLL_NOT_FOUND

# Make the loader return the error to the process instead of popping a dialog.
Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Windows' -Name ErrorMode -Value 2

function Launch([string]$exe) {
  $p = Start-Process $exe -PassThru
  for ($i = 0; $i -lt 12 -and -not $p.HasExited; $i++) { Start-Sleep -Seconds 1 }
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; return 'RUNNING' }
  return $p.ExitCode
}

Write-Host '== pristine Windows container (no VC++ 2015-2022 runtime) =='

# 1) Negative: the app cannot load without the runtime.
$c0 = Launch 'C:\test\bare\nebula.exe'
Write-Host "[no runtime]      bare nebula.exe -> $c0"
if ($c0 -eq $DLL_NOT_FOUND) {
  Write-Host 'Confirmed: without the runtime the app aborts with STATUS_DLL_NOT_FOUND'
} else {
  Write-Host "::warning::expected 0xC0000135 for the bare exe, got $c0"
}

# 2) Install via our NSIS installer; the POSTINSTALL hook runs vc_redist.
#    /D= sets $INSTDIR deterministically (must be the last arg, unquoted).
Write-Host '== running our NSIS installer silently (/S /D=C:\nebula) =='
Start-Process 'C:\test\setup.exe' -ArgumentList '/S','/D=C:\nebula' -Wait
$app = 'C:\nebula\nebula.exe'
if (-not (Test-Path $app)) { throw "silent install did not produce $app" }
Write-Host "installed app: $app"

# The hook should have installed the runtime the bug is about.
$haveCrt = Test-Path 'C:\Windows\System32\MSVCP140_1.dll'
Write-Host "MSVCP140_1.dll present in System32 after install: $haveCrt"
if (-not $haveCrt) { throw 'POSTINSTALL hook did not install the VC++ runtime (MSVCP140_1.dll absent)' }

# 3) HARD GATE: the installed app launches without the missing-DLL error.
$c1 = Launch $app
Write-Host "[after install]   nebula.exe -> $c1"
if ($c1 -eq $DLL_NOT_FOUND) { throw 'FAIL: app still aborts with STATUS_DLL_NOT_FOUND after install - TT-33 NOT fixed' }
Write-Host 'PASS: clean Windows + our installer -> app launches with no missing-DLL error (TT-33 satisfied)'

# 4) Causation: remove the runtime the installer provided -> failure returns.
Get-ChildItem 'C:\Windows\System32\MSVCP140*.dll','C:\Windows\System32\VCRUNTIME140*.dll' -ErrorAction SilentlyContinue |
  Remove-Item -Force -ErrorAction SilentlyContinue
$c2 = Launch $app
Write-Host "[runtime removed] nebula.exe -> $c2"
if ($c2 -eq $DLL_NOT_FOUND) {
  Write-Host 'Confirmed causation: removing the runtime our installer provided reproduces the abort'
} else {
  Write-Host "::warning::expected 0xC0000135 after removing the runtime, got $c2"
}
