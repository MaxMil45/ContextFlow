; Run by the NSIS installer. Everything is per-user (HKCU), so no admin prompt.

; Chrome and Edge keep `contextflow-browser-host.exe` running while the
; extension is connected, and Windows won't let a running program's file be
; replaced or deleted. Stop the copy that runs from this install folder (only
; that one: a development build elsewhere is left alone). It is safe: the
; extension reconnects within a minute and starts the new one.
;
; The installer is a 32-bit program, so it gets the 32-bit PowerShell, which
; can't read a 64-bit program's path through Get-Process (it comes back empty).
; WMI (Get-CimInstance) reports the path either way, so match on that.
!macro CF_STOP_BROWSER_HOST
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'contextflow-browser-host.exe' -and $$_.ExecutablePath -like '$INSTDIR\*' } | Invoke-CimMethod -MethodName Terminate | Out-Null; Start-Sleep -Milliseconds 500"`
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro CF_STOP_BROWSER_HOST
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Register the browser host for Chrome and Edge and unpack the extension.
  ; The one manual step (Load unpacked) is still the user's.
  nsExec::Exec '"$INSTDIR\contextflow.exe" browser install'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec '"$INSTDIR\contextflow.exe" browser uninstall'
  Pop $0
  ; Unregistered first, so the browser can't start it again while files go.
  !insertmacro CF_STOP_BROWSER_HOST
!macroend
