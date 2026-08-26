@echo off
setlocal

rem Change to the repository root so Git Bash receives a stable relative path.
cd /d "%~dp0.."

set "GIT_BASH="
if exist "%ProgramFiles%\Git\bin\bash.exe" set "GIT_BASH=%ProgramFiles%\Git\bin\bash.exe"
if not defined GIT_BASH if exist "%ProgramFiles(x86)%\Git\bin\bash.exe" set "GIT_BASH=%ProgramFiles(x86)%\Git\bin\bash.exe"
if not defined GIT_BASH if exist "%LOCALAPPDATA%\Programs\Git\bin\bash.exe" set "GIT_BASH=%LOCALAPPDATA%\Programs\Git\bin\bash.exe"

if defined GIT_BASH goto :found

rem Resolve bash from PATH, but only accept a Git for Windows bash.exe.
set "BASH_PATH="
for /f "delims=" %%i in ('where bash 2^>nul') do if not defined BASH_PATH set "BASH_PATH=%%i"

if not defined BASH_PATH goto :notfound
if /i not "%BASH_PATH:~-4%"==".exe" goto :notfound
if not exist "%BASH_PATH%" goto :notfound

for %%i in ("%BASH_PATH%") do set "BASH_DIR=%%~dpi"

if exist "%BASH_DIR%..\cmd\git.exe" set "GIT_BASH=%BASH_PATH%"
if not defined GIT_BASH if exist "%BASH_DIR%..\..\cmd\git.exe" set "GIT_BASH=%BASH_PATH%"
if not defined GIT_BASH if exist "%BASH_DIR%..\..\mingw64\bin\git.exe" set "GIT_BASH=%BASH_PATH%"

if not defined GIT_BASH goto :notfound
goto :found

:notfound
echo [run-dev-win] Git Bash not found.
echo [run-dev-win] Install Git for Windows or add bash.exe to PATH.
exit /b 1

:found
"%GIT_BASH%" "scripts/run-dev-win.sh"
exit /b %errorlevel%
