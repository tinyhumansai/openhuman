# Release Manual Smoke Checklist

Run this checklist on every release-cut. Sign-off lives in the release PR description (paste the checklist with checked items + the sign-off block at the bottom). Owns OS-level surfaces that drivers cannot assert — everything else is automated under WDIO, Vitest, or Rust integration tests (see [Testing Strategy](../gitbooks/developing/testing-strategy.md)).

This is the **only** acceptable substitute for a `🚫` row in [`TEST-COVERAGE-MATRIX.md`](./TEST-COVERAGE-MATRIX.md). If a feature has neither automated coverage nor an entry on this checklist, treat it as untested and open a coverage gap.

---

## How to use

1. Build the release artifact for each platform you ship.
2. On a clean machine (or fresh user account), walk through `## Per-release smoke` then the section for the active release line.
3. Tick each box only after you have verified the expected outcome with your own eyes.
4. Paste the completed checklist + sign-off block into the release PR description.
5. Any item that is genuinely not applicable for this release: mark `N/A` with a one-line reason; do not silently skip.
6. If `release-staging.yml` was dispatched with `skip_e2e=true`, record the reason and link the most recent relevant green pretest evidence in the PR notes (unit/rust and E2E as applicable). That override is for operator recovery, not the default release path.

---

## Per-release smoke

Applies to every release, all platforms.

### Public installer script

- [ ] **`scripts/install.sh` downloads the latest asset on a proxy/VPN network** — From a clean checkout, run `bash scripts/install.sh --dry-run --verbose`, then run the public `curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash` flow on one macOS or Linux host. Expected: release metadata resolves, the asset downloads successfully, and transient GitHub/CDN HTTP/2 failures retry over HTTP/1.1 instead of surfacing `curl: (16) Error in the HTTP2 framing layer`.

### macOS

