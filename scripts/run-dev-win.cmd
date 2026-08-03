@echo off
setlocal EnableExtensions

rem ===========================================================================
rem Entry point for `pnpm dev:app:win`.
rem
rem Why this wrapper exists
rem -----------------------
rem pnpm runs package.json scripts through `cmd.exe /d /s /c <string>`. The /S
rem flag strips the first and last quote characters from <string> before
rem parsing it. A script body that quotes an interpreter path containing
rem spaces, e.g.
rem
rem     "C:/Program Files/Git/bin/bash.exe" ../scripts/run-dev-win.sh
rem
rem therefore reaches the parser as
rem
rem     C:/Program Files/Git/bin/bash.exe ../scripts/run-dev-win.sh
rem
rem and cmd treats `C:/Program` as the program name:
rem
rem     'C:/Program' is not recognized as an internal or external command
rem
rem That breaks every machine using the default Git for Windows install
rem location. Quoting behaves normally *inside* a .cmd file, so package.json
rem now points at this wrapper -- a relative path with no spaces, which needs
rem no quoting of its own -- and the spacey bash.exe path is quoted here.
rem
rem This mirrors the workaround run-dev-win.sh already applies internally when
rem it generates a .bat shim for cargo-tauri's beforeDevCommand.
rem ===========================================================================

set "SCRIPT_DIR=%~dp0"
set "BASH_EXE="

rem Escape hatch for non-standard Git installs / portable Git.
if defined OPENHUMAN_BASH_EXE (
  if exist "%OPENHUMAN_BASH_EXE%" set "BASH_EXE=%OPENHUMAN_BASH_EXE%"
)

rem Standard Git for Windows locations, including a user-scope install.
if not defined BASH_EXE call :use_if_exists "%ProgramFiles%\Git\bin\bash.exe"
if not defined BASH_EXE call :use_if_exists "%ProgramFiles(x86)%\Git\bin\bash.exe"
if not defined BASH_EXE call :use_if_exists "%LOCALAPPDATA%\Programs\Git\bin\bash.exe"

rem Derive the install root from git.exe on PATH, which covers winget, scoop
rem and chocolatey layouts. git.exe sits in <root>\cmd or <root>\bin, so
rem bash.exe is always at <root>\bin\bash.exe.
if not defined BASH_EXE (
  for /f "delims=" %%G in ('where git.exe 2^>nul') do (
    if not defined BASH_EXE call :use_if_exists "%%~dpG..\bin\bash.exe"
  )
)

rem NOTE: we deliberately never fall back to a bare `bash` lookup on PATH.
rem When WSL is enabled, `bash` resolves to C:\Windows\System32\bash.exe -- the
rem WSL launcher -- which would run run-dev-win.sh inside a Linux distro where
rem none of the Windows toolchain it configures (MSVC, CEF, cargo-tauri) exists.
if not defined BASH_EXE goto :no_bash

"%BASH_EXE%" "%SCRIPT_DIR%run-dev-win.sh" %*
exit /b %ERRORLEVEL%

rem Reported from a label rather than inside an `if (...)` block: the literal
rem parentheses below would otherwise close the block early.
:no_bash
echo [run-dev-win] Could not locate the bash.exe shipped with Git for Windows.>&2
echo [run-dev-win] Looked under Program Files\Git\bin, Program Files (x86)\Git\bin,>&2
echo [run-dev-win] LOCALAPPDATA\Programs\Git\bin, and the Git install root derived>&2
echo [run-dev-win] from git.exe on PATH.>&2
echo [run-dev-win] Install Git for Windows from https://git-scm.com/download/win,>&2
echo [run-dev-win] or set OPENHUMAN_BASH_EXE to the full path of bash.exe and retry.>&2
exit /b 1

:use_if_exists
if exist "%~1" set "BASH_EXE=%~1"
goto :eof
