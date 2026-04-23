---
paths:
  - "app/src-tauri/**"
  - "src-tauri/**"
---

# Windows Platform Setup

## Prerequisites

### 1. Microsoft Visual Studio C++ Build Tools

Download and install from: https://visualstudio.microsoft.com/visual-cpp-build-tools/

During installation, select:

- "Desktop development with C++"
- Windows 10/11 SDK
- MSVC v143+ build tools

### 2. WebView2

Windows 10 (1803+) and Windows 11 include WebView2 by default.

For older systems, download from: https://developer.microsoft.com/microsoft-edge/webview2/

### 3. Rust

Install via rustup:

```powershell
winget install Rustlang.Rustup
```

Or download from: https://rustup.rs

## Building for Windows

```bash
# Build for current architecture
npm run tauri build

# Build for specific target
npm run tauri build -- --target x86_64-pc-windows-msvc
npm run tauri build -- --target aarch64-pc-windows-msvc
```

## Output Files

After building, find installers in:

```
src-tauri/target/release/bundle/
├── msi/
│   └── tauri-app_0.1.0_x64_en-US.msi
└── nsis/
    └── tauri-app_0.1.0_x64-setup.exe
```

## Troubleshooting

### Missing Visual C++ Redistributable

If users report missing DLLs, bundle the Visual C++ Redistributable or instruct users to install it.

### WebView2 Issues

For enterprise environments, WebView2 fixed version runtime can be bundled with the app.
