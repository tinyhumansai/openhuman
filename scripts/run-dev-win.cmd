@echo off
setlocal

rem Change to the repository root so Git Bash receives a stable relative path.
cd /d "%~dp0.."

set "GIT_BASH="
if exist "%ProgramFiles%\Git\bin\bash.exe" set "GIT_BASH=%ProgramFiles%\Git\bin\bash.exe"
if not defined GIT_BASH if exist "%ProgramFiles(x86)%\Git\bin\bash.exe" set "GIT_BASH=%ProgramFiles(x86)%\Git\bin\bash.exe"
if not defined GIT_BASH if exist "%LOCALAPPDATA%\Programs\Git\bin\bash.exe" set "GIT_BASH=%LOCALAPPDATA%\Programs\Git\bin\bash.exe"

if defined GIT_BASH goto :found

rem Scan every where bash result and accept the first Git for Windows bash.exe.
for /f "delims=" %%i in ('where bash 2^>nul') do (
  if /i "%%~xi"==".exe" if exist "%%i" (
    if exist "%%~dpi..\cmd\git.exe" set "GIT_BASH=%%i" & goto :found
    if exist "%%~dpi..\..\cmd\git.exe" set "GIT_BASH=%%i" & goto :found
    if exist "%%~dpi..\..\mingw64\bin\git.exe" set "GIT_BASH=%%i" & goto :found
  )
)

goto :notfound

:notfound
echo [run-dev-win] Git Bash not found.
echo [run-dev-win] Install Git for Windows or add bash.exe to PATH.
exit /b 1

:found
"%GIT_BASH%" "scripts/run-dev-win.sh"
exit /b %errorlevel%
