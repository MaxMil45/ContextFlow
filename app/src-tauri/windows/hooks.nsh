; Run by the NSIS installer. Everything is per-user (HKCU), so no admin prompt.

!macro NSIS_HOOK_POSTINSTALL
  ; Register the browser host for Chrome and Edge and unpack the extension.
  ; The one manual step (Load unpacked) is still the user's.
  nsExec::Exec '"$INSTDIR\contextflow.exe" browser install'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec '"$INSTDIR\contextflow.exe" browser uninstall'
  Pop $0
!macroend
