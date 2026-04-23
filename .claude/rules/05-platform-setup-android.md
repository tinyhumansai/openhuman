---
paths:
  - "src-tauri/gen/android/**"
  - "app/src-tauri/gen/android/**"
---

# Android Platform Setup

## Prerequisites

### 1. Android Studio

Download and install from: https://developer.android.com/studio

### 2. Android SDK

After installing Android Studio:

1. Open Android Studio
2. Go to **Settings > Languages & Frameworks > Android SDK**
3. Install:
   - Android SDK Platform 34 (or latest)
   - Android SDK Build-Tools
   - Android SDK Platform-Tools
   - NDK (Side by side)

### 3. Environment Variables

Add to your shell profile (`~/.zshrc` or `~/.bashrc`):

```bash
export ANDROID_HOME="$HOME/Library/Android/sdk"
export NDK_HOME="$ANDROID_HOME/ndk/$(ls -1 $ANDROID_HOME/ndk | tail -n 1)"
export PATH="$PATH:$ANDROID_HOME/platform-tools"
export PATH="$PATH:$ANDROID_HOME/tools/bin"
```

### 4. Rust Android Targets

```bash
rustup target add aarch64-linux-android
rustup target add armv7-linux-androideabi
rustup target add i686-linux-android
rustup target add x86_64-linux-android
```

## Initialize Android Project

```bash
npm run tauri android init
```

This creates the Android project in `src-tauri/gen/android/`.

## Development

### Using Emulator

1. Open Android Studio
2. Create AVD (Android Virtual Device)
3. Start the emulator
4. Run: `npm run tauri android dev`

### Using Physical Device

1. Enable Developer Options on device
2. Enable USB Debugging
3. Connect device via USB
4. Run: `npm run tauri android dev`

## Building for Android

```bash
# Debug build
npm run tauri android build -- --debug

# Release build
npm run tauri android build
```

## Output Files

```
src-tauri/gen/android/app/build/outputs/
├── apk/
│   ├── debug/
│   │   └── app-debug.apk
│   └── release/
│       └── app-release-unsigned.apk
└── bundle/
    └── release/
        └── app-release.aab
```

## Signing for Release

### Create Keystore

```bash
keytool -genkey -v -keystore release.keystore -alias my-key-alias -keyalg RSA -keysize 2048 -validity 10000
```

### Configure Signing

Edit `src-tauri/gen/android/app/build.gradle.kts`:

```kotlin
android {
    signingConfigs {
        create("release") {
            storeFile = file("path/to/release.keystore")
            storePassword = System.getenv("KEYSTORE_PASSWORD")
            keyAlias = "my-key-alias"
            keyPassword = System.getenv("KEY_PASSWORD")
        }
    }
    buildTypes {
        getByName("release") {
            signingConfig = signingConfigs.getByName("release")
        }
    }
}
```
