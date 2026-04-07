# Release policy: latest desktop builds and OAuth

This runbook describes how we avoid users completing **OAuth** (including **Gmail**) on **outdated desktop installers** while the canonical flow is the **latest** release.

## Distribution

- **GitHub Releases** for [tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman/releases) are the primary source for desktop builds.
- The **Tauri updater** endpoint (see `scripts/prepareTauriConfig.js` and release workflows) should point users at the current release artifacts.

## Minimum app version for OAuth

Production web builds embed a **minimum supported app semver** at **build time** so OAuth deep links cannot complete on deprecated binaries. Each installer carries the floor that was set when that build was produced; raising the floor for users who never upgrade requires a **new** release they install (or in-app update). Optional future work: enforce a moving minimum via a **runtime** API with the bundled value as fallback only.

| Variable                             | Purpose                                                                                                               |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `VITE_MINIMUM_SUPPORTED_APP_VERSION` | e.g. `0.51.0` — desktop app must be **≥** this to finish `openhuman://oauth/success`.                                 |
| `VITE_LATEST_APP_DOWNLOAD_URL`       | Optional; defaults to `https://github.com/tinyhumansai/openhuman/releases/latest`. Opened when the gate blocks OAuth. |

Configure these as **GitHub Actions variables** (see `Build frontend` in `.github/workflows/release.yml` and `build-windows.yml`). Leave `VITE_MINIMUM_SUPPORTED_APP_VERSION` **unset** for local dev (gate disabled).

Implementation: `app/src/utils/oauthAppVersionGate.ts`, `app/src/utils/desktopDeepLinkListener.ts`.

## Gmail / Google Cloud OAuth

- **Redirect URIs** in Google Cloud Console must match the **current** backend + tunnel callback paths.
- The desktop scheme (`openhuman://`) is stable; the **installed binary** must meet the minimum version when `VITE_MINIMUM_SUPPORTED_APP_VERSION` is set.

## Release checklist (avoid regressions)

1. Bump `app/package.json` and `app/src-tauri/tauri.conf.json` (and root `Cargo.toml` / core) per existing version workflows.
2. When dropping support for older installs, set **`VITE_MINIMUM_SUPPORTED_APP_VERSION`** to the new floor **before** or **with** that release.
3. Smoke-test **Gmail connect** on a fresh install from **releases/latest**.
