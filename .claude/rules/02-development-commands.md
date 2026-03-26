# Development Commands

## Prerequisites

Before running any commands, ensure you have:

1. **Node.js** (v22+) and npm installed
2. **Rust** installed via rustup
3. **Platform-specific SDKs** (see platform setup guides)

## Common Commands

### Desktop Development

```bash
# Frontend dev server only (port 1420)
yarn dev

# Desktop dev with hot-reload (starts Vite + Tauri)
yarn tauri dev

# Production build (TypeScript compile + Vite build + Tauri bundle)
yarn tauri build

# Debug build with .app bundle (required for deep link testing on macOS)
yarn tauri build --debug --bundles app
yarn macos:dev
```

### Android Development

```bash
# Initialize Android project (first time)
yarn tauri android init

# Start Android development
yarn tauri android dev

# Build Android APK/AAB
yarn tauri android build
```

### iOS Development

```bash
# Initialize iOS project (first time)
yarn tauri ios init

# Start iOS development
yarn tauri ios dev

# Build iOS app
yarn tauri ios build
```

### Frontend Only

```bash
# Install dependencies
yarn install

# Start Vite dev server only
yarn dev

# Build frontend only
yarn build

# Preview production build
yarn preview
```

### Code Quality

```bash
# Run linting and formatting checks
yarn lint
yarn format

# TypeScript compilation check
yarn typecheck

# Rust checks
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
```

## Stopping Development Servers

### Desktop (Tauri Dev)

```bash
# In the terminal running `yarn tauri dev`:
Ctrl + C

# If process is stuck, force kill:
# macOS/Linux:
pkill -f "tauri dev"
pkill -f "cargo-tauri"
lsof -ti:1420 | xargs kill -9   # Kill process on Vite port

# Windows (PowerShell):
Stop-Process -Name "tauri-app" -Force
Get-Process | Where-Object {$_.ProcessName -like "*tauri*"} | Stop-Process -Force
netstat -ano | findstr :1420    # Find PID on port 1420
taskkill /PID <PID> /F          # Kill by PID
```

### Android Dev Server

```bash
# In the terminal running `yarn tauri android dev`:
Ctrl + C

# If emulator/device is stuck:
adb kill-server                  # Stop ADB server
adb start-server                 # Restart ADB server

# Force stop app on device:
adb shell am force-stop com.openhuman.app

# Kill Gradle daemon if stuck:
# macOS/Linux:
pkill -f "gradle"
./gradlew --stop                 # From src-tauri/gen/android/

# Windows:
taskkill /F /IM java.exe         # Kills Gradle processes
```

### iOS Dev Server

```bash
# In the terminal running `yarn tauri ios dev`:
Ctrl + C

# If simulator is stuck:
xcrun simctl shutdown all        # Shutdown all simulators
xcrun simctl erase all           # Reset all simulators (clears data)

# Kill specific simulator:
xcrun simctl shutdown booted     # Shutdown currently running simulator

# Force kill Xcode processes:
pkill -f "Simulator"
pkill -f "xcodebuild"

# If build process hangs:
pkill -f "cargo"
```

### Frontend Only (Vite)

```bash
# In the terminal running `yarn dev`:
Ctrl + C

# If port 1420 is still occupied:
# macOS/Linux:
lsof -ti:1420 | xargs kill -9

# Windows:
netstat -ano | findstr :1420
taskkill /PID <PID> /F
```

### Kill All Development Processes

```bash
# macOS/Linux - Nuclear option (kills all related processes):
pkill -f "tauri"
pkill -f "vite"
pkill -f "cargo"
pkill -f "node.*tauri"

# Windows (PowerShell):
Get-Process | Where-Object {$_.ProcessName -match "tauri|vite|cargo|node"} | Stop-Process -Force
```

## Build Targets

| Platform | Command                    | Output              |
| -------- | -------------------------- | ------------------- |
| Windows  | `yarn tauri build`         | `.msi`, `.exe`      |
| macOS    | `yarn tauri build`         | `.dmg`, `.app`      |
| Linux    | `yarn tauri build`         | `.deb`, `.AppImage` |
| Android  | `yarn tauri android build` | `.apk`, `.aab`      |
| iOS      | `yarn tauri ios build`     | `.ipa`              |
