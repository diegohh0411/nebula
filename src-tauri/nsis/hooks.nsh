; Tauri NSIS installer hooks — ship the Visual C++ 2015-2022 x64 runtime.
;
; nebula.exe links the dynamic CRT (/MD, see src-tauri/.cargo/config.toml), so
; MSVCP140.dll / MSVCP140_1.dll / VCRUNTIME140.dll / VCRUNTIME140_1.dll must be
; present at runtime. Static linking is off the table because ort (ONNX Runtime
; + DirectML) ships prebuilt libs against the dynamic CRT (see TT-32 / PR #33).
;
; vc_redist.x64.exe is bundled as a resource (bundle.resources in
; tauri.conf.json), so it installs to $INSTDIR\redist\vc_redist.x64.exe. We run
; it silently after install and then clean it up. "/install /quiet /norestart"
; is idempotent: it is a no-op when an equal-or-newer runtime is already present
; (e.g. exit code 1638).

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing Visual C++ 2015-2022 Redistributable (x64)..."
  ExecWait '"$INSTDIR\redist\vc_redist.x64.exe" /install /quiet /norestart' $0
  DetailPrint "vc_redist.x64.exe finished with exit code $0"
  Delete "$INSTDIR\redist\vc_redist.x64.exe"
  RMDir "$INSTDIR\redist"
!macroend
