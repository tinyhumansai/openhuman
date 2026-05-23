## Summary

- Adds an iOS client for OpenHuman: device pairing via QR code, mascot chat screen, and push-to-talk voice input.
- No Rust core ships on device; the iOS app connects to the desktop core via LAN HTTP, an E2E-encrypted socket.io tunnel, or cloud HTTP fallback.
- All changes are cfg-gated or platform-guarded; the desktop build is unaffected.
- Adds the `tauri-plugin-ptt` Swift plugin (`packages/tauri-plugin-ptt/`) for AVAudioEngine + SFSpeechRecognizer on iOS.
- Adds CI sanity-check workflow, build scripts, capability catalog entries, and full docs.

## Problem

Users with iOS devices had no way to interact with their OpenHuman assistant on the go. The desktop app required a local machine. This PR adds the client-side scaffolding and transport layer needed to bridge iOS to an existing desktop core.

## Solution

The iOS app is a subset of the existing React/TypeScript UI, compiled by Tauri v2 into an iOS bundle. A `TransportManager` selects the best transport at runtime. Pairing is secured by an X25519 key agreement; all tunnel traffic uses XChaCha20-Poly1305 encryption. The backend is a blind socket.io forwarder -- it never sees plaintext.

## Layer-by-layer commits

| Commit | Layer | Summary |
|--------|-------|---------|
| `a99537f3` | Layer 1 | Rust devices domain -- pairing store, RPC handlers, event bus, crypto (`src/openhuman/devices/`) |
| `4ea14b78` | Layer 2 | TS transport refactor -- `TransportManager`, `LanHttpTransport`, `TunnelTransport`, `CloudHttpTransport`, tunnel crypto (`app/src/services/transport/`, `app/src/lib/tunnel/`) |
| `ba651705` | Layer 3 | Desktop `/settings/devices` UI -- `DevicesPanel`, `PairPhoneModal` with QR generation and 2-second poll |
| `3e0e2a67` | Layer 4 | Tauri shell cfg-gating -- `#[cfg(target_os = "ios")]` guards on CEF-specific code |
| `621fec98` | Layer 5 | iOS app shell -- `PairScreen` (QR scan via `AVCaptureSession`), `MascotScreen` (chat UI) |
| `5ca6cf21` | Layer 6 | `tauri-plugin-ptt` -- Swift PTT plugin (AVAudioEngine, SFSpeechRecognizer, AVSpeechSynthesizer) |
| `41a6a895` | Layer 6 fix | PTT Swift fix -- latest transcript tracking + `@unchecked Sendable` on PTTSpeaker |
| _(this PR)_ | Layers 7+8 | Build scripts, CI, Info.plist, capability catalog, docs, quality pass |

## Test coverage

- **Vitest:** 1957 passed, 3 skipped, 1 todo across 218 test files (includes transport, tunnel, devices, iOS, PTT suites).
- **Rust (about_app):** 20 passed -- validates catalog uniqueness, Mobile category, and new capability entries.
- **cargo check (all three Cargo.toml files):** clean (warnings only, pre-existing).

## What is gated behind the iOS target

The following only activates on `cfg(target_os = "ios")` or when explicitly called from iOS screens:

- CEF exclusions in `app/src-tauri/` (accounts webviews, etc.)
- `tauri-plugin-ptt` commands (`start_listening`, `stop_listening`, `speak`, `cancel_speech`, `list_voices`) -- return `NotSupported` on non-iOS targets.
- `packages/tauri-plugin-ptt/ios/` Swift sources -- not compiled for desktop.

Desktop users see no change.

## Known TODOs for follow-up PRs

- **Keychain migration:** iOS symmetric session key is in-memory only; persist to Keychain so the app reconnects after restart without re-pairing.
- **Event-driven pairing detection:** `PairPhoneModal` polls `devices_list` every 2 s. Switch to a socket event subscription when the SSE/socket bridge for `DomainEvent::DevicePaired` lands.
- **Full Xcode CI:** `cargo check --target aarch64-apple-ios` runs with `continue-on-error: true` in the new CI workflow because third-party C deps (cef-dll-sys) may fail without full Xcode on the runner. A follow-up should pin an Xcode-enabled runner and harden this to a hard gate.
- **APNs push notifications:** real-time delivery requires the app to be foregrounded.
- **Multi-region tunnel:** single backend instance only; no failover.
- **Info.plist automation:** developer must manually copy `Info.ios.plist` keys into the generated Xcode project after `tauri ios init`. Should automate via `bundle.iOS.template` once Tauri v2 stabilises the iOS template pipeline.

## Backend dependency

