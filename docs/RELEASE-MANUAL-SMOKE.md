# Release Manual Smoke Checklist

Run this checklist on every release-cut. Sign-off lives in the release PR description (paste the checklist with checked items + the sign-off block at the bottom). Owns OS-level surfaces that drivers cannot assert — everything else is automated under WDIO, Vitest, or Rust integration tests (see [`TESTING-STRATEGY.md`](./TESTING-STRATEGY.md)).

This is the **only** acceptable substitute for a `🚫` row in [`TEST-COVERAGE-MATRIX.md`](./TEST-COVERAGE-MATRIX.md). If a feature has neither automated coverage nor an entry on this checklist, treat it as untested and open a coverage gap.

---

## How to use

1. Build the release artifact for each platform you ship.
2. On a clean machine (or fresh user account), walk through `## Per-release smoke` then the section for the active release line.
3. Tick each box only after you have verified the expected outcome with your own eyes.
4. Paste the completed checklist + sign-off block into the release PR description.
5. Any item that is genuinely not applicable for this release: mark `N/A` with a one-line reason; do not silently skip.

---

## Per-release smoke

Applies to every release, all platforms.

### macOS

- [ ] **Gatekeeper accepts the signed `.app` on first launch** — Double-click the `.app` from a fresh download (Quarantine attribute set). Expected: app opens without `"OpenHuman" cannot be opened because the developer cannot be verified` dialog. If it appears, the build is unsigned or the notarization stapler is missing.
- [ ] **`codesign --verify --deep --strict <path-to-OpenHuman.app>` exits 0** — Run from terminal. Expected: no output, exit 0. Any `code object is not signed at all` or `invalid signature` output blocks the release.
- [ ] **DMG drag-to-Applications flow works** — Mount the `.dmg`, drag `OpenHuman.app` to the `Applications` alias. Expected: copy completes; eject succeeds; first launch from `/Applications` does not re-prompt Gatekeeper.
- [ ] **Accessibility permission prompt fires on first agent run** — Trigger an agent action that uses Accessibility (e.g. window-control skill). Expected: macOS prompts `OpenHuman would like to control this computer using accessibility features`. Granting it allows the action; denying it surfaces a clear in-app fallback.
- [ ] **Input Monitoring prompt fires on first hotkey use** — Press the registered global hotkey for the first time. Expected: `Input Monitoring` prompt; granting it makes the hotkey trigger; denying it does not crash the app.
- [ ] **Screen Recording prompt fires on first screen-share** — Use the screen-share skill or `getDisplayMedia` shim. Expected: `Screen Recording` prompt; granted → picker shows windows + screens; denied → in-app message explaining the requirement.
- [ ] **Microphone prompt fires on first voice capture** — Start a voice session. Expected: standard mic prompt; granted → capture begins; denied → fallback message, no panic.

### Windows

- [ ] **SmartScreen does not block install** — Run the installer from a fresh download. Expected: SmartScreen passes (signed binary). If `Windows protected your PC` appears, the EV signature is missing or the reputation has not built up — escalate before shipping.
- [ ] **Installer creates Start Menu + Desktop shortcuts** — Defaults preserved. Expected: both shortcuts launch the app.
- [ ] **App registers `openhuman://` URL scheme** — From a browser, click an `openhuman://oauth/success?...` link. Expected: OS prompts to open in OpenHuman; clicking through delivers the deep link.

### Linux

- [ ] **`.deb` and/or `.AppImage` install on a clean Ubuntu 22.04** — `sudo dpkg -i openhuman_*.deb` or `chmod +x openhuman-*.AppImage && ./openhuman-*.AppImage`. Expected: no missing-dependency errors; app launches.
- [ ] **OS-native notification toasts fire** — Trigger a notification from inside the app (e.g. memory captured, agent finished). Expected: a libnotify-style toast appears outside the app window. (CI Linux only sees Xvfb — this surface only verifies on a real desktop.)

### Cross-platform

- [ ] **First launch flow completes for a brand-new user** — Fresh OS user account, no `~/.openhuman` directory. Walk through onboarding to first agent reply. Expected: no crashes, no permission deadlocks, no stale-config errors.
- [ ] **Auto-update download + relaunch succeeds** — Install the previous release, point the updater feed at this release, trigger an update check. Expected: download completes, relaunch installs the new binary, version string in `Settings > About` matches the release tag.
- [ ] **Logging out + logging back in preserves nothing private** — Sign out, sign in as a different user. Expected: no leaked memory, threads, or skill state from the previous session (regression watch — see #900).

---

## Active release line

> If multiple stable release lines are in flight (security backports, LTS), add a sub-section per line and check the same boxes for each. As of writing, `0.52.x` is the only active line — older minor versions are end-of-life. Fold this section to suit when more release lines exist.

### 0.52.x — current

- [ ] **OAuth gate respects `VITE_MINIMUM_SUPPORTED_APP_VERSION`** (per [`RELEASE_POLICY.md`](./RELEASE_POLICY.md)) — Set the variable to a value above this build's version, build, attempt OAuth from the older binary. Expected: gate blocks the deep link; opens `VITE_LATEST_APP_DOWNLOAD_URL`.
- [ ] **Gmail connect succeeds on a fresh install from `releases/latest`** — Per release-policy step 4. Expected: token exchange completes, inbox lists in-app.

---

## Sign-off

```
Release: vX.Y.Z
Tester: @<github-handle>
Date: YYYY-MM-DD
Platforms tested: [macOS arm64] [macOS x64] [Windows] [Linux .deb] [Linux .AppImage]
Notes:
```

Paste the filled block into the release PR description before tagging.
