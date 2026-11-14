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

; The background agent (`contextflow-agent.exe`) keeps running between
; commands, so its file is locked too, and after an upgrade an old agent would
; go on answering with the old code. Ask it to stop first: it finishes a
; restore in progress before it exits (`agent stop` waits up to 10 seconds).
; An install from before the agent existed has no such command; that just
; fails and is ignored. Then end any copy still running from this install
; folder, the same way as the browser host. It is safe: with no agent, every
; program does its own work, and `contextflow agent start` brings it back.
!macro CF_STOP_AGENT
  nsExec::Exec '"$INSTDIR\contextflow.exe" agent stop'
  Pop $0
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'contextflow-agent.exe' -and $$_.ExecutablePath -like '$INSTDIR\*' } | Invoke-CimMethod -MethodName Terminate | Out-Null"`
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro CF_STOP_BROWSER_HOST
  !insertmacro CF_STOP_AGENT
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
  !insertmacro CF_STOP_AGENT
!macroend
