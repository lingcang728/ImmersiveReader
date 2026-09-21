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

  ; 06-F-09: opt-in full data removal. Silent uninstalls (/S) and the default
  ; button keep everything — destructive cleanup is always an explicit user
  ; choice, never a silent default. Covers both channels: %LOCALAPPDATA%
  ; ImmersiveReader(+QA) app roots, the roaming settings dir, the Documents
  ; library roots, and the Credential Manager secrets.
  IfSilent immersive_keep_user_data
  MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "是否同时删除沉浸阅读的全部本机数据？$\r$\n$\r$\n包括：书库文稿（Documents\沉浸阅读）、设置、任务记录、缓存、知乎登录档案，以及已保存的 DeepSeek API Key。$\r$\n$\r$\n选「否」保留全部数据。" /SD IDNO IDNO immersive_keep_user_data
  ; YES falls through here.
  RMDir /r "$LOCALAPPDATA\ImmersiveReader"
  RMDir /r "$LOCALAPPDATA\ImmersiveReader-QA"
  RMDir /r "$APPDATA\immersive-reader"
  RMDir /r "$DOCUMENTS\沉浸阅读"
  RMDir /r "$DOCUMENTS\Codex\ImmersiveReader-QA"
  ExecWait 'cmdkey.exe /delete:com.lingcang.immersivereading/deepseek-api-key'
  ExecWait 'cmdkey.exe /delete:com.lingcang.immersivereading.qa/deepseek-api-key'
immersive_keep_user_data:
!macroend
