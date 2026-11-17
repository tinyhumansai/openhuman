# `scripts/`

Repo-maintenance, CI, dev-loop, and release tooling. This is a map, not a
manual, each entry point below has its own header comment or README with the
details.

Standalone Embed source bundles: [`bootstrap-embed-consumer.py`](bootstrap-embed-consumer.py) verifies a commit pin, prepares submodules and generates a consumer Cargo workspace; see [consumer setup](../crates/openhuman-embed/CONSUMERS.md).

## Sub-directories

The first five have their own README; the rest are documented by the header
comments of the scripts inside them. Benchmark, profiling and stress-test
drivers live in
[tinyhumansai/openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks).

| Directory | Purpose |
| --- | --- |
| [`ci/`](ci/README.md) | Merge-gate scripts (layout, feature forwarding, module pins, coverage, toolchain drift). |
| [`debug/`](debug/README.md) | Agent-friendly wrappers around the test runners, run via `pnpm debug ...`. |
| [`rabbit/`](rabbit/README.md) | Auto-retriggers CodeRabbit reviews once a PR's rate-limit window elapses. |
| [`shortcuts/`](shortcuts/README.md) | High-level pnpm workflow commands (`pnpm review`, `pnpm work`, `pnpm reset`, ...). |
| [`test-planning/`](test-planning/README.md) | Mines GitHub issues/PRs into a test-planning backlog via an LLM CLI. |
| [`release/`](release) | Release build, signing, packaging, and publishing scripts (macOS notarization, APT/Homebrew, npm, updater manifests). |
| [`mock-api/`](mock-api) | The shared mock backend modules (routes, socket, admin, state) behind [`mock-api-server.mjs`](mock-api-server.mjs) / [`mock-api-core.mjs`](mock-api-core.mjs); exercised by `pnpm mock:api` and Rust/E2E test runs. |
| [`modules/`](modules) | `build.mjs <id\|all>` builds a loadable module from its vendored gitlink for local development (`SourceSpec` in the registry record; artifact plus an operator-trust `modules.toml` under `target/module-sources/<id>/`, cached by submodule tree, rustc, target, profile and features). `stage-local-modules.sh` is the helper `test-rust-with-mock.sh` and `test-rust-e2e.sh` source. With `OPENHUMAN_MODULE_SOURCE=source` those runners build modules from source; without it they use the pinned releases, same as CI. CI never calls `build.mjs`. `pin-status.mjs` is the local, advisory view of the two module pins (in sync, ahead, behind/diverged, exempt; `--print-env` emits `OPENHUMAN_MODULE_PIN_AHEAD`, which a debug core reads to warn once per id when it serves such a module; `.husky/pre-push` runs it as a hint when `vendor/**` changed). `release-and-repin.mjs <id> [--bump patch\|minor\|major] [--version X.Y.Z] [--dry-run]` dispatches the module's `Release` workflow after asking, then rewrites the registry record verbatim from the release's `checksum.toml` (never hashing anything), checks out the tag, stages the gitlink and runs the strict pin gates. |
| [`lib/`](lib) | Small parsers shared by the CI gates (checklist, coverage matrix, feature forwarding, module pins, module source specs). |
| [`__tests__/`](__tests__), [`tests/`](tests) | Unit tests for the scripts themselves (`node --test`, plus a PowerShell installer test). |
| [`fixtures/`](fixtures) | `latest.json` / `release.json` release-metadata fixtures consumed by [`test_install.sh`](test_install.sh) (the [`install.sh`](install.sh) test). |
| [`theme-codemod/`](theme-codemod) | Codemod collapsing audited `light dark:` Tailwind pairings into the semantic theme utilities (`node scripts/theme-codemod/migrate.mjs [--write]`; see [`docs/gitbooks/en/developing/theming.md`](../docs/gitbooks/en/developing/theming.md)). |
| [`agent-batch/`](agent-batch) | Validates a batch spec of parallel agent branches, checks file overlap, prints per-agent launch prompts, and reports status (`pnpm agent-batch <validate\|overlap\|launch\|status> <spec.json>`). |
| [`deep-work/`](deep-work) | Issue-to-PR workflow automation over worktrees and AI agents: `pnpm deep-work start\|pick\|continue\|status\|list\|cleanup`. |
| [`dep-audit/`](dep-audit/README.md) | Cargo dependency audit (unused, duplicate-version, heavy, cross-repo drift) over the root workspace, `openhuman-app`, and every Cargo submodule, driven by `tinyanalyzer`. |

## pnpm-wired entry points

