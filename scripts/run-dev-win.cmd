@echo off
setlocal

rem Change to the repository root so Git Bash receives a stable relative path.
cd /d "%~dp0.."

set "GIT_BASH="
if exist "%ProgramFiles%\Git\bin\bash.exe" set "GIT_BASH=%ProgramFiles%\Git\bin\bash.exe"
if not defined GIT_BASH if exist "%ProgramFiles(x86)%\Git\bin\bash.exe" set "GIT_BASH=%ProgramFiles(x86)%\Git\bin\bash.exe"
if not defined GIT_BASH if exist "%LOCALAPPDATA%\Programs\Git\bin\bash.exe" set "GIT_BASH=%LOCALAPPDATA%\Programs\Git\bin\bash.exe"

if not defined GIT_BASH (
  where bash >nul 2>nul
  if not errorlevel 1 (
    bash "scripts/run-dev-win.sh"
    exit /b %errorlevel%
  )

  echo [run-dev-win] Git Bash not found.
  echo [run-dev-win] Install Git for Windows or add bash.exe to PATH.
  exit /b 1
)

"%GIT_BASH%" "scripts/run-dev-win.sh"
exit /b %errorlevel%