- [ ] **Gatekeeper accepts the signed `.app` on first launch** — Double-click the `.app` from a fresh download (Quarantine attribute set). Expected: app opens without `"OpenHuman" cannot be opened because the developer cannot be verified` dialog. If it appears, the build is unsigned or the notarization stapler is missing.
- [ ] **`codesign --verify --deep --strict <path-to-OpenHuman.app>` exits 0** — Run from terminal. Expected: no output, exit 0. Any `code object is not signed at all` or `invalid signature` output blocks the release.
- [ ] **DMG drag-to-Applications flow works** — Mount the `.dmg`, drag `OpenHuman.app` to the `Applications` alias. Expected: copy completes; eject succeeds; first launch from `/Applications` does not re-prompt Gatekeeper.
- [ ] **Accessibility permission prompt fires on first agent run** — Trigger an agent action that uses Accessibility (e.g. window-control skill). Expected: macOS prompts `OpenHuman would like to control this computer using accessibility features`. Granting it allows the action; denying it surfaces a clear in-app fallback.
- [ ] **Input Monitoring prompt fires on first hotkey use** — Press the registered global hotkey for the first time. Expected: `Input Monitoring` prompt; granting it makes the hotkey trigger; denying it does not crash the app.
- [ ] **Screen Recording prompt fires on first screen-share** — Use the screen-share skill or `getDisplayMedia` shim. Expected: `Screen Recording` prompt; granted → picker shows windows + screens; denied → in-app message explaining the requirement.
- [ ] **Meet "Present" surfaces the Chrome screen-picker (regression watch — see #2636)** — Open the Google Meet webview account, join a meeting, and click `Present now`. Expected: Chromium's native screen-picker UI appears (Entire screen / Window / Chrome tab tabs) and `getDisplayMedia` only resolves after the user picks a source. Hard fail mode: capture starts immediately with no picker — that means `displayCapture` was re-granted via `Browser.grantPermissions` and bypassed Chromium's transient-activation gate.
- [ ] **Slack huddle screen-share surfaces the Chrome screen-picker (regression watch — see #2636)** — Open the Slack webview account, start or join a huddle, and click the screen-share button. Expected: same Chromium native screen-picker as Meet; capture only begins after a deliberate user selection. Hard fail mode: huddle begins broadcasting immediately with no picker prompt.
- [ ] **Microphone prompt fires on first voice capture** — Start a voice session. Expected: standard mic prompt; granted → capture begins; denied → fallback message, no panic.
- [ ] **Bluetooth prompt fires on first Gmeet call (regression watch — see #1288)** — Open the Google Meet webview account and join a meeting from a fresh install. Expected: macOS prompts `OpenHuman would like to use Bluetooth` the first time the device picker enumerates audio peripherals; granted → AirPods/headsets appear in the picker; denied → fallback to built-in mic, no crash. Hard fail mode (key absent) is a SIGABRT before the prompt can render.
- [ ] **Location prompt does not crash on Gmeet room-finder probe** — If Gmeet surfaces nearby-room suggestions, the first probe should trigger `OpenHuman would like to use your current location`; granting or denying must NOT crash the app. (Probe path is webview-driven; only verify the no-crash invariant here.)
- [ ] **File picker does not crash on Documents/Downloads/Desktop selections** — From an embedded app (Slack, Discord, Telegram), trigger a file upload and pick a file from `Documents`, `Downloads`, and `Desktop` in turn. Expected: macOS prompts `OpenHuman would like to access files in your <Folder> folder` the first time per folder; deny + retry must not crash.

### Windows

- [ ] **SmartScreen does not block install** — Run the installer from a fresh download. Expected: SmartScreen passes (signed binary). If `Windows protected your PC` appears, the EV signature is missing or the reputation has not built up — escalate before shipping.
- [ ] **Installer creates Start Menu + Desktop shortcuts** — Defaults preserved. Expected: both shortcuts launch the app.
- [ ] **App registers `openhuman://` URL scheme** — From a browser, click an `openhuman://oauth/success?...` link. Expected: OS prompts to open in OpenHuman; clicking through delivers the deep link.

### Linux

- [ ] **Public `install.sh` prefers the `.deb` path on clean Ubuntu 24.04** — Run `curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash` on a host with `apt-get` and `dpkg`. Expected: the script resolves `OpenHuman_*_amd64.deb` or `OpenHuman_*_arm64.deb`, installs it with `apt-get`, and launch does not fail on missing CEF runtime libraries such as `libgbm.so.1`.
- [ ] **`.deb` and/or `.AppImage` install on a clean Ubuntu 22.04** — `sudo apt-get install -y --no-install-recommends ./OpenHuman_*.deb` or `chmod +x OpenHuman_*.AppImage && ./OpenHuman_*.AppImage`. Expected: no missing-dependency errors; app launches.
- [ ] **`.AppImage` launches on a clean Ubuntu 24.04 host without a sibling extracted tree** — Run the downloaded AppImage directly from an empty directory. Expected: no `Interpreter not found!` error; `sharun` finds its bundled dynamic linker and the app reaches the first window.
- [ ] **OS-native notification toasts fire** — Trigger a notification from inside the app (e.g. memory captured, agent finished). Expected: a libnotify-style toast appears outside the app window. (CI Linux sees only Xvfb; this surface verifies on a real desktop.)
- [ ] **Headless supervisor update stages without self-exit** — On a Linux service deployment with `[update] restart_strategy = "supervisor"` and `rpc_mutations_enabled = false`, stage a new core binary through the documented operator flow. Expected: the running process stays up until the supervisor restart, the staged binary is present on disk, and `systemctl restart openhuman` (or equivalent) picks up the new version.

### Cross-platform

- [ ] **First launch flow completes for a brand-new user** — Fresh OS user account, no `~/.openhuman` directory. Walk through onboarding to first agent reply. Expected: no crashes, no permission deadlocks, no stale-config errors.
- [ ] **Auto-update download + relaunch succeeds** — Install the previous release, point the updater feed at this release, trigger an update check. Expected: download completes, relaunch installs the new binary, version string in `Settings > About` matches the release tag.
- [ ] **GitHub Release notes are AI-generated from the previous release tag** — Before publishing the draft production release, inspect the GitHub Release body. Expected: notes start with a thematic H1 title, include high-level highlight sections with PR links and contributor thanks, omit a separate pull-request dump, include new-contributor thanks only when applicable, and the full compare URL is previous release tag → current release tag.
- [ ] **Logging out + logging back in preserves nothing private** — Sign out, sign in as a different user. Expected: no leaked memory, threads, or skill state from the previous session (regression watch — see #900).
- [ ] **`memory_tree` migrates WAL→TRUNCATE on upgrade with memory intact** — Install a previous (WAL-era) build, use it enough to populate memory so a `chunks.db-wal`/`-shm` pair exists under `~/.openhuman/.../workspace/memory_tree/`, then upgrade to this build. Expected on first launch: `PRAGMA journal_mode` on `chunks.db` reports `truncate`, the `-wal`/`-shm` side-files are gone, previously-captured memories still surface in recall, and no `Failed to initialize memory_tree schema` errors appear.

---

## Active release line

> If multiple stable release lines are in flight (security backports, LTS), add a sub-section per line and check the same boxes for each. As of writing, `0.52.x` is the only active line — older minor versions are end-of-life. Fold this section to suit when more release lines exist.

### 0.52.x — current

- [ ] **OAuth gate respects `VITE_MINIMUM_SUPPORTED_APP_VERSION`** (per [Release Policy](../gitbooks/developing/release-policy.md)) — Set the variable to a value above this build's version, build, attempt OAuth from the older binary. Expected: gate blocks the deep link; opens `VITE_LATEST_APP_DOWNLOAD_URL`.
- [ ] **Gmail connect succeeds on a fresh install from `releases/latest`** — Per release-policy step 4. Expected: token exchange completes, inbox lists in-app.

---

## Sign-off

```text
Release: vX.Y.Z
Tester: @<github-handle>
Date: YYYY-MM-DD
Platforms tested: [macOS arm64] [macOS x64] [Windows] [Linux .deb] [Linux .AppImage]
Notes:
```

Paste the filled block into the release PR description before tagging.
