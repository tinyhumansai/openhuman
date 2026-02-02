# Troubleshooting Guide

## Common Issues

### Build Errors

#### "error: failed to run custom build command for `tauri`"

**Cause**: Missing system dependencies

**Solution**:

- macOS: `xcode-select --install`
- Windows: Install Visual Studio Build Tools
- Linux: `sudo apt install libwebkit2gtk-4.1-dev build-essential libssl-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`

#### "cargo: command not found"

**Cause**: Rust not installed or not in PATH

**Solution**:

```bash
source "$HOME/.cargo/env"
# Or reinstall Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Development Server Issues

#### Frontend loads but shows blank page

**Cause**: Dev server not running or wrong port

**Solution**:

1. Check `devUrl` in `tauri.conf.json` matches Vite port
2. Start frontend first: `npm run dev`
3. Then: `npm run tauri dev`

#### Hot reload not working

**Cause**: File watcher issues

**Solution**:

- macOS: Increase file descriptor limit
- Windows: Disable antivirus scanning on project folder
- All: Restart dev server

### Android Issues

#### "ANDROID_HOME not set"

**Solution**:

```bash
export ANDROID_HOME="$HOME/Library/Android/sdk"
export PATH="$PATH:$ANDROID_HOME/platform-tools"
```

Add to `~/.zshrc` or `~/.bashrc` for persistence.

#### "No connected devices"

**Solution**:

1. Enable Developer Options on device
2. Enable USB Debugging
3. Accept RSA key prompt on device
4. Run: `adb devices` to verify connection

#### "NDK not found"

**Solution**:

1. Open Android Studio
2. Settings > Languages & Frameworks > Android SDK
3. SDK Tools tab > Check "NDK (Side by side)"
4. Install latest NDK

### iOS Issues

#### "No code signing certificates found"

**Solution**:

1. Open Xcode
2. Preferences > Accounts > Add Apple ID
3. Let Xcode manage signing automatically
4. Or set `APPLE_DEVELOPMENT_TEAM` environment variable

#### "Unable to install app on device"

**Solution**:

1. Device must be registered in your developer account
2. Create provisioning profile including the device
3. Trust developer in Settings > General > Device Management

#### Simulator not launching

**Solution**:

```bash
# Reset simulator
xcrun simctl erase all

# Or boot specific simulator
xcrun simctl boot "iPhone 15"
```

### Performance Issues

#### App runs slowly

**Solutions**:

1. Enable release mode: `npm run tauri build`
2. Check for unnecessary re-renders in React
3. Profile Rust code with `cargo flamegraph`

#### Large bundle size

**Solutions**:

1. Enable stripping in `Cargo.toml`:
   ```toml
   [profile.release]
   strip = true
   lto = true
   ```
2. Analyze frontend bundle: `npm run build -- --analyze`
3. Remove unused dependencies

### Plugin Issues

#### "Plugin not initialized"

**Cause**: Plugin not added to Tauri builder

**Solution**:
Check `src-tauri/src/lib.rs`:

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_fs::init())  // Add plugin here
    .plugin(tauri_plugin_dialog::init())
```

#### "Permission denied" for plugin

**Cause**: Missing capability

**Solution**:
Add permission to `src-tauri/capabilities/default.json`:

```json
{ "permissions": ["fs:default", "fs:allow-read-text-file"] }
```

## Getting Help

1. **Tauri Discord**: https://discord.gg/tauri
2. **GitHub Issues**: https://github.com/tauri-apps/tauri/issues
3. **Documentation**: https://tauri.app/
4. **Stack Overflow**: Tag with `tauri`
