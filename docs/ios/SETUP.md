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

`tauri ios init` scaffolds `app/src-tauri/gen/apple/`. That directory is **gitignored** (it contains bundle-identifier-specific Xcode project files that differ per developer account).

### Info.plist privacy keys

`tauri ios init` creates a generated `Info.plist` at:

```
app/src-tauri/gen/apple/<bundle-id>_iOS/Info.plist
```

You must copy the three privacy keys from `app/src-tauri/Info.ios.plist` into that generated file before building:

```xml
<key>NSCameraUsageDescription</key>
<string>OpenHuman uses the camera to scan the pairing QR code from your desktop.</string>

<key>NSMicrophoneUsageDescription</key>
<string>OpenHuman uses the microphone for push-to-talk voice messages.</string>

<key>NSSpeechRecognitionUsageDescription</key>
<string>OpenHuman uses on-device speech recognition to transcribe your voice messages.</string>
```

**Option A (recommended for now):** Manual copy after each `tauri ios init` run.

**Option B (automate in a follow-up PR):** Set the `bundle.iOS.template` key in `app/src-tauri/tauri.conf.json` to point at a hand-crafted `Info.plist` template once Tauri v2 stabilises its iOS template pipeline. Until that happens, Option A is simpler and less brittle.

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
- Xcode signing must be set manually in the generated project (no CI automation yet).

---

## CI

The `.github/workflows/ios-compile.yml` workflow runs on every PR that touches iOS-related paths. It provides:

- **Hard gate:** `cargo check` on the host target for `app/src-tauri` and `packages/tauri-plugin-ptt`.
- **Hard gate:** TypeScript compile (`pnpm compile`).
- **Hard gate:** iOS-related Vitest suites.
- **Soft gate (`continue-on-error: true`):** `cargo check --target aarch64-apple-ios` -- this catches gross API breakage but may fail on third-party C deps that need full Xcode. Failures are flagged but do not block merge.

Full iOS builds (simulator + device) require macOS runners with Xcode installed. This is tracked as a follow-up to this PR.

---

## Backend dependency

The tunnel transport requires `tinyhumansai/backend#709` to be merged and deployed before end-to-end pairing works. The `devices_create_pairing` RPC will return a tunnel registration error until that backend is live.
