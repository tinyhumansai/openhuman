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
5. Complete the [manual smoke checklist](../../docs/RELEASE-MANUAL-SMOKE.md), then paste the completed sign-off block (verbatim, with every checked item left checked) as a **GitHub commit comment on the `v<version>-staging` tagged commit** that QA validated (there is no release PR in the promotion flow). Before approving the production run, the `Release-Approval` required reviewer verifies (a) the sign-off comment exists on the staging-tagged commit and (b) the production run actually targets that validated content — pass the staging-tagged SHA as `commit_sha`, or confirm nothing but `[skip ci]` version-bump commits separate it from the run's target. Anything more than bump commits means new content QA never smoked: re-run staging first.

## Branch model and CI lanes

Two long-lived branches, two CI lanes:

- **`main`** — where all feature/fix PRs land. Every PR (and push to main) runs **CI Lite** ([`ci-lite.yml`](../../.github/workflows/ci-lite.yml)): quality checks per changed area plus unit tests scoped to the changed files, gated at ≥ 80% diff coverage by the `PR CI Gate` check.
- **`release`** — a maintainer-promoted snapshot of `main` that releases are cut from. PRs targeting `release` and every push to `release` run **CI Full** ([`ci-full.yml`](../../.github/workflows/ci-full.yml)): complete unit suites, Rust mock-backend E2E, Playwright web E2E, and the full desktop E2E matrix on Linux/macOS/Windows. The `CI Full Gate` check aggregates every lane **except the Playwright spec run**, which is non-blocking signal for now (`continue-on-error`, flaky under CI contention — #3615): a green gate does not prove Playwright specs passed, so check that lane's result in the run before cutting. Only the Playwright artifact *build* is gated.

The cycle:

1. A maintainer dispatches [`promote-main-to-release.yml`](../../.github/workflows/promote-main-to-release.yml), which pushes a **merge commit from `main` into `release`** (no PR). Re-dispatching refreshes `release` with main's latest while preserving fix commits already on `release`; when `release` already contains `main` it's a no-op.
2. CI Full runs on the promotion push. If it finds breakage, anyone with write access opens a **fix PR directly against `release`**; fix PRs run both lanes — CI Lite for quick lint/coverage feedback and CI Full as the merge-blocking `CI Full Gate` check — and the post-merge push re-runs CI Full on the merge result.
3. Once CI Full is green on `release` HEAD, cut production with `release-production.yml`. Staging may instead be dispatched from `main` when QA needs to validate main before promotion. The release workflows do not query or enforce the `CI Full Gate`; operators verify the relevant CI evidence before cutting.
4. A cut sourced from `release` back-merges `release` into `main` (`scripts/release/merge-release-into-main.sh`: fast-forward when possible, else a versioned merge commit such as `chore(release): merge release v1.2.4 back into main`), so bump commits and fix commits flow back. A staging cut sourced from `main` needs no back-merge. Version-bump commits carry `[skip ci]`.

Required GitHub settings for this model (repo **Settings → Rules**): `main` requires the `PR CI Gate` status check on PRs; `release` requires PRs for non-bypass actors with the `CI Full Gate` status check required (it runs on PRs targeting release); the release GitHub App's identity sits on the bypass list of both rulesets so the promote/release workflows can push directly.

## Workflows: staging vs. production

Two first-class GitHub Actions workflows, one per environment. Pick by intent rather than toggling a flag. Staging follows the selected `main` or `release` dispatch ref; production always checks out `release`, regardless of the dispatch ref shown by GitHub's workflow UI.

| Workflow                                                | Branch    | Bumps   | Tags pushed                | Concurrency group       | Use when                                                              |
| ------------------------------------------------------- | --------- | ------- | -------------------------- | ----------------------- | --------------------------------------------------------------------- |
| [`release-staging.yml`](../../.github/workflows/release-staging.yml) | `main` or `release` | `patch` only | `v<version>-staging`        | `release-staging`       | Cutting a staging build for QA from the selected branch. |
| [`release-production.yml`](../../.github/workflows/release-production.yml) | `release` | `patch` / `minor` / `major` (`release_type` input) | `v<version>`                | `release-production`    | Shipping a production release from validated `release` HEAD (or a pinned `commit_sha`). |

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

1. Run **Release (Staging)** via `workflow_dispatch` from `release` (optionally pinning a release-reachable `commit_sha`; `create_tag = false` bumps + commits without tagging or building).
2. The workflow bumps `patch` on `release`, commits `chore(staging): vX.Y.Z [skip ci]`, pushes, and creates an immutable `vX.Y.Z-staging` tag at that commit.
3. Build matrix runs from the **tag** (not release HEAD), so reruns rebuild byte-identical content even if `release` has moved on.
4. The bump commit (and anything else on `release`) is merged back into `main`.
5. On failure the staging tag is auto-deleted; the bump commit on `release` stays so the next cut continues from `vX.Y.(Z+1)`.

There is no separate `staging` branch — staging cuts and production releases both live on `release`. The two are distinguished only by tag suffix (`-staging` vs none) and by which workflow created the tag.

### Shipping a production release

1. Run **Release Production** via `workflow_dispatch` with the desired `release_type` (`patch` / `minor` / `major`), from `release` HEAD or a pinned release-reachable `commit_sha`.
2. The run first parks on the **`review-approval`** job (`environment: Release-Approval`); a [required reviewer](#release-app-token-approval-gate-and-rotation) must approve before anything is pushed. Only after approval does `prepare-build` run the bump-and-tag path: bump on `release`, commit `chore(release): vX.Y.Z [skip ci]`, push, tag `vX.Y.Z`, build, publish.
3. `release` is merged back into `main` right after the cut.

### Tag policy and rollback

- **Naming.** Staging tags use the SemVer pre-release suffix `-staging` (`v1.2.4-staging`) so they sort *before* the matching production tag.
- **Collisions.** Both workflows fail fast if the target tag already exists locally or on `origin`. Resolve by deleting the stale tag (org maintainers only) or bumping past it.
- **Rollback (production).** A failed build matrix triggers `cleanup-failed-release`, which deletes both the draft GitHub Release and the `v<version>` tag.
- **Rollback (staging).** A failed staging build deletes the `v<version>-staging` tag. The bump commit on the selected source branch is left in place; the next staging cut continues from the new patch number rather than re-using it (we accept a small “gap” in patch numbers over racing with concurrent merges).
- **Who can delete tags.** Same write-access as `main`. Workflow-driven cleanup deletes run with the workflow's token via `actions/github-script` (the GitHub App token is only used by `prepare-build` for the bump commit + tag push); manual deletes (`git push --delete origin <tag>`) require equivalent maintainer permissions.

## Release App token: approval gate and rotation

`release-production.yml` bumps the version, **commits to `release` and back-merges into `main`**, pushing those commits + tag with a GitHub App token (`secrets.XGITHUB_APP_ID` / `secrets.XGITHUB_APP_PRIVATE_KEY`) that **bypasses branch protection**. The same App pushes staging bumps to the selected `main` or `release` source (`release-staging.yml`) and promotion merge commits (`promote-main-to-release.yml`). A leaked private key (via a log, a compromised action, or a misconfigured runner) would let an attacker push arbitrary commits to protected branches ([CWE-250: Execution with Unnecessary Privileges](https://cwe.mitre.org/data/definitions/250.html)). Two controls bound that blast radius.

### Manual approval gate

The `review-approval` job runs **before** `prepare-build` and parks every production run on the **`Release-Approval`** GitHub environment, so a human must approve before any push happens.

One-time setup (repo **Settings → Environments**):

1. Create an environment named **`Release-Approval`** (exact name: the workflow references it verbatim).
2. Under **Deployment protection rules**, enable **Required reviewers** and add the maintainers allowed to authorize a production push to `main`. A reviewer **cannot** approve their own run unless _Prevent self-review_ is left off; for a release gate, keeping a second approver is preferred.
3. (Optional) Set a short **wait timer** of 0. The gate is a human decision, not a delay.

When a production run starts it shows **"Waiting"** on the `review-approval` job; an approver opens the run and clicks **Review deployments → Approve**. Rejecting (or cancelling) leaves `prepare-build` skipped, so nothing is pushed.

### Quarterly key rotation

Rotate `XGITHUB_APP_PRIVATE_KEY` **every quarter** (and immediately on any suspected exposure). Schedule: end of **Mar / Jun / Sep / Dec**.

1. In the GitHub **App settings** (Org → Settings → Developer settings → GitHub Apps → the release App), under **Private keys** click **Generate a private key**. Download the new `.pem`.
2. Update the repo secret: **Settings → Secrets and variables → Actions → `XGITHUB_APP_PRIVATE_KEY`** → paste the full new key (including the `-----BEGIN/END-----` lines). `XGITHUB_APP_ID` is unchanged.
3. Trigger a low-risk verification run (e.g. **Release (Staging)**) and confirm the **Generate GitHub App token** step succeeds and the push authenticates.
   Do not use **Release Production** for this check unless you intentionally want to cut a real bump commit: even with `create_release = false`, `prepare-build` still bumps the version and commits to `release` (staging with `create_tag = false` does too, but skips the tag/build and is the lower-risk probe).
4. Back in **App settings → Private keys**, **delete the old key** so only the freshly-issued one remains valid.
5. Record the rotation date (PR description, ops log, or `docs/OPERATIONS.md`) so the next quarter's owner can see when it last happened.

Rotating invalidates any copy of the old key that may have leaked, capping the exposure window at one quarter.
