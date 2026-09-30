; Pegoles installer hooks (Tauri NSIS, per-machine install, elevated).
;
; The only privileged component is the PegolesVmBroker service
; (pegoles-broker.exe in the install folder, which only administrators can
; write). It is registered here, demand-start, and removed on uninstall.
; Nothing else is changed on the system: no firewall rule, no Defender
; exclusion, no Windows feature (Virtual Machine Platform is turned on
; later, from the app, only after the person agrees).

!macro NSIS_HOOK_PREINSTALL
  ; An update replaces pegoles-broker.exe: stop and unregister the old
  ; service first (harmless when it is not installed).
  ${If} ${FileExists} "$INSTDIR\pegoles-broker.exe"
    nsExec::Exec '"$INSTDIR\pegoles-broker.exe" uninstall-service'
    Pop $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToLog '"$INSTDIR\pegoles-broker.exe" install-service'
  Pop $0
  ${If} $0 != "0"
    MessageBox MB_ICONEXCLAMATION|MB_OK "Pegoles could not register its virtual machine service (code $0). Pegoles will explain what to do when you open it." /SD IDOK
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Removing the service ends its computers (they live only as long as
  ; the broker and the helpers do); then stop what is left of Pegoles.
  nsExec::Exec '"$INSTDIR\pegoles-broker.exe" uninstall-service'
  Pop $0
  ; Only Pegoles' own processes: matched by their full path in this
  ; install folder, never by name (another program could use the name).
  nsExec::Exec '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -Command "$$d = [IO.Path]::GetFullPath($\'$INSTDIR$\'); Get-Process pegoles-vm-host, pegoles-llm-worker -ErrorAction SilentlyContinue | Where-Object { $$_.Path -and [IO.Path]::GetDirectoryName($$_.Path) -eq $$d } | Stop-Process -Force"'
  Pop $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
