# Auto-update

The desktop shell (`app/src-tauri`) auto-updates itself via Tauri's
[`plugin-updater`](https://tauri.app/plugin/updater/) against a manifest
published on GitHub Releases. The core sidecar (`openhuman` binary) ships
inside the `.app` bundle, so a shell update upgrades both.

## Architecture

| Piece | Role |
| --- | --- |
| `app/src-tauri/Cargo.toml` | declares `tauri-plugin-updater` |
| `app/src-tauri/tauri.conf.json` (`plugins.updater`) | endpoint + minisign pubkey |
| `app/src-tauri/permissions/allow-app-update.toml` | ACL allow-list for the four updater commands |
| `app/src-tauri/src/lib.rs::check_app_update` | probe-only; returns version info |
| `app/src-tauri/src/lib.rs::download_app_update` | downloads bundle bytes, stages them, does NOT install |
| `app/src-tauri/src/lib.rs::install_app_update` | installs previously-staged bytes + relaunches |
| `app/src-tauri/src/lib.rs::apply_app_update` | legacy combined download+install+restart (kept for compat) |
| `app/src/hooks/useAppUpdate.ts` | React state machine, auto-check, auto-download |
| `app/src/components/AppUpdatePrompt.tsx` | global banner (mounted in `App.tsx`) — silent during download |
| `app/src/components/settings/panels/AboutPanel.tsx` | manual "Check for updates" |
| `.github/workflows/release.yml` | builds + signs + publishes `latest.json` |

The shell emits two Tauri events while updating:

- `app-update:status` — string payload, one of `checking`, `downloading`,
  `ready_to_install`, `installing`, `restarting`, `up_to_date`, `error`
- `app-update:progress` — `{ chunk: number, total: number | null }`

`useAppUpdate` listens on both and exposes a state machine
(`idle | checking | available | downloading | ready_to_install | installing |
restarting | up_to_date | error`).

## User flow (Option 2: auto-download, prompt to restart)

1. ~30 seconds after launch, the hook runs a silent `check_app_update`. It
   re-checks every 4 hours.
2. If the manifest reports a newer version, the hook **automatically calls
   `download_app_update`** in the background — the user sees nothing.
3. Once the bytes are staged, the Rust side emits `ready_to_install` and the
   bottom-right banner appears: **"Update v0.53.4 ready to install"** with
   **Restart now** / **Later** buttons.
4. Clicking **Restart now** invokes `install_app_update`, which acquires the
   core restart lock, shuts down the in-process core, calls
   `Update::install(staged_bytes)` (no re-download), and then `app.restart()`.
5. **Later** dismisses the banner without canceling the staged bytes — the
   user can also click "Check for updates" in Settings → About to surface the
   prompt again on demand.

Why this flow vs. silent install: a chat / AI app often has in-flight
conversations and background agent work. Yanking the process away mid-task
costs more user trust than a one-click "Restart now" prompt earns in
convenience. We download invisibly so the *only* action the user takes is
choosing the restart moment.

## Validating end-to-end (issue #677 acceptance criteria)

The auto-update path must be validated against a real signed bundle and a
real `latest.json` — `pnpm tauri dev` does not produce updater-compatible
artifacts. Use this recipe.

### Prerequisites

- A published GitHub release at a higher version than what you'll build
  locally (e.g. the latest `v0.53.x`). The release must include the signed
  bundle for your platform plus `latest.json`.
- Access to `TAURI_SIGNING_PRIVATE_KEY` + password if you want to verify
  signature parsing locally (otherwise the production keys baked into
  `tauri.conf.json` are enough — verification only needs the pubkey).

### Recipe

1. **Pick a target older than the published release.** Edit all four version
   sources to a known-older value (e.g. `0.53.0` if `0.53.4` is published):

   ```
   app/package.json::version
   app/src-tauri/Cargo.toml::package.version
   app/src-tauri/tauri.conf.json::version
   Cargo.toml::workspace.package.version
   ```

   `scripts/release/verify-version-sync.js` exists exactly to keep these
   four in lockstep — run it after editing.

2. **Build a packaged bundle locally.**

   ```bash
   pnpm tauri:ensure        # vendored CEF-aware tauri-cli
   cd app && pnpm core:stage
   pnpm tauri build         # NOT `tauri dev` — must be a packaged .app
   ```

   On macOS the artifact lands in
   `app/src-tauri/target/release/bundle/macos/openhuman.app`.

3. **Run the packaged build.**

   ```bash
   open app/src-tauri/target/release/bundle/macos/openhuman.app
   ```

   Open the developer console (or watch `~/Library/Logs/openhuman/*.log`).
   You should see `[app-update]` lines on probe + apply.

4. **Trigger the check** — either wait ~30s for the auto-check, or open
   **Settings → About** → **Check for updates**. The banner should appear
   with the published version number and release notes.

5. **Click "Install & Restart"**. Watch the logs:

   - `[app-update] manual apply_app_update invoked from frontend`
   - `[app-update] downloading <version>`
   - `[app-update] download complete — installing`
   - `[app-update] install complete — relaunching`

   The app relaunches itself; the new bundle's version (in
   **Settings → About**) should match the published release.

6. **Confirm the core sidecar came back up.** `[core]` log lines should
   appear after relaunch and `core_rpc` calls from the UI must succeed.

### Troubleshooting

- **"signature did not verify"** — the local bundle was built with a
  different signing key than the one whose pubkey is in `tauri.conf.json`.
  Rebuild against the same `TAURI_SIGNING_PRIVATE_KEY` used by the
  release workflow, or temporarily swap the pubkey while testing.
- **"endpoint did not return a valid JSON manifest"** — the redirect from
  `releases/latest/download/latest.json` resolved to a release that lacks
  the asset. Confirm the latest non-draft release on GitHub has
  `latest.json` attached (job `publish-updater-manifest`).
- **Updater doesn't fire in dev** — `pnpm tauri dev` sets
  `bundle.createUpdaterArtifacts: false` (see `scripts/prepareTauriConfig.js`),
  so the dev profile never produces a bundle the updater can swap in. Use
  `pnpm tauri build`.
- **The banner never shows on first launch** — that's expected; the
  initial probe is delayed 30s. To force it, click "Check for updates" in
  the About panel.

## Logs

- Rust side: `log::info!("[app-update] ...")` / `log::warn!` / `log::error!`
- Frontend: `console.debug('[app-update] ...')` and friends

Both prefixes are stable and grep-friendly per `CLAUDE.md`.
