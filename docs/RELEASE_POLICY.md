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

Configure these as **GitHub Actions variables**. They must be present on **both** the standalone **`pnpm build`** step and the **`tauri-apps/tauri-action`** step env in `.github/workflows/build-desktop.yml` (the reusable matrix invoked by `release-production.yml` / `release-staging.yml`) and `build-windows.yml` so the Vite bundle embedded in shipped installers includes the gate. Leave `VITE_MINIMUM_SUPPORTED_APP_VERSION` **unset** for local dev (gate disabled).

Implementation: `app/src/utils/oauthAppVersionGate.ts`, `app/src/utils/desktopDeepLinkListener.ts`.

## Gmail / Google Cloud OAuth

- **Redirect URIs** in Google Cloud Console must match the **current** backend + tunnel callback paths.
- The desktop scheme (`openhuman://`) is stable; the **installed binary** must meet the minimum version when `VITE_MINIMUM_SUPPORTED_APP_VERSION` is set.

## Release checklist (avoid regressions)

1. Bump `app/package.json` and `app/src-tauri/tauri.conf.json` (and root `Cargo.toml` / core) per existing version workflows.
2. When dropping support for older installs, set **`VITE_MINIMUM_SUPPORTED_APP_VERSION`** to the new floor **before** or **with** that release (repo Actions variables + both workflow steps above).
3. Remove, redirect, or retire older stable installers and stale **updater** entries from user-facing surfaces (GitHub Release assets, website, CDN, updater feed). Confirm deprecated artifacts are not reachable from default install/update flows.
4. Smoke-test **Gmail connect** on a fresh install from **releases/latest**.
5. Complete the [manual smoke checklist](./RELEASE-MANUAL-SMOKE.md), then paste the completed sign-off block (verbatim, with every checked item left checked) into the release PR description before tagging.

## Workflows: staging vs. production

Two first-class GitHub Actions workflows, one per environment. Pick by intent rather than toggling a flag.

| Workflow                                                | Branch    | Bumps   | Tags pushed                | Concurrency group       | Use when                                                              |
| ------------------------------------------------------- | --------- | ------- | -------------------------- | ----------------------- | --------------------------------------------------------------------- |
| [`release-staging.yml`](../.github/workflows/release-staging.yml) | `staging` | `patch` only | `v<version>-staging`        | `release-staging`       | Cutting a staging build for QA. Runs frequently; narrow semver moves. |
| [`release-production.yml`](../.github/workflows/release-production.yml) | `main`    | `patch` / `minor` / `major` (only on `main_head`) | `v<version>`                | `release-production`    | Promoting a validated staging tag, or hotfixing from `main` HEAD.     |

The matrix build / sign / Sentry-DIF / artifact-upload pipeline used by both flows lives in [`.github/workflows/build-desktop.yml`](../.github/workflows/build-desktop.yml) as a `workflow_call` reusable workflow. The two top-level workflows above own ref resolution, version bumping, tagging, and publish/cleanup; the build itself is shared.

### Cutting a staging build

1. Push to `staging` branch. Run **Release (Staging)** via `workflow_dispatch`.
2. The workflow bumps `patch` on staging, commits `chore(staging): vX.Y.Z`, pushes the branch, and creates an immutable `vX.Y.Z-staging` tag at that commit.
3. Build matrix runs from the **tag** (not the branch tip), so reruns rebuild byte-identical content even if `staging` has moved on.
4. On failure the staging tag is auto-deleted; the bump commit stays so the next cut continues from `vX.Y.(Z+1)`.

### Promoting to production (default flow)

1. Run **Release Production** via `workflow_dispatch` with `release_source = staging_tag` (the default).
2. Leave `staging_tag` blank to promote the latest `v*-staging`, or pass an explicit tag (e.g. `v1.2.4-staging`) to pin.
3. The workflow strips `-staging`, creates `v<version>` at the same commit, and runs the production build matrix from that tag. **No further version bump** — the artifact reuses what staging already validated.

### Hotfix from `main` HEAD

1. Run **Release Production** via `workflow_dispatch` with `release_source = main_head` and the desired `release_type` (`patch` / `minor` / `major`).
2. The workflow runs the legacy bump-and-tag path: bump on `main`, commit `chore(release): vX.Y.Z`, push, tag `vX.Y.Z`, build.
3. Use this only when a production-only fix needs to ship without going through staging.

### Tag policy and rollback

- **Naming.** Staging tags use the SemVer pre-release suffix `-staging` (`v1.2.4-staging`) so they sort *before* the matching production tag. Promotion to production drops the suffix verbatim; the version embedded in the bundled installer is identical between the two tags.
- **Collisions.** Both workflows fail fast if the target tag already exists locally or on `origin`. Resolve by deleting the stale tag (org maintainers only) or bumping past it.
- **Rollback (production).** A failed build matrix triggers `cleanup-failed-release`, which deletes both the draft GitHub Release and the `v<version>` tag. The staging tag it was promoted from is left untouched and can be re-promoted after fixing.
- **Rollback (staging).** A failed staging build deletes the `v<version>-staging` tag. The bump commit on the `staging` branch is left in place; the next staging cut continues from the new patch number rather than re-using it (we accept a small “gap” in patch numbers over racing with concurrent merges).
- **Who can delete tags.** Same write-access as the `staging` / `main` branches. Workflow-driven cleanup deletes run with the workflow's token via `actions/github-script` (the GitHub App token is only used by `prepare-build` for the bump commit + tag push); manual deletes (`git push --delete origin <tag>`) require equivalent maintainer permissions.
