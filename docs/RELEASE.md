# Releases

OpenHuman ships through two distinct GitHub Actions workflows. Staging
proves a build; production promotes it.

| Workflow                 | File                                                                                | Trigger                         | Tags it creates                                                | Cadence                            |
| ------------------------ | ----------------------------------------------------------------------------------- | ------------------------------- | -------------------------------------------------------------- | ---------------------------------- |
| **Release (Staging)**    | [`.github/workflows/release-staging.yml`](../.github/workflows/release-staging.yml) | `workflow_dispatch` from `main` | `staging/vX.Y.Z-N` (immutable, monotonic `N` per base `X.Y.Z`) | Frequent (per-merge / per-feature) |
| **Release** (production) | [`.github/workflows/release.yml`](../.github/workflows/release.yml)                 | `workflow_dispatch` from `main` | `vX.Y.Z` (semver)                                              | On promotion                       |

The two workflows have **separate concurrency groups** (`release-staging`
vs `release-production-main`), so a staging cut never blocks a production
release and vice versa.

---

## Cutting a staging build

1. Run **Release (Staging)** via the Actions tab.
2. The workflow:
   - reads the current base version from `app/package.json` (it does **not**
     mutate any version files),
   - counts existing `staging/v<base>-*` tags and picks the next `N`,
   - pushes the immutable tag `staging/vX.Y.Z-N`,
   - builds the desktop matrix in **debug profile**, uploads installers as
     workflow artifacts (no GitHub Release object is created), and
   - records a Sentry deploy marker with `environment=staging`.
3. If any matrix leg fails, the `cleanup-failed-staging` job removes the
   freshly-pushed staging tag so production can never resolve to a
   half-built artifact.

**Why patch-only / counter-only:** keeping staging cadence noise-free.
No version-file commits land on `main` from staging — the tag itself is
the stable version identity. Production decides its own semver bump
separately when promoting.

## Promoting to production

1. Run **Release** via the Actions tab.
2. Pick `release_source`:
   - `staging_tag` (default) — build from the **last QA-validated staging
     cut**. Leave `staging_tag` empty to auto-resolve the latest
     `staging/v*` tag (`git tag --sort=-creatordate`), or pin a specific
     one (e.g. `staging/v0.53.4-2`).
   - `main_head` — escape hatch for hotfixes when no staging tag fits.
     Builds from `origin/main` HEAD.
3. Pick `release_type` (`patch` | `minor` | `major`). Production owns
   `minor` and `major` semver promotion; `patch` is allowed for
   hotfixes.
4. The workflow:
   - resolves the source ref (logged via
     [`scripts/release/resolve-release-source.sh`](../scripts/release/resolve-release-source.sh)
     for traceability — the bump commit message records `source`, `ref`,
     and `sha`),
   - bumps versions in `app/package.json`, `app/src-tauri/tauri.conf.json`,
     `app/src-tauri/Cargo.toml`, and root `Cargo.toml`,
   - commits, pushes, and tags `vX.Y.Z` on `main`,
   - builds the matrix in **release profile**, signs and notarizes the
     macOS bundles, builds the GHCR Docker image, publishes
     `latest.json` for the Tauri auto-updater, and publishes the GitHub
     Release.
5. If any later phase fails, `cleanup-failed-release` deletes the draft
   release, the remote tag, and the staging Docker image.

## Tag conventions and rollback

- **Production tags:** `vX.Y.Z` (e.g. `v0.53.4`). Never delete a
  published production tag — the auto-updater clients pin to it.
- **Staging tags:** `staging/vX.Y.Z-N` (e.g. `staging/v0.53.4-2`).
  - Auto-deleted by `cleanup-failed-staging` when their build fails.
  - May be deleted manually by a maintainer with `contents: write` if a
    staging cut is later invalidated. Production's resolver uses
    `--sort=-creatordate`, so dropping a tag promotes the next-latest.
  - Tag collisions abort the staging workflow before the build matrix
    starts (see `Ensure staging tag does not already exist on remote`).

## Helper scripts

| Script                                                                                      | Role                                                                           |
| ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| [`scripts/release/next-staging-tag.js`](../scripts/release/next-staging-tag.js)             | Compute next `staging/vX.Y.Z-N` tag from `app/package.json` + existing tags    |
| [`scripts/release/resolve-release-source.sh`](../scripts/release/resolve-release-source.sh) | Resolve `release_source` + optional `staging_tag` to a concrete (`ref`, `sha`) |
| [`scripts/release/bump-version.js`](../scripts/release/bump-version.js)                     | Bump semver across all four authoritative version files                        |
| [`scripts/release/verify-version-sync.js`](../scripts/release/verify-version-sync.js)       | Assert version sync after a bump                                               |
