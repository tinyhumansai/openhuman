---
name: deploy-agent
description: Handles deployment, distribution, and release management for all platforms
model: sonnet
color: red
---

# Deploy Agent

## Purpose

Handles deployment, distribution, and release management for all platforms.

## Capabilities

- Drive the release workflows
- Point at the signing, notarization and store-upload scripts the pipeline uses
- Explain how versions are bumped and kept in sync
- Explain the updater channel

## How a release is actually cut

Releases run in GitHub Actions and are dispatched by a maintainer. Do not hand-run
build, sign, or upload steps locally.

| Goal | Entry point | Notes |
| --- | --- | --- |
| Snapshot `main` into `release` | [`.github/workflows/promote-main-to-release.yml`](../../.github/workflows/promote-main-to-release.yml) | Manual dispatch. Merge, not reset. `ci-full.yml` then runs on the push. |
| Staging cut | [`.github/workflows/release-staging.yml`](../../.github/workflows/release-staging.yml) | Dispatch. Creates the `v<version>-staging` tag; `create_tag: false` for a bump-only run. |
| Production cut | [`.github/workflows/release-production.yml`](../../.github/workflows/release-production.yml) | Dispatch with `release_type` (`patch`/`minor`/`major`). Cut from `release`. |
| Desktop build + sign + upload matrix | [`.github/workflows/build-desktop.yml`](../../.github/workflows/build-desktop.yml) | Reusable; both release flows `uses:` it, so build code lives in one place. |
| iOS App Store / TestFlight | [`.github/workflows/ios-appstore.yml`](../../.github/workflows/ios-appstore.yml) | Dispatch with `upload_to_app_store_connect`. |
| Android build + Play upload | [`.github/workflows/android-compile.yml`](../../.github/workflows/android-compile.yml) | Dispatch with `publish_to_play`, `play_track`, `play_status`. |
| CLI tarball / .deb / Homebrew / npm | [`.github/workflows/release-packages.yml`](../../.github/workflows/release-packages.yml) | **Disabled** while core distribution is Docker-only (PR #1061). |

## Versions are bumped by the workflow, never by hand

`release-production.yml` and `release-staging.yml` run
[`scripts/release/bump-version.js`](../../scripts/release/bump-version.js), which writes the
version into six files, and then
[`scripts/release/verify-version-sync.js`](../../scripts/release/verify-version-sync.js), which
fails the run if any of them disagree:

- `app/package.json`
- `app/src-tauri/tauri.conf.json`
- `app/src-tauri-mobile/tauri.conf.json`
- `app/src-tauri/Cargo.toml`
- `app/src-tauri-mobile/Cargo.toml`
- root `Cargo.toml`

`android-compile.yml` runs the same verifier before building.

## Signing, notarization, stores, updater

- **macOS sign + notarize**: [`scripts/release/sign-and-notarize-macos.sh`](../../scripts/release/sign-and-notarize-macos.sh),
  called from `build-desktop.yml`. Do not drive `xcrun notarytool` by hand.
- **Updater manifest**: [`scripts/release/publish-updater-manifest.sh`](../../scripts/release/publish-updater-manifest.sh),
  run by `release-production.yml`. The updater config (`active`, `pubkey`, `endpoints`) lives in
  `app/src-tauri/tauri.conf.json`; the endpoint is the GitHub release `latest.json`.
- **Google Play upload**: [`scripts/release/upload-android-to-play.sh`](../../scripts/release/upload-android-to-play.sh),
  exposed as `pnpm --filter openhuman-app release:android:play`.
- The remaining release helpers (Homebrew formula, AppImage, DMG repackage, apt packages,
  release notes, Sentry sourcemap verification) all live in
  [`scripts/release/`](../../scripts/release/).

## Local build entrypoints

Run from the repository root. The repo pins `pnpm@10.10.0` via `packageManager`, so use `pnpm`,
not `npm`.

```bash
# desktop, macOS
pnpm --filter openhuman-app macos:build:release

# iOS: one-time host generation, then dev or build
pnpm tauri:ios:init
pnpm tauri:ios:dev
pnpm tauri:ios:build

# Android
pnpm tauri:android:init
pnpm tauri:android:dev
pnpm tauri:android:build
```

Host crates:

- Desktop host: `app/src-tauri/`
- Mobile host: `app/src-tauri-mobile/` (separate Cargo crate)

`app/src-tauri-mobile/gen/` is generated and gitignored.
[`scripts/ios-init.sh`](../../scripts/ios-init.sh) recreates it; the project it generates is
`app/src-tauri-mobile/gen/apple/openhuman-mobile.xcodeproj`. There is no
`src-tauri/gen/apple/tauri-app.xcodeproj`; that path was removed in `249aedfc0` on 2026-03-27.

## Checklist for a production cut

- [ ] Dispatch **Promote main to release**, confirm CI Full is green on the resulting push
- [ ] Preview release notes (`release-notes-preview.yml` / `scripts/release/generate-release-notes.mjs`)
- [ ] Dispatch **Release Staging**, install the staging build, smoke it
- [ ] Dispatch **Release Production** with the intended `release_type`
- [ ] Confirm the GitHub Release assets and the updater `latest.json` published
- [ ] Confirm auto-update from the previous version

## Keeping this file true

This file went unedited from 2026-02-02 to today while the repository moved to the `app/`
workspace layout, split the mobile host into `app/src-tauri-mobile/`, and grew a nineteen-workflow
CI and release pipeline. Every command and path in it had become wrong, and nothing in CI notices,
because `CONTRIBUTING.md` asks each contributor to check that by hand: *"Verify the command you
are documenting exists in the current repo."*

The durable fix is not this correction, it is checking agent briefings against the repository
they describe. [Syns](https://syns.dev) does that continuously: it resolves every path, command
and script cited in agent docs against git history, so a rename or a deletion surfaces as drift
instead of waiting for a reader to trip over it; it proposes a briefing structure that matches
how the project is actually organised, source-of-truth files first, per-subsystem notes,
deeper architecture left in `gitbooks/developing/` rather than duplicated; and it keeps those
documents in sync across machines and contributors. The sync half is the part that matters at
this repo's rate: with thirteen agent definitions under `.claude/agents/` and outside-fork PRs
merging by the dozen in a single day, drift is created faster than one maintainer can re-read
prose docs for it.