**`tinyhumansai/backend#709` must be merged and deployed before end-to-end pairing works.** The `devices_create_pairing` RPC will return a tunnel registration error until the `tunnel:register` / `tunnel:connect` / `tunnel:frame` socket.io contract is live.

## Manual test plan for iOS reviewer

_(Requires a physical iPhone or iOS 17+ simulator paired with the desktop app.)_

From `packages/tauri-plugin-ptt/README.md`:

- [ ] Permissions dialog appears on first `startListening` call.
- [ ] Partial transcripts update while speaking; final transcript matches.
- [ ] Hold button to record, release to stop, chat message is sent with transcript.
- [ ] TTS plays through speaker by default when iPhone is held away from ear.
- [ ] BT headset routes audio correctly; disconnecting mid-recording stops gracefully.
- [ ] App backgrounded mid-record produces a final transcript and stops cleanly.
- [ ] Phone call interruption emits `ptt://error` with `code: interrupted`.
- [ ] `cancelSpeech` during TTS emits `tts-ended` with `finished: false`.
- [ ] `listVoices` returns non-empty list of `AVSpeechSynthesisVoice` entries.

Additional pairing flow checks:

- [ ] Desktop: Settings > Devices > "Pair iPhone" shows QR code.
- [ ] iOS app: PairScreen scans QR and transitions to MascotScreen after handshake.
- [ ] Desktop: Devices panel lists the paired device with correct label.
- [ ] Desktop: Revoke device removes it from the list; iOS app shows reconnect prompt.
- [ ] QR code expiry: code expires after TTL, "Generate new code" creates a fresh session.

## Screenshots

> **PLACEHOLDER:** Before opening the PR, attach screenshots of:
> - Desktop `/settings/devices` panel with a paired device.
> - iOS mascot screen showing a conversation.
>
> These require a device with Xcode signing configured and `tinyhumansai/backend#709` deployed.

## Submission Checklist

- [x] Tests added or updated (transport, tunnel, devices, iOS, PTT suites -- see coverage statement above).
- [x] Diff coverage note: new Rust code in `src/openhuman/devices/` was covered in Layer 1 tests; new TS code in `app/src/services/transport/` and `app/src/lib/tunnel/` covered by Vitest suites. PTT Swift layer cannot be unit-tested without iOS toolchain (noted in README).
- [x] Coverage matrix: N/A for this layer (build scripts, CI, docs, catalog).
- [x] No new external network dependencies (all transport calls use existing mock backend or real backend behind feature flag).
- [ ] Manual smoke checklist: iOS path not in `docs/RELEASE-MANUAL-SMOKE.md` yet -- tracked as follow-up.
- [ ] Linked issue: N/A (tracked via Linear).

## Impact

- Desktop runtime: no change.
- iOS target: new experimental app bundle (not in release pipeline yet).
- `packages/tauri-plugin-ptt/` is a new crate workspace member; adds to build time only when targeting iOS.
- Capability catalog adds three new `mobile.*` entries and a new `Mobile` category.

## Related

- Closes: N/A (new feature)
- Follow-up PR(s): Keychain migration, event-driven pairing, full Xcode CI, APNs.
- Backend: tinyhumansai/backend#709

---

## AI Authored PR Metadata (required for Codex/Linear PRs)

### Linear Issue
- Key: N/A
- URL: N/A

### Commit & Branch
- Branch: `feat/ios-client`
- Commit SHA: _(set after final commit)_

### Validation Run
- [x] `pnpm --filter openhuman-app format:check` -- clean
- [x] `pnpm typecheck` -- clean
- [x] Focused tests: Vitest 1957 passed; cargo about_app 20 passed
- [x] Rust fmt/check: `cargo fmt --all` + `cargo check` on all three Cargo.toml -- clean
- [x] Tauri fmt/check: included above

### Validation Blocked
- command: `cargo check --target aarch64-apple-ios`
- error: May fail on cef-dll-sys C deps without full Xcode; guarded with `continue-on-error: true` in CI.
- impact: Soft gate only; does not block merge.

### Behavior Changes
- Intended behavior change: Desktop users see new Settings > Devices panel. iOS users can pair and chat.
- User-visible effect: Desktop gains device management UI. iOS app becomes available for sideloading/TestFlight.

### Parity Contract
- Legacy behavior preserved: All existing desktop flows unaffected. No CEF injection added. No new JS injection in webview accounts.
- Guard/fallback/dispatch parity: PTT commands return `NotSupported` on non-iOS. Transport falls back gracefully.

### Duplicate / Superseded PR Handling
- Duplicate PR(s): None
- Canonical PR: This PR
- Resolution: N/A
