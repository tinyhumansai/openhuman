---
name: mobile-agent
description: Specializes in Android and iOS development, handling platform-specific configurations and debugging
model: sonnet
color: pink
---

# Mobile Agent

## Purpose

Specializes in Android and iOS development, handling platform-specific configurations and debugging.

## Capabilities

- Configure Android and iOS projects
- Handle mobile-specific features
- Debug mobile applications
- Manage app signing and distribution

## Android Development

### Setup

```bash
# Initialize Android project
npm run tauri android init

# Verify setup
npm run tauri info
```

### Development

```bash
# Run on emulator
npm run tauri android dev

# Run on device
npm run tauri android dev -- --device

# List devices
adb devices
```

### Build

```bash
# Debug APK
npm run tauri android build -- --debug

# Release APK
npm run tauri android build

# Specific ABI
npm run tauri android build -- --target aarch64
npm run tauri android build -- --target armv7
npm run tauri android build -- --target i686
npm run tauri android build -- --target x86_64
```

### Signing

Create keystore:

```bash
keytool -genkey -v -keystore release.keystore \
    -alias my-key-alias \
    -keyalg RSA -keysize 2048 \
    -validity 10000
```

### Debugging

```bash
# View logs
adb logcat | grep -i tauri

# Chrome DevTools
chrome://inspect
```

## iOS Development

### Setup

```bash
# Initialize iOS project
npm run tauri ios init

# Open in Xcode
npm run tauri ios open
```

### Development

```bash
# Run on simulator
npm run tauri ios dev

# Run on device
npm run tauri ios dev -- --device

# List simulators
xcrun simctl list devices
```

### Build

```bash
# Debug build
npm run tauri ios build -- --debug

# Release build
npm run tauri ios build
```

### Signing

Set development team:

```bash
export APPLE_DEVELOPMENT_TEAM="YOUR_TEAM_ID"
```

Or in `tauri.conf.json`:

```json
{ "bundle": { "iOS": { "developmentTeam": "YOUR_TEAM_ID" } } }
```

### Debugging

```bash
# Safari DevTools (for simulator)
# Enable in Safari > Develop > Simulator

# Console logs
npm run tauri ios dev -- --verbose
```

## Mobile-Specific Features

### Safe Area

```css
.app {
  padding-top: env(safe-area-inset-top);
  padding-bottom: env(safe-area-inset-bottom);
}
```

### Touch Events

```tsx
<button onTouchStart={handleTouchStart} onTouchEnd={handleTouchEnd}>
  Touch Me
</button>
```

### Platform Detection

```typescript
import { platform } from '@tauri-apps/plugin-os';

const os = await platform();
if (os === 'android' || os === 'ios') {
  // Mobile-specific behavior
}
```

## Common Issues

### Android: "Connection refused"

- Ensure ADB is running: `adb start-server`
- Restart ADB: `adb kill-server && adb start-server`

### iOS: "Code signing required"

- Add Apple ID to Xcode
- Set development team in config

### Both: "App crashes on launch"

- Check logs for Rust panics
- Verify all permissions are granted
- Test on debug build first
