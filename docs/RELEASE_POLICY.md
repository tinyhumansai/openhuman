# Release policy: latest desktop builds and OAuth

This runbook describes how we avoid users completing **OAuth** (including **Gmail**) on **outdated desktop installers** while the canonical flow is the **latest** release.

## Distribution

- **GitHub Releases** for [tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman/releases) are the primary source for desktop builds.
- The **Tauri updater** endpoint (see `scripts/prepareTauriConfig.js` and release workflows) should point users at the current release artifacts.
- **Retiring old stable artifacts:** When dropping a release line, remove or hide obsolete installer assets on **GitHub Releases**, update **website / CDN** download links to **releases/latest** (or current), refresh the **updater manifest** (e.g. Gist / `latest.json`) so it does not point users at deprecated builds, and spot-check that old direct URLs are **redirected, 404, or 410** where appropriate. Verification: try known-old asset URLs from docs or bookmarks and confirm they no longer deliver primary install paths.

## Minimum app version for OAuth

Production web builds embed a **minimum supported app semver** at **build time** so OAuth deep links cannot complete on deprecated binaries. Each installer carries the floor that was set when that build was produced; raising the floor for users who never upgrade requires a **new** release they install (or in-app update). Optional future work: enforce a moving minimum via a **runtime** API with the bundled value as fallback only.

| Variable                             | Purpose                                                                                                               |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `VITE_MINIMUM_SUPPORTED_APP_VERSION` | e.g. `0.51.0` — desktop app must be **≥** this to finish `openhuman://oauth/success`.                                 |
| `VITE_LATEST_APP_DOWNLOAD_URL`       | Optional; defaults to `https://github.com/tinyhumansai/openhuman/releases/latest`. Opened when the gate blocks OAuth. |

Configure these as **GitHub Actions variables**. They must be present on **both** the standalone **`pnpm build`** step and the **`tauri-apps/tauri-action`** step env in `.github/workflows/release.yml` and `build-windows.yml` so the Vite bundle embedded in shipped installers includes the gate. Leave `VITE_MINIMUM_SUPPORTED_APP_VERSION` **unset** for local dev (gate disabled).

Implementation: `app/src/utils/oauthAppVersionGate.ts`, `app/src/utils/desktopDeepLinkListener.ts`.

## Gmail / Google Cloud OAuth

- **Redirect URIs** in Google Cloud Console must match the **current** backend + tunnel callback paths.
- The desktop scheme (`openhuman://`) is stable; the **installed binary** must meet the minimum version when `VITE_MINIMUM_SUPPORTED_APP_VERSION` is set.

## Release checklist (avoid regressions)

1. Bump `app/package.json` and `app/src-tauri/tauri.conf.json` (and root `Cargo.toml` / core) per existing version workflows.
2. When dropping support for older installs, set **`VITE_MINIMUM_SUPPORTED_APP_VERSION`** to the new floor **before** or **with** that release (repo Actions variables + both workflow steps above).
3. Remove, redirect, or retire older stable installers and stale **updater** entries from user-facing surfaces (GitHub Release assets, website, CDN, updater feed). Confirm deprecated artifacts are not reachable from default install/update flows.
4. Smoke-test **Gmail connect** on a fresh install from **releases/latest**.
5. Complete the [manual smoke checklist](./RELEASE-MANUAL-SMOKE.md) and paste the sign-off block into the release PR description before tagging.
