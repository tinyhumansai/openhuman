; NSIS installer hooks for OpenHuman (issue #4395, follow-up to #3605).
;
; Part 2 of #4395 — installer pre-install process kill. The default tauri NSIS
; template only stops the *main* binary (OpenHuman.exe) via CheckIfAppIsRunning
; before overwriting it. A leftover / wedged `openhuman-core.exe` from the prior
; version (CLI/MCP/core mode) can still hold file handles under $INSTDIR and
; survive the update, which is exactly the "stale process blocks update" symptom
; #3605 tracks. Terminate those related processes before any file is copied so
; the new binary always lands cleanly.
;
; NSIS_HOOK_PREINSTALL runs before the template copies files, sets registry
; keys, and creates shortcuts (and before the built-in CheckIfAppIsRunning), so
; this is the correct place to guarantee no OpenHuman process is holding the
; install directory open.
;
; taskkill ships with every supported Windows version. /T also terminates child
; processes (CEF helpers); a non-zero exit (nothing matched) is fine and ignored.

!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Stopping any running OpenHuman helper processes before install..."
  ; Force-kill the embedded core / CLI / MCP process and its child tree.
  nsExec::Exec 'taskkill /F /IM openhuman-core.exe /T'
  Pop $0
  ; Belt-and-suspenders: the main binary and its CEF child processes. The
  ; template's CheckIfAppIsRunning also covers OpenHuman.exe, but in a silent
  ; auto-update relaunch we want the whole tree gone before files are replaced.
  nsExec::Exec 'taskkill /F /IM ${MAINBINARYNAME}.exe /T'
  Pop $0
!macroend
