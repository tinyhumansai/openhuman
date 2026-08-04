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
rem
rem Style constraint: no labels, no jumps, no multi-line parenthesised blocks
rem ------------------------------------------------------------------------
rem cmd.exe locates a label by byte offset and re-reads the script in 512-byte
rem chunks, which makes label-based control flow the one batch construct whose
rem behaviour depends on the file's line endings. This script is therefore
rem written as straight-line batch guarded by single-line `if` statements, so
rem it behaves identically whether it is checked out with LF or CRLF. The
rem .gitattributes rule that checks batch files out as CRLF stays for editor
rem and tooling consistency -- correctness simply no longer depends on it.
rem ===========================================================================

set "SCRIPT_DIR=%~dp0"
set "BASH_EXE="

rem Escape hatch for non-standard Git installs / portable Git.
if defined OPENHUMAN_BASH_EXE if exist "%OPENHUMAN_BASH_EXE%" set "BASH_EXE=%OPENHUMAN_BASH_EXE%"

rem Standard Git for Windows locations, including a user-scope install.
if not defined BASH_EXE if exist "%ProgramFiles%\Git\bin\bash.exe" set "BASH_EXE=%ProgramFiles%\Git\bin\bash.exe"
if not defined BASH_EXE if exist "%ProgramFiles(x86)%\Git\bin\bash.exe" set "BASH_EXE=%ProgramFiles(x86)%\Git\bin\bash.exe"
if not defined BASH_EXE if exist "%LOCALAPPDATA%\Programs\Git\bin\bash.exe" set "BASH_EXE=%LOCALAPPDATA%\Programs\Git\bin\bash.exe"

rem Scoop only ever puts shims on PATH, so the git.exe probe below would derive
rem <scoop>\bin\bash.exe, which does not exist. The real binary always sits
rem under apps\git\current\bin, via the `current` junction scoop maintains
rem across upgrades. Probe the user, relocated and global scoop roots.
if not defined BASH_EXE if exist "%SCOOP%\apps\git\current\bin\bash.exe" set "BASH_EXE=%SCOOP%\apps\git\current\bin\bash.exe"
if not defined BASH_EXE if exist "%USERPROFILE%\scoop\apps\git\current\bin\bash.exe" set "BASH_EXE=%USERPROFILE%\scoop\apps\git\current\bin\bash.exe"
if not defined BASH_EXE if exist "%ProgramData%\scoop\apps\git\current\bin\bash.exe" set "BASH_EXE=%ProgramData%\scoop\apps\git\current\bin\bash.exe"

rem Derive the install root from git.exe on PATH, which covers the winget and
rem chocolatey layouts. git.exe sits in <root>\cmd or <root>\bin, so bash.exe
rem is always at <root>\bin\bash.exe.
if not defined BASH_EXE for /f "delims=" %%G in ('where git.exe 2^>nul') do if not defined BASH_EXE if exist "%%~dpG..\bin\bash.exe" set "BASH_EXE=%%~dpG..\bin\bash.exe"

rem NOTE: we deliberately never fall back to a bare `bash` lookup on PATH.
rem When WSL is enabled, `bash` resolves to C:\Windows\System32\bash.exe -- the
rem WSL launcher -- which would run run-dev-win.sh inside a Linux distro where
rem none of the Windows toolchain it configures - MSVC, CEF, cargo-tauri -
rem exists.

if not defined BASH_EXE echo [run-dev-win] Could not locate the bash.exe shipped with Git for Windows.>&2
if not defined BASH_EXE echo [run-dev-win] Looked under the machine-scope and user-scope Git for Windows>&2
if not defined BASH_EXE echo [run-dev-win] install directories, the scoop apps\git\current\bin directories,>&2
if not defined BASH_EXE echo [run-dev-win] and the Git install root derived from git.exe on PATH.>&2
if not defined BASH_EXE echo [run-dev-win] Install Git for Windows from https://git-scm.com/download/win,>&2
if not defined BASH_EXE echo [run-dev-win] or set OPENHUMAN_BASH_EXE to the full path of bash.exe and retry.>&2
if not defined BASH_EXE exit /b 1

"%BASH_EXE%" "%SCRIPT_DIR%run-dev-win.sh" %*
exit /b %ERRORLEVEL%
