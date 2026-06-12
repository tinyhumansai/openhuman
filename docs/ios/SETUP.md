# iOS Client Setup

This document covers everything a developer needs to build, run, and test the OpenHuman iOS client.

---

## Prerequisites

- macOS 14+ with Xcode 15.4+
- iOS 17+ physical device or simulator
- Rust toolchain with `aarch64-apple-ios` target
- pnpm (version pinned in root `package.json`)
- Apple Developer account with a provisioning profile

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

---

## Initial setup

Run the helper script from the repo root. It calls `tauri ios init` with the correct working directory and prints next steps.

```bash
bash scripts/ios-init.sh
```

`tauri ios init` scaffolds `app/src-tauri-mobile/gen/apple/`. That directory is **gitignored** (it contains bundle-identifier-specific Xcode project files that differ per developer account).

### Info.plist privacy keys

`tauri ios init` creates a generated `Info.plist` at:

```
app/src-tauri-mobile/gen/apple/<bundle-id>_iOS/Info.plist
```

`scripts/ios-init.sh` injects these privacy keys into the generated plist:

```xml
<key>NSCameraUsageDescription</key>
<string>OpenHuman uses the camera to scan the pairing QR code from your desktop.</string>

<key>NSMicrophoneUsageDescription</key>
<string>OpenHuman uses the microphone for push-to-talk voice messages.</string>

<key>NSSpeechRecognitionUsageDescription</key>
<string>OpenHuman uses on-device speech recognition to transcribe your voice messages.</string>
```

---

## Development workflow

```bash
# Start the iOS dev build (hot-reload via Vite, deployed to simulator or device):
pnpm tauri:ios:dev

# From the repo root:
pnpm tauri:ios:dev
```

The `tauri:ios:dev` script uses `@tauri-apps/cli@^2` directly (via `npx --package`), **not** the vendored CEF-aware CLI. The CEF CLI is only needed for the desktop build.

Set your development team in Xcode (generated project > Signing & Capabilities) before deploying to a physical device.

---

## Production build

```bash
pnpm tauri:ios:build
# or from repo root:
pnpm tauri:ios:build
```

---

## App Store Connect delivery

`.github/workflows/ios-appstore.yml` builds a signed `iphoneos` archive, exports an IPA, uploads the IPA to App Store Connect/TestFlight with `altool`, and stores the IPA + dSYMs as GitHub Actions artifacts.

Run it from GitHub Actions > iOS App Store. Inputs:

- `ref` -- optional git ref to build.
- `build_number` -- optional `CFBundleVersion`; defaults to `github.run_number`.
- `upload_to_app_store_connect` -- set `false` for a signed archive/export dry run.

Required GitHub environment: `App-Store`.

Required secrets:

- `APPLE_TEAM_ID` -- Apple Developer Team ID.
- `IOS_KEYCHAIN_PASSWORD` -- temporary CI keychain password.
- `IOS_DISTRIBUTION_CERTIFICATE_BASE64` -- base64-encoded `.p12` Apple Distribution certificate.
- `IOS_DISTRIBUTION_CERTIFICATE_PASSWORD` -- password for that `.p12`.
- `IOS_APPSTORE_PROVISIONING_PROFILE_BASE64` -- base64-encoded App Store provisioning profile for `com.tinyhumansai.openhuman`.
- `APP_STORE_CONNECT_API_KEY_ID` -- App Store Connect API key ID.
- `APP_STORE_CONNECT_ISSUER_ID` -- App Store Connect issuer ID.
- `APP_STORE_CONNECT_API_PRIVATE_KEY_BASE64` -- base64-encoded `AuthKey_<key id>.p8`.

Local encoding helpers:

```bash
base64 -i ios_distribution.p12 | pbcopy
base64 -i OpenHuman_AppStore.mobileprovision | pbcopy
base64 -i AuthKey_XXXXXXXXXX.p8 | pbcopy
```

The workflow uploads a build to App Store Connect. It does not submit the build for App Review; that remains a deliberate App Store Connect action.

### Local upload script

After downloading an App Store provisioning profile and App Store Connect API key, you can build/export/upload from this Mac:

```bash
TEAM_ID=XXXXXXXXXX \
IOS_APPSTORE_PROVISIONING_PROFILE_PATH=/path/to/OpenHuman_AppStore.mobileprovision \
ASC_KEY_ID=XXXXXXXXXX \
ASC_ISSUER_ID=xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx \
ASC_KEY_PATH=/path/to/AuthKey_XXXXXXXXXX.p8 \
UPLOAD=1 \
scripts/ios-appstore-upload.sh
```

Use `UPLOAD=0` to stop after IPA export.

### Updating without a new App Store build

iOS cannot self-update native code or replace the installed app binary outside App Store/TestFlight distribution. For OpenHuman, this means changes to Rust, Tauri plugins, native permissions, bundled frontend code, or the app shell need a new reviewed build.

Safe server-side updates include model/provider configuration, feature flags, prompt/content changes, remote data, and backend behavior that the shipped client already knows how to render. Be conservative with remote JavaScript or plugin-style features: Apple allows some software/content delivered outside the binary under specific rules, but it must stay within App Review limits and must not expose native platform APIs without permission.

---

## Pairing flow

```
Desktop                              iOS
  |                                    |
  |-- Settings > Devices > "Pair"      |
  |-- devices_create_pairing RPC       |
  |   (backend issues channelId,       |
  |    pairingToken, sessionToken)      |
  |-- QR shown                         |
  |                    scan QR --------|
  |                    (extract cid,   |
  |                     pt, cpk, rpc?) |
  |                    iOS connects    |
  |                    to backend      |
  |                    tunnel:connect  |
  |                    (role:client,   |
  |                     channelId,     |
  |                     pairingToken)  |
  |                    backend returns |
  |                    iOS sessionToken|
  |                    X25519 handshake|
  |                    over tunnel     |
  |<-- DevicePaired event              |
  |-- device appears in Devices list   |
```

Transport selection (handled by `TransportManager`):

1. LAN HTTP -- fast, zero-latency, requires same network.
2. Socket.io tunnel -- E2E encrypted via XChaCha20-Poly1305 over X25519 key agreement.
3. Cloud HTTP -- fallback when LAN and tunnel are unreachable.

---

## Security notes

- The tunnel backend is a **blind forwarder**. It never sees plaintext payloads.
- `pairingToken` is single-use and hashed at rest on the backend.
- `sessionToken` is per-peer, revocable from the desktop Devices panel.
- X25519 key agreement runs on first connect; the derived symmetric key is stored in-memory for the session.
- **TODO (follow-up PR):** migrate the iOS symmetric key to the iOS Keychain for persistence across app restarts without re-pairing.

---

## Known limitations

- Single backend instance only (no multi-region failover).
- No APNs push notifications -- app must be foregrounded for real-time delivery.
- Event-driven pairing detection on the desktop side uses 2-second polling until an SSE/socket event bridge lands.

---

## CI

The `.github/workflows/ios-compile.yml` workflow runs as an iOS compile sanity check. It provides:

- **Hard gate:** `cargo check` on the iOS target for `app/src-tauri-mobile` and a host-target check for `packages/tauri-plugin-ptt`.
- **Hard gate:** TypeScript compile (`pnpm compile`).
- **Hard gate:** iOS-related Vitest suites.

Full signed App Store builds run through `.github/workflows/ios-appstore.yml`.

---

## Backend dependency

The tunnel transport requires `tinyhumansai/backend#709` to be merged and deployed before end-to-end pairing works. The `devices_create_pairing` RPC will return a tunnel registration error until that backend is live.
