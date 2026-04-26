# Sentry Release Tracking & Source Maps

_Tracks issue [#405](https://github.com/tinyhumansai/openhuman/issues/405)._

OpenHuman reports crashes and errors from two surfaces that must group under
a **single Sentry release** so a new deploy's regressions are easy to see:

- **Frontend** — `@sentry/react` in `app/src/services/analytics.ts`.
- **Rust core sidecar** — `sentry::init` in `src/main.rs`.

The Tauri shell binary (`app/src-tauri`) has no Sentry wiring today.

## Canonical release identifier

Both surfaces report the **same** release tag:

```
openhuman@<semver>+<short_git_sha>
```

Where:

- `<semver>` is `packageJson.version` / `env!("CARGO_PKG_VERSION")`.
- `<short_git_sha>` is the first 12 chars of the commit that produced the
  build. When the SHA is absent (local dev), the tag collapses to
  `openhuman@<semver>` with no `+` suffix.

The frontend computes this in `app/src/utils/config.ts::SENTRY_RELEASE`
from `VITE_BUILD_SHA`. The core does the same in
`src/main.rs::build_release_tag()` from `option_env!("OPENHUMAN_BUILD_SHA")`.

## Environments

Reported as the Sentry `environment` tag:

| Value         | When                                                            |
| ------------- | --------------------------------------------------------------- |
| `development` | Local `pnpm tauri dev` / debug builds                            |
| `staging`     | `VITE_OPENHUMAN_APP_ENV=staging` or `OPENHUMAN_APP_ENV=staging`  |
| `production`  | Release builds from `workflow_dispatch` with `build_target=production` |

Fallback precedence for the core:

1. `OPENHUMAN_APP_ENV` env var at runtime (override).
2. Compile-time `debug_assertions` → `development`.
3. Otherwise → `production`.

## Source-map upload

The frontend emits source maps (`vite.config.ts` sets `build.sourcemap =
true`). When `SENTRY_AUTH_TOKEN` is present at build time
`@sentry/vite-plugin`:

1. Uploads every `dist/**/*.js` and its `.map` sibling.
2. Tags the upload with the canonical release name above.
3. **Deletes the on-disk `.map` files after upload** so users never receive
   them in the shipped bundle.

If `SENTRY_AUTH_TOKEN` is empty (local dev, smoke CI, forks without
secrets), the plugin registers as a no-op — the build still produces source
maps on disk but nothing is uploaded. This keeps the local dev loop zero-
config.

## CI configuration

`release.yml` + `release-packages.yml` thread the following through to the
build steps. Any subset can be set on a per-environment basis in the
`Production` / `Staging` GitHub Actions environment:

### Required for upload to work

| Name                                  | Type     | Scope           | Purpose                                       |
| ------------------------------------- | -------- | --------------- | --------------------------------------------- |
| `secrets.SENTRY_AUTH_TOKEN`           | secret   | build-desktop   | Auth for `@sentry/vite-plugin` uploads        |
| `vars.SENTRY_ORG`                     | variable | build-desktop   | Sentry org slug                                |
| `vars.SENTRY_PROJECT_FRONTEND`        | variable | build-desktop   | Sentry project slug for the frontend bundle   |
| `vars.OPENHUMAN_SENTRY_DSN`           | variable | build-desktop   | Core sidecar DSN (baked via `option_env!`)    |
| `vars.VITE_SENTRY_DSN`                | variable | build-desktop   | Frontend DSN (baked by Vite define)           |

### Provided automatically

| Name                     | Source                                           |
| ------------------------ | ------------------------------------------------ |
| `VITE_BUILD_SHA`         | `needs.prepare-build.outputs.sha` (tag commit)    |
| `OPENHUMAN_BUILD_SHA`    | Same — passed to `cargo build` for the sidecar    |
| `SENTRY_RELEASE`         | `openhuman@<version>+<sha>` — same on both steps |

### Personal Sentry DSN (local)

Drop the DSN into your repo-local `.env`:

```sh
# .env
OPENHUMAN_SENTRY_DSN=https://<key>@o<org>.ingest.sentry.io/<project>
```

`src/main.rs` now loads `.env` **before** `sentry::init`, so the runtime
env var is visible to the client at startup without needing a manual
`source scripts/load-dotenv.sh`.

For the frontend, put `VITE_SENTRY_DSN` in `app/.env.local`.

## Verification runbook

1. **Event arrives**. Trigger a test event from the core CLI:
   ```sh
   ./target/release/openhuman-core sentry-test
   # or on an installed release (Windows):
   #   "%LOCALAPPDATA%\Programs\OpenHuman\OpenHuman.exe" core sentry-test
   # or (macOS):
   #   /Applications/OpenHuman.app/Contents/MacOS/openhuman-core-* sentry-test
   ```
   The command prints an event UUID on success; search it in the Sentry
   dashboard.
2. **Release tag is right**. On the event detail page, the `Release` field
   should read `openhuman@<version>+<short_sha>` (matching the tag that cut
   the release).
3. **Environment tag is right**. Production CI dispatch → `production`.
   Staging dispatch → `staging`. Local `pnpm tauri dev` → `development`.
4. **Stack traces are symbolicated**. Force a frontend error from the
   installed app; the event's stack trace should show original
   TypeScript file names and line numbers (not hashed `assets/index-*.js`).
5. **CI failure is loud when misconfigured**. If `SENTRY_AUTH_TOKEN` is
   missing and the release is supposed to upload source maps, the CI run
   will warn in the Vite build log rather than silently producing an
   un-symbolicated release.

## Troubleshooting

- **Events arrive without a release tag** — check the Vite build log for
  `SENTRY_RELEASE`; if empty, the CI workflow didn't pass it through.
- **Events arrive without symbolication** — open the release in Sentry →
  "Source Maps" tab. Missing artifacts mean either `SENTRY_AUTH_TOKEN` was
  empty, or the plugin ran but the `assets:` glob didn't match (inspect the
  upload summary printed during `pnpm build`).
- **Frontend and core show different releases** — verify
  `needs.prepare-build.outputs.sha` is identical between the core build
  step (`OPENHUMAN_BUILD_SHA`) and the frontend build step
  (`VITE_BUILD_SHA` / `SENTRY_RELEASE`).
- **No events from a release build, only from local** — `vars.*` probably
  isn't defined on the `Production` environment. Set it and re-cut the
  release.
