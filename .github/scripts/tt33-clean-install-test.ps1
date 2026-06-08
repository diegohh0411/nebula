# TT-33 acceptance test, run INSIDE a pristine Windows container
# (mcr.microsoft.com/windows/servercore) that ships with NO VC++ 2015-2022
# runtime - the "fresh Windows machine" from the task.
#
# What this proves deterministically:
#   1. On the clean machine the VC++ CRT DLLs are ABSENT and the app cannot even
#      load: nebula.exe aborts with STATUS_DLL_NOT_FOUND (0xC0000135) - exactly
#      the TT-33 failure (missing MSVCP140_1.dll & friends).
#   2. After running OUR NSIS installer (/S), the POSTINSTALL hook has installed
#      the full VC++ 2015-2022 runtime into System32 - the precise DLLs the bug
#      is about. The missing-DLL blocker is gone.
#
# Note: servercore is headless and lacks the desktop/WebView2 DLLs a Tauri
# window needs, so a *visual* launch isn't possible here (and is unrelated to
# TT-33). The user's clean desktop VM covers that last visual step; this job
# proves the runtime-shipping fix end to end on a genuinely clean machine.
#
# Expects C:\test\setup.exe and C:\test\bare\nebula.exe (mounted from the host).

$ErrorActionPreference = 'Stop'
$DLL_NOT_FOUND = -1073741515   # 0xC0000135 STATUS_DLL_NOT_FOUND
$sys = 'C:\Windows\System32'
$crt = 'MSVCP140.dll','MSVCP140_1.dll','VCRUNTIME140.dll','VCRUNTIME140_1.dll'

# Make the loader return the error to the process instead of popping a dialog.
Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Windows' -Name ErrorMode -Value 2

function Launch([string]$exe) {
  $p = Start-Process $exe -PassThru
  for ($i = 0; $i -lt 12 -and -not $p.HasExited; $i++) { Start-Sleep -Seconds 1 }
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; return 'RUNNING' }
  return $p.ExitCode
}

Write-Host '== pristine Windows container (no VC++ 2015-2022 runtime) =='

# --- 1) Clean machine: runtime absent, app cannot load ---
$present = $crt | Where-Object { Test-Path "$sys\$_" }
if ($present) { throw "Container is not clean - VC++ runtime already present: $($present -join ', ')" }
Write-Host "Confirmed clean: none of [$($crt -join ', ')] are present"

$c0 = Launch 'C:\test\bare\nebula.exe'
Write-Host "[no runtime]    bare nebula.exe -> $c0"
if ($c0 -ne $DLL_NOT_FOUND) { throw "Expected STATUS_DLL_NOT_FOUND on the clean machine, got $c0" }
Write-Host 'Reproduced TT-33: without the VC++ runtime the app aborts (STATUS_DLL_NOT_FOUND)'

# --- 2) Install via our NSIS installer; POSTINSTALL hook runs vc_redist ---
#         /D= sets $INSTDIR deterministically (must be the last arg, unquoted).
Write-Host '== running our NSIS installer silently (/S /D=C:\nebula) =='
Start-Process 'C:\test\setup.exe' -ArgumentList '/S','/D=C:\nebula' -Wait
if (-not (Test-Path 'C:\nebula\nebula.exe')) { throw 'silent install did not produce C:\nebula\nebula.exe' }
Write-Host 'app installed to C:\nebula\nebula.exe'

# --- HARD GATE: the hook installed the exact VC++ runtime the bug names ---
$still = $crt | Where-Object { -not (Test-Path "$sys\$_") }
if ($still) { throw "POSTINSTALL hook did not install the VC++ runtime; still missing: $($still -join ', ')" }
Write-Host "PASS: our installer placed the full VC++ 2015-2022 runtime on the clean machine:"
$crt | ForEach-Object { Write-Host "    $sys\$_  ->  $((Get-Item "$sys\$_").Length) bytes" }
Write-Host 'TT-33 satisfied: a fresh machine that runs our installer gets the missing MSVCP140_1.dll (and the rest of the CRT).'
