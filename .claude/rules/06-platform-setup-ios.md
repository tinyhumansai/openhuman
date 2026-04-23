---
paths:
  - "src-tauri/gen/apple/**"
  - "app/src-tauri/gen/apple/**"
---

# iOS Platform Setup

## Prerequisites

### 1. macOS

iOS development requires a Mac with macOS.

### 2. Xcode

Install from the Mac App Store (requires latest version for latest iOS SDKs).

### 3. Xcode Command Line Tools

```bash
xcode-select --install
```

### 4. Additional Tools

```bash
brew install xcodegen
brew install libimobiledevice
brew install ios-deploy
```

### 5. CocoaPods

```bash
sudo gem install cocoapods
```

### 6. Rust iOS Targets

```bash
rustup target add aarch64-apple-ios
rustup target add aarch64-apple-ios-sim
rustup target add x86_64-apple-ios
```

## Initialize iOS Project

```bash
npm run tauri ios init
```

This creates the iOS project in `src-tauri/gen/apple/`.

## Apple Developer Account

### For Development

- Free Apple ID allows testing on your own devices
- Must register device UDID in Xcode

### For Distribution

- Requires paid Apple Developer Program ($99/year)
- Needed for App Store, TestFlight, or Ad Hoc distribution

## Development Team Configuration

Set in `tauri.conf.json`:

```json
{ "bundle": { "iOS": { "developmentTeam": "YOUR_TEAM_ID" } } }
```

Or via environment variable:

```bash
export APPLE_DEVELOPMENT_TEAM="YOUR_TEAM_ID"
```

## Development

### Using Simulator

```bash
# List available simulators
xcrun simctl list devices

# Run on simulator
npm run tauri ios dev
```

### Using Physical Device

1. Connect device via USB
2. Trust the computer on the device
3. Run: `npm run tauri ios dev -- --device`

## Building for iOS

```bash
# Debug build
npm run tauri ios build -- --debug

# Release build
npm run tauri ios build
```

## Output Files

```
src-tauri/gen/apple/build/
└── arm64/
    └── tauri-app.app
```

## Code Signing

### Automatic Signing

Xcode can manage signing automatically when configured with your Apple ID.

### Manual Signing

1. Create provisioning profile in Apple Developer Portal
2. Download and install in Xcode
3. Configure in Xcode project settings

## App Store Submission

1. Build release version
2. Archive in Xcode
3. Upload via Xcode Organizer or Transporter
4. Complete submission in App Store Connect
