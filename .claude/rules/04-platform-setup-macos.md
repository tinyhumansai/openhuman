# macOS Platform Setup

## Prerequisites

### 1. Xcode

Install from the Mac App Store or:

```bash
xcode-select --install
```

### 2. Xcode Command Line Tools

```bash
xcode-select --install
```

### 3. Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 4. Additional Dependencies (via Homebrew)

For iOS development:

```bash
brew install xcodegen
brew install libimobiledevice
```

## Building for macOS

```bash
# Build for current architecture
npm run tauri build

# Build universal binary (Intel + Apple Silicon)
npm run tauri build -- --target universal-apple-darwin
```

## Output Files

After building, find installers in:

```
src-tauri/target/release/bundle/
├── macos/
│   └── tauri-app.app
└── dmg/
    └── tauri-app_0.1.0_x64.dmg
```

## Code Signing

### Development

For local development, no code signing is required.

### Distribution

Set up code signing for App Store or notarization:

1. Enroll in Apple Developer Program
2. Create signing certificates
3. Configure in `tauri.conf.json`:

```json
{ "bundle": { "macOS": { "signingIdentity": "Developer ID Application: Your Name (TEAM_ID)" } } }
```

## Notarization

For distribution outside the App Store:

```bash
# Build and notarize
npm run tauri build -- --target universal-apple-darwin

# Or use xcrun for manual notarization
xcrun notarytool submit ./path/to/app.dmg --apple-id "your@email.com" --team-id "TEAM_ID"
```
