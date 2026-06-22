---
description: Release cadence, version policy, OAuth-and-installer rules. How shipping works.
icon: ship
---

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
| `VITE_MINIMUM_SUPPORTED_APP_VERSION` | e.g. `0.51.0` - desktop app must be **≥** this to finish `openhuman://oauth/success`.                                 |
| `VITE_LATEST_APP_DOWNLOAD_URL`       | Optional; defaults to `https://github.com/tinyhumansai/openhuman/releases/latest`. Opened when the gate blocks OAuth. |

Configure these as **GitHub Actions variables**. They must be present on **both** the standalone **`pnpm build`** step and the **`tauri-apps/tauri-action`** step env in `.github/workflows/build-desktop.yml` (the reusable matrix invoked by `release-production.yml` / `release-staging.yml`) so the Vite bundle embedded in shipped installers includes the gate. Leave `VITE_MINIMUM_SUPPORTED_APP_VERSION` **unset** for local dev (gate disabled).

Implementation: `app/src/utils/oauthAppVersionGate.ts`, `app/src/utils/desktopDeepLinkListener.ts`.

## Gmail / Google Cloud OAuth

- **Redirect URIs** in Google Cloud Console must match the **current** backend + tunnel callback paths.
- The desktop scheme (`openhuman://`) is stable; the **installed binary** must meet the minimum version when `VITE_MINIMUM_SUPPORTED_APP_VERSION` is set.

## Release checklist (avoid regressions)

1. Bump `app/package.json` and `app/src-tauri/tauri.conf.json` (and root `Cargo.toml` / core) per existing version workflows.
2. When dropping support for older installs, set **`VITE_MINIMUM_SUPPORTED_APP_VERSION`** to the new floor **before** or **with** that release (repo Actions variables + both workflow steps above).
3. Remove, redirect, or retire older stable installers and stale **updater** entries from user-facing surfaces (GitHub Release assets, website, CDN, updater feed). Confirm deprecated artifacts are not reachable from default install/update flows.
4. Smoke-test **Gmail connect** on a fresh install from **releases/latest**.
5. Complete the [manual smoke checklist](../../docs/RELEASE-MANUAL-SMOKE.md), then paste the completed sign-off block (verbatim, with every checked item left checked) into the release PR description before tagging.

## Workflows: staging vs. production

Two first-class GitHub Actions workflows, one per environment. Pick by intent rather than toggling a flag.

| Workflow                                                | Branch    | Bumps   | Tags pushed                | Concurrency group       | Use when                                                              |
| ------------------------------------------------------- | --------- | ------- | -------------------------- | ----------------------- | --------------------------------------------------------------------- |
| [`release-staging.yml`](../../.github/workflows/release-staging.yml) | `main`    | `patch` only | `v<version>-staging`        | `release-staging`       | Cutting a staging build for QA. Runs frequently; narrow semver moves. |
| [`release-production.yml`](../../.github/workflows/release-production.yml) | `main`    | `patch` / `minor` / `major` (only on `main_head`) | `v<version>`                | `release-production`    | Promoting a validated staging tag, or hotfixing from `main` HEAD.     |

The matrix build / sign / Sentry-DIF / artifact-upload pipeline used by both flows lives in [`.github/workflows/build-desktop.yml`](../../.github/workflows/build-desktop.yml) as a `workflow_call` reusable workflow. The two top-level workflows above own ref resolution, version bumping, tagging, and publish/cleanup; the build itself is shared.

### Android / Google Play

Android releases are handled by the separate [`.github/workflows/android-compile.yml`](../../.github/workflows/android-compile.yml) workflow, which builds a release Android App Bundle (`.aab`), signs it with the Play upload key, and uploads it to Google Play when publishing is enabled. The workflow keeps the unsigned and signed AABs as Actions artifacts for audit/debugging.

Manual Android uploads use the same workflow:

```bash
pnpm --dir app release:android:play -- --track internal
pnpm --dir app release:android:play -- --ref main --track production --status draft
```

Required GitHub Actions secrets:

| Secret                                | Purpose                                                                                         |
| ------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `ANDROID_UPLOAD_KEYSTORE_BASE64`      | Base64-encoded Play upload keystore (`.jks`). Use the upload key, not the Google app signing key. |
| `ANDROID_UPLOAD_KEY_ALIAS`            | Keystore alias for the upload key.                                                              |
| `ANDROID_UPLOAD_KEYSTORE_PASSWORD`    | Keystore password.                                                                              |
| `ANDROID_UPLOAD_KEY_PASSWORD`         | Key password.                                                                                   |
| `GOOGLE_PLAY_SERVICE_ACCOUNT_JSON`    | Raw JSON for the Play Console service account with release permissions for `com.openhuman.app`. |

Optional GitHub Actions variables:

| Variable              | Default     | Purpose                                                               |
| --------------------- | ----------- | --------------------------------------------------------------------- |
| `ANDROID_PLAY_TRACK`  | `internal`  | Play track to upload to (`internal`, `alpha`, `beta`, or `production`). |
| `ANDROID_PLAY_STATUS` | `completed` | Play release status (`completed`, `draft`, `inProgress`, `halted`).   |

Google Play requires each upload to use a monotonically increasing Android `versionCode`. The release bump scripts update `app/src-tauri-mobile/tauri.conf.json`, `app/src-tauri-mobile/Cargo.toml`, and `app/src-tauri-mobile/Cargo.lock` alongside desktop files so the generated Android `tauri.properties` moves with each release.

### Cutting a staging build

