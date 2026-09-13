; Uninstall hooks for the NSIS bundle. The default uninstaller only removes
; $INSTDIR — everything below was written under HKCU by
; install-latest-immersive-reader.ps1 (ship:local) and would otherwise point
; at a deleted executable.
;
; Deliberately NOT touched: %LOCALAPPDATA%\ImmersiveReader, %APPDATA%\
; immersive-reader, the Documents\沉浸阅读 library, and Credential Manager
; secrets — those are user data, not registration residue.

!macro NSIS_HOOK_POSTUNINSTALL
  ; Markdown ProgId + file-extension pointers written by the installer script.
  DeleteRegKey HKCU "Software\Classes\ImmersiveReader.Markdown"
  DeleteRegValue HKCU "Software\Classes\.md\OpenWithProgids" "ImmersiveReader.Markdown"
  DeleteRegValue HKCU "Software\Classes\.markdown\OpenWithProgids" "ImmersiveReader.Markdown"
  ; .md/.markdown (default) = our ProgId only when the script set it — clear
  ; the dangling pointer in that case, leave a foreign association alone.
  ReadRegStr $0 HKCU "Software\Classes\.md" ""
  StrCmp $0 "ImmersiveReader.Markdown" 0 +2
    DeleteRegValue HKCU "Software\Classes\.md" ""
  ReadRegStr $0 HKCU "Software\Classes\.markdown" ""
  StrCmp $0 "ImmersiveReader.Markdown" 0 +2
    DeleteRegValue HKCU "Software\Classes\.markdown" ""

  ; Legacy `md` ProgId: the install script rewrites its open command only when
  ; it already points at this product family. If that command now resolves to
  ; this install's exe, the entry dies with the app — drop the stale command
  ; and icon, keep any foreign-owned `md` registration untouched.
  ReadRegStr $0 HKCU "Software\Classes\md\shell\open\command" ""
  StrCmp $0 '"$INSTDIR\immersive-reader.exe" "%1"' 0 +3
    DeleteRegValue HKCU "Software\Classes\md\shell\open\command" ""
    DeleteRegKey HKCU "Software\Classes\md\DefaultIcon"

  ; Shell identity: Open-With app entry, App Paths, capability declaration,
  ; RegisteredApplications entry.
  DeleteRegKey HKCU "Software\Classes\Applications\immersive-reader.exe"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\App Paths\immersive-reader.exe"
  DeleteRegKey HKCU "Software\ImmersiveReader"
  DeleteRegValue HKCU "Software\RegisteredApplications" "沉浸阅读"
!macroend
