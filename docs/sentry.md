# Sentry Release Tracking & Source Maps

_Tracks issue [#405](https://github.com/tinyhumansai/openhuman/issues/405)._

OpenHuman reports crashes and errors from three surfaces, each with its own
Sentry project but all sharing the **same release tag** so events line up:

- **Frontend** — `@sentry/react` in `app/src/services/analytics.ts` →
  `openhuman-react`.
- **Rust core (CLI / Docker)** — `sentry::init` in `src/main.rs` →
  `openhuman-core`.
- **Tauri shell (desktop)** — `sentry::init` in
  `app/src-tauri/src/lib.rs::run()` → `openhuman-tauri`. The core is
  linked into this binary as a path dep, so a single `cargo tauri build`
  produces all the Rust DIFs uploaded for both projects.

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

## Rust debug symbols + source context

`scripts/upload_sentry_symbols.sh` runs after the Tauri build in
`release.yml` and pushes:

- **Debug info files** (`.dwp` / `.debug` / `.pdb` / macOS `.dSYM`) found
  under `app/src-tauri/target/<triple>/release/deps`. The
  `[profile.release] debug = "line-tables-only"` setting in both
  `Cargo.toml` files emits just enough DWARF (file+line tables, no full
  type info) for Sentry to symbolicate frame addresses without bloating
  the shipped binary. `split-debuginfo = "packed"` writes the debug data
  into a separate `.dSYM` bundle on macOS.
- **A `.src.zip` source bundle** built from the Rust source files
  referenced by those DIFs (`sentry-cli upload-dif --include-sources`).
  This is what lets Sentry render the surrounding lines of source for a
  panic, not just `function_name + 0xNNN`. Without it, the event detail
  page shows a symbolicated stack with empty source context.

For Sentry to actually walk the loaded shared libraries at runtime and
attach each image's debug-id to events, the `sentry` crate is built with
the `debug-images` feature in both `Cargo.toml` files. This registers
`DebugImagesIntegration` as part of the default integration set — events
arrive with `debug_meta.images` populated, and Sentry's symbolicator
resolves those debug-ids against uploaded DIFs to attach `pre_context` /
`context_line` / `post_context` to each frame.

The script drives the per-project release lifecycle for the project it's
called against:

1. `sentry-cli releases new "$SENTRY_RELEASE"` — creates / no-ops the release.
2. `sentry-cli releases set-commits --auto --ignore-missing` — associates
   commits using the GitHub-provided range. `--ignore-missing` keeps shallow
   CI checkouts from failing.
3. `sentry-cli upload-dif --include-sources` — DIFs + `.src.zip`.
4. `sentry-cli releases finalize "$SENTRY_RELEASE"` — marks the release
   complete (used by Sentry to compute "regression" / "new in release").

`releases new`, `set-commits`, and `finalize` are idempotent — re-running
on the same SHA reuses the existing release and DIFs are deduplicated by
debug-ID. The deploy marker is **not** in this script — it lives in
`release.yml`'s "Record Sentry deploy marker" step, which fires once per
matrix target after the upload step. `sentry-cli releases deploys ... new`
does not deduplicate by (release, env), so re-running CI for the same
release intentionally adds another deploy row representing a separate
deploy attempt.

## CI configuration

`release.yml` + `release-packages.yml` thread the following through to the
build steps. Any subset can be set on a per-environment basis in the
`Production` / `Staging` GitHub Actions environment:

### Required for upload to work

| Name                                  | Type     | Scope                  | Purpose                                                |
| ------------------------------------- | -------- | ---------------------- | ------------------------------------------------------ |
| `secrets.SENTRY_AUTH_TOKEN`           | secret   | build-desktop          | Auth for `@sentry/vite-plugin` + `sentry-cli`          |
| `vars.SENTRY_ORG`                     | variable | build-desktop          | Sentry org slug                                        |
| `vars.SENTRY_PROJECT_REACT`           | variable | build-desktop (Vite)   | Project slug for the frontend bundle + source maps     |
| `vars.SENTRY_PROJECT_CORE`            | variable | symbols-upload         | Project slug for the Rust DIFs + source bundle         |
| `vars.SENTRY_PROJECT_TAURI`           | variable | (reserved)             | Reserved for the Tauri shell when symbol-uploads split |
| `vars.OPENHUMAN_REACT_SENTRY_DSN`     | variable | build-desktop (Vite)   | Frontend DSN (baked by Vite define)                    |
| `vars.OPENHUMAN_CORE_SENTRY_DSN`      | variable | build-desktop (Rust)   | Core sidecar DSN (baked via `option_env!`)             |
| `vars.OPENHUMAN_TAURI_SENTRY_DSN`     | variable | build-desktop (Tauri)  | Tauri shell DSN (baked via `option_env!`)              |

The legacy `vars.OPENHUMAN_SENTRY_DSN`, `vars.VITE_SENTRY_DSN`,
`vars.SENTRY_PROJECT`, and `vars.SENTRY_PROJECT_FRONTEND` are no longer
read by `release.yml` — they were superseded by the per-surface variables
above as part of #1032. Safe to delete from any configured GitHub Actions
environment that still has them set.

### Provided automatically

| Name                     | Source                                                              |
| ------------------------ | ------------------------------------------------------------------- |
| `VITE_BUILD_SHA`         | `needs.prepare-build.outputs.sha` (tag commit, full 40 chars)        |
| `OPENHUMAN_BUILD_SHA`    | Same — passed to `cargo build` for the Rust core / Tauri shell       |
| `SENTRY_RELEASE`         | `openhuman@<version>+<short_sha>` — `short_sha` is `sha[:12]`, matches the truncation `config.ts` / `vite.config.ts` / `main.rs` / `app/src-tauri/src/lib.rs` apply at runtime. Same value on Vite, symbols upload, and the deploy-marker steps |
| `SENTRY_ENVIRONMENT`     | `staging` / `production` from the workflow's `build_target` — only consumed by the deploy-marker step |

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
- **Rust frames show function name but no source** — the `.src.zip` for
  this release didn't upload, OR the `debug-images` integration isn't
  active. Check the "Upload core sidecar debug symbols to Sentry" workflow
  log for `Bundled N source files`; absence means `--include-sources`
  didn't take effect or DWARF wasn't emitted (verify the
  `[profile.release] debug = "line-tables-only"` block in `Cargo.toml`).
  If the bundle uploaded but events still render blank, confirm the
  `sentry` crate has the `debug-images` feature enabled in both
  `Cargo.toml` files.
- **DIFs uploaded but events still report a release with no artifacts**
  — verify `SENTRY_RELEASE` was set to `openhuman@<version>+<short_sha>`
  in all three places that construct it (Vite build step, symbols-upload
  step, deploy-marker step). All three must reference
  `needs.prepare-build.outputs.short_sha`, not the full `sha`.
- **No deploy marker on the release page** — confirm the dedicated
  "Record Sentry deploy marker" step ran and `SENTRY_ENVIRONMENT`
  resolved to a non-empty value (`release.yml` derives it from
  `inputs.build_target`).