1. Run **Release (Staging)** via `workflow_dispatch` from `main`.
2. The workflow bumps `patch` on `main`, commits `chore(staging): vX.Y.Z`, pushes the branch, and creates an immutable `vX.Y.Z-staging` tag at that commit.
3. Build matrix runs from the **tag** (not main HEAD), so reruns rebuild byte-identical content even if `main` has moved on.
4. On failure the staging tag is auto-deleted; the bump commit on `main` stays so the next cut continues from `vX.Y.(Z+1)`.

There is no separate `staging` branch, staging cuts and production promotions both live on `main`. The two are distinguished only by tag suffix (`-staging` vs none) and by which workflow created the tag.

### Promoting to production (default flow)

1. Run **Release Production** via `workflow_dispatch` with `release_source = staging_tag` (the default).
2. Leave `staging_tag` blank to promote the latest `v*-staging`, or pass an explicit tag (e.g. `v1.2.4-staging`) to pin.
3. The workflow strips `-staging`, creates `v<version>` at the same commit, and runs the production build matrix from that tag. **No further version bump**, the artifact reuses what staging already validated.

### Hotfix from `main` HEAD

1. Run **Release Production** via `workflow_dispatch` with `release_source = main_head` and the desired `release_type` (`patch` / `minor` / `major`).
2. The run first parks on the **`review-approval`** job (`environment: Release-Approval`); a [required reviewer](#release-app-token-approval-gate-and-rotation) must approve before anything is pushed. Only after approval does `prepare-build` run the bump-and-tag path: bump on `main`, commit `chore(release): vX.Y.Z`, push, tag `vX.Y.Z`, build.
3. Use this only when a production-only fix needs to ship without going through staging.

> The `staging_tag` promotion path is **not** gated by `review-approval` — it pushes only an immutable `v<version>` tag at an already-validated staging commit, never a commit to `main`. It remains gated by the existing `Production` environment.

### Tag policy and rollback

- **Naming.** Staging tags use the SemVer pre-release suffix `-staging` (`v1.2.4-staging`) so they sort *before* the matching production tag. Promotion to production drops the suffix verbatim; the version embedded in the bundled installer is identical between the two tags.
- **Collisions.** Both workflows fail fast if the target tag already exists locally or on `origin`. Resolve by deleting the stale tag (org maintainers only) or bumping past it.
- **Rollback (production).** A failed build matrix triggers `cleanup-failed-release`, which deletes both the draft GitHub Release and the `v<version>` tag. The staging tag it was promoted from is left untouched and can be re-promoted after fixing.
- **Rollback (staging).** A failed staging build deletes the `v<version>-staging` tag. The bump commit on `main` is left in place; the next staging cut continues from the new patch number rather than re-using it (we accept a small “gap” in patch numbers over racing with concurrent merges).
- **Who can delete tags.** Same write-access as `main`. Workflow-driven cleanup deletes run with the workflow's token via `actions/github-script` (the GitHub App token is only used by `prepare-build` for the bump commit + tag push); manual deletes (`git push --delete origin <tag>`) require equivalent maintainer permissions.

## Release App token: approval gate and rotation

The `main_head` path of `release-production.yml` bumps the version, **commits to `main`**, and pushes that commit + tag with a GitHub App token (`secrets.XGITHUB_APP_ID` / `secrets.XGITHUB_APP_PRIVATE_KEY`) that **bypasses branch protection**. A leaked private key (via a log, a compromised action, or a misconfigured runner) would let an attacker push arbitrary commits to the default branch — [CWE-250: Execution with Unnecessary Privileges](https://cwe.mitre.org/data/definitions/250.html). Two controls bound that blast radius.

### Manual approval gate

The `review-approval` job runs **before** `prepare-build` and parks the run on the **`Release-Approval`** GitHub environment, so a human must approve before any push happens. It is scoped to `release_source == 'main_head'`; the `staging_tag` promotion path skips it (it pushes only a tag, never a commit to `main`).

One-time setup (repo **Settings → Environments**):

1. Create an environment named **`Release-Approval`** (exact name — the workflow references it verbatim).
2. Under **Deployment protection rules**, enable **Required reviewers** and add the maintainers allowed to authorize a production push to `main`. A reviewer **cannot** approve their own run unless _Prevent self-review_ is left off; for a release gate, keeping a second approver is preferred.
3. (Optional) Set a short **wait timer** of 0 — the gate is a human decision, not a delay.

When a `main_head` run starts it shows **"Waiting"** on the `review-approval` job; an approver opens the run and clicks **Review deployments → Approve**. Rejecting (or cancelling) leaves `prepare-build` skipped, so nothing is pushed.

### Quarterly key rotation

Rotate `XGITHUB_APP_PRIVATE_KEY` **every quarter** (and immediately on any suspected exposure). Schedule: end of **Mar / Jun / Sep / Dec**.

1. In the GitHub **App settings** (Org → Settings → Developer settings → GitHub Apps → the release App), under **Private keys** click **Generate a private key**. Download the new `.pem`.
2. Update the repo secret: **Settings → Secrets and variables → Actions → `XGITHUB_APP_PRIVATE_KEY`** → paste the full new key (including the `-----BEGIN/END-----` lines). `XGITHUB_APP_ID` is unchanged.
3. Trigger a low-risk verification run (e.g. **Release (Staging)**) and confirm the **Generate GitHub App token** step succeeds and the push authenticates.
   Do not use `main_head` for this check unless you intentionally want to cut a real bump commit and production tag — even with `create_release = false`, `prepare-build` still bumps the version, commits to `main`, and pushes the release tag.
4. Back in **App settings → Private keys**, **delete the old key** so only the freshly-issued one remains valid.
5. Record the rotation date (PR description, ops log, or `docs/OPERATIONS.md`) so the next quarter's owner can see when it last happened.

Rotating invalidates any copy of the old key that may have leaked, capping the exposure window at one quarter.
