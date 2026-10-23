---
description: How OpenHuman ships a release, with the branch model, CI lanes, minimum-version gate and key rotation.
icon: ship
---

# Release policy

This runbook covers how releases ship. It also explains how we stop users from completing OAuth (including Gmail) on an outdated desktop installer when the latest release is the supported flow.

## Distribution

- GitHub Releases for [tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman/releases) are the primary source for desktop builds.
- The Tauri updater endpoint (see `scripts/prepareTauriConfig.js` and the release workflows) should point users at the current release artifacts.
- To retire old stable artifacts when you drop a release line:
  - Remove or hide the obsolete installer assets on GitHub Releases.
  - Point website and CDN download links at `releases/latest`.
  - Refresh the updater manifest (for example a Gist or `latest.json`) so it does not point at deprecated builds.
  - Check that old direct URLs redirect or return 404 or 410. Try known-old asset URLs from docs or bookmarks and confirm they no longer deliver a primary install path.

## Minimum app version for OAuth

Production web builds embed a minimum supported app version at build time, so OAuth deep links cannot complete on deprecated binaries. Each installer carries the floor that was set when that build was made. Raising the floor for users who never upgrade needs a new release that they install, or an in-app update.

| Variable                             | Purpose                                                                                                               |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `VITE_MINIMUM_SUPPORTED_APP_VERSION` | For example `0.51.0`. The desktop app must be at least this version to finish `openhuman://oauth/success`. |
| `VITE_LATEST_APP_DOWNLOAD_URL`       | Optional; defaults to `https://github.com/tinyhumansai/openhuman/releases/latest`. Opened when the gate blocks OAuth. |

Configure these as GitHub Actions variables. Set them on both the standalone `pnpm build` step and the `tauri-apps/tauri-action` step in `.github/workflows/build-desktop.yml` (the reusable matrix that `release-production.yml` and `release-staging.yml` call), so the Vite bundle in shipped installers includes the gate. Leave `VITE_MINIMUM_SUPPORTED_APP_VERSION` unset for local development, which disables the gate.

The code is in `app/src/utils/oauthAppVersionGate.ts`, `app/src/utils/desktopDeepLinkListener.ts`.

## Gmail / Google Cloud OAuth

- Redirect URIs in Google Cloud Console must match the current backend + tunnel callback paths.
- The desktop scheme (`openhuman://`) is stable. When `VITE_MINIMUM_SUPPORTED_APP_VERSION` is set, the installed binary must meet the minimum version.

## Release checklist

