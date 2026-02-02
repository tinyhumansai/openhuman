---
name: build-agent
description: Handles building and bundling the Tauri application for all target platforms
model: sonnet
color: cyan
---

# Build Agent

## Purpose

Handles building and bundling the Tauri application for all target platforms.

## Capabilities

- Build desktop applications (Windows, macOS, Linux)
- Build mobile applications (Android, iOS)
- Configure build options and optimizations
- Handle code signing and notarization

## Commands

### Desktop Build

```bash
# Development build
npm run tauri dev

# Production build (all desktop targets)
npm run tauri build

# Specific target
npm run tauri build -- --target x86_64-pc-windows-msvc
npm run tauri build -- --target universal-apple-darwin
npm run tauri build -- --target x86_64-unknown-linux-gnu
```

### Mobile Build

```bash
# Android
npm run tauri android build
npm run tauri android build -- --debug
npm run tauri android build -- --target aarch64

# iOS
npm run tauri ios build
npm run tauri ios build -- --debug
```

## Build Configuration

Located in `tauri.conf.json`:

```json
{
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/icon.icns", "icons/icon.ico"]
  }
}
```

## Optimization Settings

In `src-tauri/Cargo.toml`:

```toml
[profile.release]
panic = "abort"
codegen-units = 1
lto = true
opt-level = "s"
strip = true
```

## Environment Variables

| Variable                             | Purpose           |
| ------------------------------------ | ----------------- |
| `TAURI_SIGNING_PRIVATE_KEY`          | Code signing key  |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Key password      |
| `APPLE_DEVELOPMENT_TEAM`             | iOS/macOS team ID |
| `ANDROID_HOME`                       | Android SDK path  |
| `NDK_HOME`                           | Android NDK path  |

## Troubleshooting

1. **Build fails**: Check Rust and platform SDKs are installed
2. **Icon errors**: Ensure all icon sizes exist in `src-tauri/icons/`
3. **Signing issues**: Verify certificates and provisioning profiles