| Command | Script |
| --- | --- |
| `pnpm docs:generate` / `docs:check` | [`generate-architecture-docs.mjs`](generate-architecture-docs.mjs) |
| `pnpm test:rust` | [`test-rust-with-mock.sh`](test-rust-with-mock.sh) (via `openhuman-app`'s `test:rust`) |
| `pnpm test:rust:e2e` | [`test-rust-e2e.sh`](test-rust-e2e.sh) |
| `pnpm mock:api` | [`mock-api-server.mjs`](mock-api-server.mjs) |
| `pnpm debug ...` | [`debug/cli.sh`](debug/cli.sh) |
| `pnpm rust:layout` | [`ci/check-openhuman-rust-layout.mjs`](ci/check-openhuman-rust-layout.mjs) |
| `pnpm dep:audit` | [`dep-audit/run.sh`](dep-audit/run.sh) |
| `pnpm test:inventory` | [`generate-test-inventory.mjs`](generate-test-inventory.mjs) |
| `pnpm pr:checklist` | [`check-pr-checklist.mjs`](check-pr-checklist.mjs) |
| `pnpm prompt:report` | [`prompt-report.sh`](prompt-report.sh), every agent's fixed prefix (prompt + tool schemas), largest first; report-only |

See [`package.json`](../package.json) for the full list, including `i18n:*`, `tauri:ios:*`,
`tauri:android:*`, and the `shortcuts/`/`deep-work`/`agent-batch` commands
above.

## Families by prefix

- **`run-dev-*`**: dev-loop launchers. [`run-dev-macos.sh`](run-dev-macos.sh) backs `dev:app` in
  [`app/package.json`](../app/package.json); [`run-dev-win.cmd`](run-dev-win.cmd) backs `dev:app:win`; [`run-dev-web.sh`](run-dev-web.sh)
  backs the root `dev:app:web`.
- **`test-*`**: Rust and flow test runners ([`test-rust-with-mock.sh`](test-rust-with-mock.sh),
  [`test-rust-e2e.sh`](test-rust-e2e.sh), `test-rust-inference-e2e.sh`, channel/webhook/onboarding
  flow smoke tests).
- **`debug-*`**: one-off live debugging helpers (Composio, Notion, agent
  prompts, skills) distinct from the `debug/` runner directory.
- **`check-*`** (outside `ci/`), repo-level gates: [`check-coverage-matrix.mjs`](check-coverage-matrix.mjs),
  [`check-domain-e2e-coverage.mjs`](check-domain-e2e-coverage.mjs), [`check-pr-checklist.mjs`](check-pr-checklist.mjs),
  [`check-kernel-floor.sh`](check-kernel-floor.sh) (dependency-floor ratchet), [`check-linux-tls-dependencies.sh`](check-linux-tls-dependencies.sh).
- **`i18n-*`**: translation coverage and audit tools ([`i18n-coverage.ts`](i18n-coverage.ts),
  [`i18n-find-english.ts`](i18n-find-english.ts), [`i18n-react-audit.ts`](i18n-react-audit.ts), behind `pnpm i18n:*`);
  [`i18n-doc-scan.sh`](i18n-doc-scan.sh) scans the Chinese GitBook docs instead of the app.
- **[`ios-init.sh`](ios-init.sh) / [`android-init.sh`](android-init.sh)**: `tauri ios init` /
  `tauri android init` wrappers (`pnpm tauri:ios:init`, `pnpm tauri:android:init`)
  for the experimental mobile clients; **`ios-appstore-*`** build, export, and
  upload the iOS IPA and its App Store Connect assets/metadata.
- **[`install.sh`](install.sh) / [`install.ps1`](install.ps1)**: the public `curl | bash` / `irm | iex`
  installers documented in [`docs/gitbooks/en/developing/getting-set-up.md`](../docs/gitbooks/en/developing/getting-set-up.md); tested by
  [`test_install.sh`](test_install.sh) / `tests/OpenHumanWindowsInstall.Tests.ps1` and smoke-checked
  by [`docs/RELEASE-MANUAL-SMOKE.md`](../docs/RELEASE-MANUAL-SMOKE.md).
- **[`ci-cancel-aware.sh`](ci-cancel-aware.sh)**: wraps a long-running CI command, polling the
  workflow run's status (via `GH_TOKEN`) and killing the command's process tree
  when the run is cancelled, needed because `docker exec` swallows the
  runner's signals in container jobs. Required by [`AGENTS.md`](../AGENTS.md) for long CI
  build/test commands.
- **[`assert-shed.sh`](assert-shed.sh)** / **[`dep-sim.py`](dep-sim.py)**: prove or simulate a dependency
  reduction before claiming one in a PR; cited by [`AGENTS.md`](../AGENTS.md).

## Further reading

- [`tests/README.md`](../tests/README.md), the Rust integration targets these runners drive.
- [Testing strategy](../docs/gitbooks/en/developing/testing-strategy.md) and [E2E testing](../docs/gitbooks/en/developing/e2e-testing.md).
- [Getting set up](../docs/gitbooks/en/developing/getting-set-up.md) and [`CONTRIBUTING.md`](../CONTRIBUTING.md).
- [`crates/README.md`](../crates/README.md) for the crates these scripts build.