1. Bump `app/package.json` and `crates/openhuman-app/tauri.conf.json` (and root `Cargo.toml` / core) per existing version workflows.
2. When dropping support for older installs, set `VITE_MINIMUM_SUPPORTED_APP_VERSION` to the new floor before or with that release (repo Actions variables + both workflow steps above).
3. Remove, redirect, or retire older stable installers and stale updater entries from user-facing surfaces (GitHub Release assets, website, CDN, updater feed). Confirm deprecated artifacts are not reachable from default install/update flows.
4. Smoke-test Gmail connect on a fresh install from releases/latest.
5. Complete the [manual smoke checklist](https://github.com/tinyhumansai/openhuman/blob/main/docs/RELEASE-MANUAL-SMOKE.md). Then paste the completed sign-off block, verbatim with every checked item left checked, as a GitHub commit comment on the `v<version>-staging` tagged commit that QA validated. The promotion flow has no release PR. Before approving the production run, the `Release-Approval` reviewer checks two things. First, the sign-off comment exists on the staging-tagged commit. Second, the production run targets that validated content: either pass the staging-tagged SHA as `commit_sha`, or confirm that only `[skip ci]` version-bump commits separate it from the run's target. Anything more is new content QA never smoked, so re-run staging first.

## Branch model and CI lanes

Two long-lived branches, two CI lanes:

- `main`: where all feature and fix PRs land. Every PR runs CI Fast ([`ci-fast.yml`](https://github.com/tinyhumansai/openhuman/blob/main/.github/workflows/ci-fast.yml)), with quality checks and full unit-test suites for changed areas, gated at 80% diff coverage or more.
- `release`: a maintainer-promoted snapshot of `main` that releases are cut from. PRs targeting `release` and every push to `release` run CI Full ([`ci-full.yml`](https://github.com/tinyhumansai/openhuman/blob/main/.github/workflows/ci-full.yml)): complete unit suites, Rust mock-backend E2E, Playwright web E2E, and desktop E2E on Linux. macOS and Windows desktop E2E are manual dispatches that default to off until a native driver exists for each, so cross-platform desktop signal is opt-in. The `CI Full Gate` check aggregates every lane except the Playwright spec run. That run is non-blocking for now (`continue-on-error`, because it is flaky under CI contention), so a green gate does not prove the Playwright specs passed. Check that lane's result before cutting. Only the Playwright artifact build is gated.

The cycle:

1. A maintainer dispatches [`promote-main-to-release.yml`](https://github.com/tinyhumansai/openhuman/blob/main/.github/workflows/promote-main-to-release.yml), which pushes a merge commit from `main` into `release` with no PR. Re-dispatching refreshes `release` with the latest `main` and keeps fix commits already on `release`. If `release` already contains `main`, it does nothing.
2. CI Full runs on the promotion push. If it finds breakage, anyone with write access opens a fix PR directly against `release`. Fix PRs run both lanes: CI Fast for quick lint and coverage feedback, and CI Full as the merge-blocking `CI Full Gate` check. The post-merge push re-runs CI Full on the merge result.
3. Once CI Full is green on `release` HEAD, cut production with `release-production.yml`. You can dispatch staging from `main` instead when QA needs to validate `main` before promotion. The release workflows do not check the `CI Full Gate`, so operators must verify the CI evidence before cutting.
4. A cut from `release` back-merges `release` into `main` with `scripts/release/merge-release-into-main.sh`. It fast-forwards when it can, and otherwise makes a versioned merge commit such as `chore(release): merge release v1.2.4 back into main`. Bump and fix commits flow back that way. A staging cut from `main` needs no back-merge. Version-bump commits carry `[skip ci]`.

Required GitHub settings for this model (repo Settings > Rules): `main` requires the `PR CI Gate` status check on PRs; `release` requires PRs for non-bypass actors with the `CI Full Gate` status check required (it runs on PRs targeting release); the release GitHub App is on the bypass list of both rulesets so the promote and release workflows can push directly.

## Workflows: staging and production

There is one GitHub Actions workflow per environment. Pick by intent instead of toggling a flag. Staging follows the selected `main` or `release` dispatch ref; production always checks out `release`, regardless of the dispatch ref shown by GitHub's workflow UI.

| Workflow                                                                   | Branch              | Bumps                                              | Tags pushed          | Concurrency group    | Use when                                                                                |
| -------------------------------------------------------------------------- | ------------------- | -------------------------------------------------- | -------------------- | -------------------- | --------------------------------------------------------------------------------------- |
| [`release-staging.yml`](https://github.com/tinyhumansai/openhuman/blob/main/.github/workflows/release-staging.yml)       | `main` or `release` | `patch` only                                       | `v<version>-staging` | `release-staging`    | Cutting a staging build for QA from the selected branch.                                |
| [`release-production.yml`](https://github.com/tinyhumansai/openhuman/blob/main/.github/workflows/release-production.yml) | `release`           | `patch` / `minor` / `major` (`release_type` input) | `v<version>`         | `release-production` | Shipping a production release from validated `release` HEAD (or a pinned `commit_sha`). |

The build, sign, Sentry debug-file and artifact-upload pipeline that both flows use lives in [`.github/workflows/build-desktop.yml`](https://github.com/tinyhumansai/openhuman/blob/main/.github/workflows/build-desktop.yml) as a reusable `workflow_call` workflow. The two workflows above own ref resolution, version bumping, tagging, and publish and cleanup. The build itself is shared.

### Android / Google Play

Android releases are handled by the separate [`.github/workflows/android-compile.yml`](https://github.com/tinyhumansai/openhuman/blob/main/.github/workflows/android-compile.yml) workflow. It builds a release Android App Bundle (`.aab`), signs it with the Play upload key, and uploads it to Google Play when publishing is enabled. The unsigned and signed AABs stay as Actions artifacts for audit and debugging.

Manual Android uploads use the same workflow:

```bash
pnpm --dir app release:android:play -- --track internal
pnpm --dir app release:android:play -- --ref main --track production --status draft
```

Required GitHub Actions secrets:

| Secret                             | Purpose                                                                                           |
| ---------------------------------- | ------------------------------------------------------------------------------------------------- |
| `ANDROID_UPLOAD_KEYSTORE_BASE64`   | Base64-encoded Play upload keystore (`.jks`). Use the upload key, not the Google app signing key. |
| `ANDROID_UPLOAD_KEY_ALIAS`         | Keystore alias for the upload key.                                                                |
| `ANDROID_UPLOAD_KEYSTORE_PASSWORD` | Keystore password.                                                                                |
| `ANDROID_UPLOAD_KEY_PASSWORD`      | Key password.                                                                                     |
| `GOOGLE_PLAY_SERVICE_ACCOUNT_JSON` | Raw JSON for the Play Console service account with release permissions for `com.openhuman.app`.   |

Optional GitHub Actions variables:

| Variable              | Default     | Purpose                                                                 |
| --------------------- | ----------- | ----------------------------------------------------------------------- |
| `ANDROID_PLAY_TRACK`  | `internal`  | Play track to upload to (`internal`, `alpha`, `beta`, or `production`). |
| `ANDROID_PLAY_STATUS` | `completed` | Play release status (`completed`, `draft`, `inProgress`, `halted`).     |

Google Play requires each upload to use a higher Android `versionCode` than the last. The release bump scripts update `app/src-tauri-mobile/tauri.conf.json`, `app/src-tauri-mobile/Cargo.toml` and `app/src-tauri-mobile/Cargo.lock` with the desktop files, so the generated Android `tauri.properties` moves with each release.

### Cutting a staging build

1. Run Release (Staging) via `workflow_dispatch` from `release` (optionally pinning a release-reachable `commit_sha`). `create_tag = false` bumps and commits without tagging or building.
2. The workflow bumps `patch` on `release`, commits `chore(staging): vX.Y.Z [skip ci]`, pushes, and creates an immutable `vX.Y.Z-staging` tag at that commit.
3. The build matrix runs from the tag, not `release` HEAD, so reruns rebuild identical content even if `release` has moved on.
4. The bump commit (and anything else on `release`) is merged back into `main`.
5. On failure the staging tag is deleted automatically. The bump commit on `release` stays, so the next cut continues from `vX.Y.(Z+1)`.

There is no separate `staging` branch. Staging cuts and production releases both live on `release`, and only the tag suffix (`-staging` or none) and the workflow that made the tag tell them apart.

### Shipping a production release

1. Run Release Production via `workflow_dispatch` with the desired `release_type` (`patch` / `minor` / `major`), from `release` HEAD or a pinned release-reachable `commit_sha`.
2. The run first waits on the `review-approval` job (`environment: Release-Approval`). A [required reviewer](#release-app-token-approval-gate-and-rotation) must approve before anything is pushed. Then `prepare-build` bumps the version on `release`, commits `chore(release): vX.Y.Z [skip ci]`, pushes, tags `vX.Y.Z`, builds and publishes.
3. `release` is merged back into `main` right after the cut.

### Tag policy and rollback

- Naming: staging tags use the SemVer pre-release suffix `-staging` (`v1.2.4-staging`), so they sort before the matching production tag.
- Collisions: both workflows fail fast if the target tag already exists locally or on `origin`. Delete the stale tag (org maintainers only) or bump past it.
- Production rollback: a failed build matrix triggers `cleanup-failed-release`, which deletes the draft GitHub Release and the `v<version>` tag.
- Staging rollback: a failed staging build deletes the `v<version>-staging` tag. The bump commit on the source branch stays. The next staging cut continues from the new patch number, which leaves a small gap in patch numbers instead of racing concurrent merges.
- Deleting tags: you need the same write access as `main`. Workflow cleanup runs with the workflow's token through `actions/github-script`. The GitHub App token is used only by `prepare-build` for the bump commit and tag push. Manual deletes (`git push --delete origin <tag>`) need equivalent maintainer permissions.

## Release App token: approval gate and rotation

`release-production.yml` bumps the version, commits to `release` and back-merges into `main`, pushing those commits and the tag with a GitHub App token (`secrets.XGITHUB_APP_ID` / `secrets.XGITHUB_APP_PRIVATE_KEY`) that bypasses branch protection. The same App pushes staging bumps to the selected `main` or `release` source (`release-staging.yml`) and promotion merge commits (`promote-main-to-release.yml`). A leaked private key (through a log, a compromised action or a misconfigured runner) would let an attacker push arbitrary commits to protected branches ([CWE-250](https://cwe.mitre.org/data/definitions/250)). Two controls limit the damage.

### Manual approval gate

The `review-approval` job runs before `prepare-build` and parks every production run on the `Release-Approval` GitHub environment, so a human must approve before any push happens.

One-time setup (repo Settings > Environments):

1. Create an environment named `Release-Approval` (the exact name, because the workflow references it verbatim).
2. Under Deployment protection rules, enable Required reviewers and add the maintainers allowed to authorize a production push to `main`. A reviewer cannot approve their own run unless Prevent self-review is off. For a release gate, keep it on so a second person approves.
3. Optionally set the wait timer to 0. The gate is a human decision, not a delay.

When a production run starts, the `review-approval` job shows "Waiting". An approver opens the run and clicks Review deployments > Approve. Rejecting or cancelling skips `prepare-build`, so nothing is pushed.

### Quarterly key rotation

Rotate `XGITHUB_APP_PRIVATE_KEY` every quarter (and immediately on any suspected exposure). Do it at the end of March, June, September and December.

1. In the GitHub App settings (Org > Settings > Developer settings > GitHub Apps > the release App), under Private keys click Generate a private key. Download the new `.pem`.
2. Update the repo secret: Settings > Secrets and variables > Actions > `XGITHUB_APP_PRIVATE_KEY`, then paste the full new key (including the `-----BEGIN/END-----` lines). `XGITHUB_APP_ID` is unchanged.
3. Trigger a low-risk verification run, such as Release (Staging), and confirm the Generate GitHub App token step succeeds and the push authenticates. Do not use Release Production for this unless you mean to cut a real bump commit. Even with `create_release = false`, `prepare-build` still bumps the version and commits to `release`. Staging with `create_tag = false` also commits but skips the tag and build, so it is the lower-risk probe.
4. Back in the App settings under Private keys, delete the old key so only the freshly-issued one remains valid.
5. Record the rotation date in the PR description or the ops log so the next owner can see when it last happened.

Rotating invalidates any leaked copy of the old key, which caps the exposure window at one quarter.
