# คู่มือการติดตั้งและตั้งค่าบนอุปกรณ์ใหม่ (New Device Installation Guide)
## OpenHuman Thai Local Edition (Ollama + Obsidian + Thai UI)

เอกสารนี้จัดทำขึ้นเพื่อให้คุณหรือทีมงานสามารถนำโปรเจกต์นี้ไปติดตั้งบนอุปกรณ์เครื่องอื่น (New Machine / Device) ให้ได้สภาพแวดล้อม การตั้งค่า ภาษาไทย และการเชื่อมต่อที่เหมือนกันทุกประการ 100%

---

## 📋 1. รายการสิ่งที่ต้องติดตั้งล่วงหน้า (Prerequisites Checklist)

ก่อนเริ่มการติดตั้ง ให้ดาวน์โหลดและติดตั้งโปรแกรมพื้นฐานดังนี้:

1. **Git for Windows**: https://git-scm.com/download/win
2. **Node.js (v20+ หรือ v24 LTS)**: https://nodejs.org/
3. **pnpm (Package Manager)**:
   ```powershell
   npm install -g pnpm
   ```
4. **Ollama for Windows**: https://ollama.com/download
   * หลังติดตั้ง ให้ดาวน์โหลดโมเดล LLM ภาษาไทย:
     ```powershell
     ollama pull qwen3.5:4b
     ollama pull nomic-embed-text
     ```
5. **Obsidian**: https://obsidian.md/
6. **OpenHuman Desktop App (Installer)**:
   * ดาวน์โหลดและติดตั้ง OpenHuman.exe (ติดตั้งไว้ที่ %LOCALAPPDATA%\OpenHuman\OpenHuman.exe)

---

## 📂 2. การ Clone และโครงสร้างโฟลเดอร์

1. แนะนำให้ Clone หรือวางโฟลเดอร์โปรเจกต์ไว้ที่ `D:\openhuman-local` (หรือโฟลเดอร์ที่คุณต้องการ):
   ```powershell
   git clone <repository-url> D:\openhuman-local
   cd D:\openhuman-local
   ```
2. สร้างโฟลเดอร์ Obsidian Vault:
   ```powershell
   New-Item -ItemType Directory -Path "D:\openhuman-local\OpenHuman-Obsidian" -Force
   ```
3. เปิดโปรแกรม Obsidian แล้วเลือก **"Open folder as vault"** ไปที่ `D:\openhuman-local\OpenHuman-Obsidian`

---

## ⚙️ 3. การกำหนดค่า Environment Variables (.ENV)

### 3.1 ไฟล์ `.env` ที่ Root Folder (`D:\openhuman-local\.env`)
สร้างหรือตรวจสอบไฟล์ `D:\openhuman-local\.env`:

```env
OPENHUMAN_CORE_TOKEN=openhuman-local-token-12345
OPENHUMAN_CORE_PORT=7788
OPENHUMAN_CORE_REUSE_EXISTING=1
OPENHUMAN_ACTION_DIR=D:\openhuman-local\OpenHuman-Obsidian
OLLAMA_BASE_URL=http://localhost:11434
OLLAMA_DEFAULT_MODEL=qwen3.5:4b
OPENHUMAN_AUTONOMY_LEVEL=supervised
```

### 3.2 ไฟล์ `app/.env.local` สำหรับ Frontend (`D:\openhuman-local\app\.env.local`)
สร้างไฟล์ `app/.env.local`:

```env
VITE_CORE_RPC_URL=http://127.0.0.1:7788/rpc
VITE_DEFAULT_LOCALE=th
VITE_TELEMETRY_DISABLED=true
```

---

## 🤖 4. การตั้งค่า Agent Workspace & คำสั่งภาษาไทย

รันคำสั่ง PowerShell ต่อไปนี้เพื่อสร้างไฟล์คำสั่งภาษาไทยลงใน User Workspace:

```powershell
$workspaceDir = "$env:USERPROFILE\.openhuman\users\6a8a603d6118dae0642995eb\workspace"
New-Item -ItemType Directory -Path $workspaceDir -Force | Out-Null

@'
# User Profile
- **Preferred Language**: Thai (ภาษาไทย)
- **Primary Instruction**: You MUST ALWAYS communicate, converse, and reply in fluent and natural Thai (ภาษาไทย).
- **Communication Style**: เป็นมิตร สุภาพ กระชับ ชัดเจน และเป็นธรรมชาติ
'@ | Out-File -FilePath "$workspaceDir\USER.md" -Encoding utf8

@'
# Global Agent Instructions
## Language
- Always reply in Thai (ภาษาไทย).
- All explanations, summaries, and chat responses must be written in clear, natural Thai.
- You may use English for technical terms, code snippets, or file paths where appropriate, but the conversation and surrounding text must be Thai.
'@ | Out-File -FilePath "$workspaceDir\AGENTS.md" -Encoding utf8

@'
# Writing style
Reply like you're texting a friend: casual, lowercase-ok, natural. Lead with the answer, then whatever context actually helps.
Language rule: Always respond in fluent, natural Thai (ภาษาไทย).
Two hard rules: no em-dashes (—), use commas or short sentences instead. Don't repeat yourself.
'@ | Out-File -FilePath "$workspaceDir\STYLE.md" -Encoding utf8
```

---

## 📦 5. การติดตั้ง Dependencies และคอมไพล์ระบบ

รันคำสั่งต่อไปนี้ที่ Root Directory (`D:\openhuman-local`):

```powershell
# 1. ติดตั้งแพ็กเกจ Node.js
pnpm install

# 2. คอมไพล์ Production Bundle
pnpm build
```

---

## 🚀 6. การสร้าง One-Click Launcher & Desktop Shortcut

สร้างไฟล์ `D:\openhuman-local\Start-OpenHuman-Thai.bat`:

```bat
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
    if exist "%LOCALAPPDATA%\OpenHuman\OpenHuman.exe" (
        echo [*] กำลังเปิด Core Service (พอร์ต 7788)...
        set OPENHUMAN_CORE_TOKEN=openhuman-local-token-12345
        start /b "" "%LOCALAPPDATA%\OpenHuman\OpenHuman.exe" core run --port 7788 >nul 2>&1
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
```

สร้าง Shortcut บน Desktop:

```powershell
$WshShell = New-Object -ComObject WScript.Shell
$Shortcut = $WshShell.CreateShortcut("$env:USERPROFILE\Desktop\OpenHuman (Thai).lnk")
$Shortcut.TargetPath = "D:\openhuman-local\Start-OpenHuman-Thai.bat"
$Shortcut.WorkingDirectory = "D:\openhuman-local"
$Shortcut.IconLocation = "$env:LOCALAPPDATA\OpenHuman\OpenHuman.exe,0"
$Shortcut.Save()
```

---

## 🔑 7. ข้อมูลเชื่อมต่อครั้งแรก (First-Time BootCheck Connection)

เมื่อหน้าจอแอปแสดงหน้าต่างเชื่อมต่อ Core:
* **Runtime URL**: `http://127.0.0.1:7788/rpc`
* **Auth Token**: `openhuman-local-token-12345`
* กดปุ่ม **"Test Connection"** (ขึ้นสีเขียว) ➔ กด **"Continue"**
