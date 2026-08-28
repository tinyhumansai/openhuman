@echo off
chcp 65001 >nul
title OpenHuman Thai Launcher
echo ========================================================
echo        🚀 กำลังเริ่มการทำงาน OpenHuman (ภาษาไทย)
echo ========================================================
echo.

cd /d "D:\openhuman-local"

echo [1/3] ตรวจสอบสถานะ Ollama Server...
curl -s http://localhost:11434/api/tags >nul 2>&1
if %errorlevel% neq 0 (
    echo [*] กำลังเริ่มทำงาน Ollama...
    start /b "" ollama serve >nul 2>&1
    timeout /t 2 /nobreak >nul
)
echo [OK] Ollama พร้อมทำงาน (qwen3.5:4b)

echo.
echo [2/3] ตรวจสอบ OpenHuman Core Service...
curl -s http://127.0.0.1:7788/rpc >nul 2>&1
if %errorlevel% neq 0 (
    if exist "C:\Users\PC-IT-Promax\AppData\Local\OpenHuman\OpenHuman.exe" (
        echo [*] กำลังเปิด Core Service (พอร์ต 7788)...
        set OPENHUMAN_CORE_TOKEN=openhuman-local-token-12345
        start /b "" "C:\Users\PC-IT-Promax\AppData\Local\OpenHuman\OpenHuman.exe" core run --port 7788 >nul 2>&1
        timeout /t 3 /nobreak >nul
    )
)
echo [OK] Core Service พร้อมทำงาน

echo.
echo [3/3] กำลังเปิดหน้าต่าง OpenHuman ภาษาไทย (Desktop App)...
start /b "" cmd /c "pnpm --filter openhuman-app dev --port 5173" >nul 2>&1
timeout /t 2 /nobreak >nul

if exist "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" (
    start "" "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" --app=http://localhost:5173 --window-size=1280,900
) else if exist "C:\Program Files\Google\Chrome\Application\chrome.exe" (
    start "" "C:\Program Files\Google\Chrome\Application\chrome.exe" --app=http://localhost:5173 --window-size=1280,900
) else (
    start http://localhost:5173
)

echo.
echo ========================================================
echo   ✅ เปิด OpenHuman ภาษาไทย เรียบร้อยแล้ว!
echo ========================================================
timeout /t 3 >nul
exit
