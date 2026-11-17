# CI Fast: the lane flow

CI Fast groups pre-merge checks into **lanes**, split across a few GitHub-hosted
jobs. Independent lanes within a job may run in parallel. It is the pull-request
lane flow: the `ci-lite.yml` workflow it was originally built alongside was
deleted on 2026-10-01. The one check to require is **CI Gate** (see
[CI Gate](#ci-gate)).

Every PR runs `ci-fast-hosted.yml` (`pull_request`), which calls
`.github/workflows/ci-lanes.yml`, which runs
`scripts/ci/self-hosted/lanes.mjs`. The plan itself is in
`scripts/ci/self-hosted/lanes-plan.mjs`, and
`scripts/__tests__/self-hosted-lanes.test.mjs` pins its shape.

OpenHuman no longer uses the EX63 runners. `lanes-plan.mjs` still carries the
unused `ex63` profile.

## Lanes

| Lane              | What runs                                                                                                                                                                                                                                                                 |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `static`          | fmt, layout, runtime boundary, ignored-tests, TLS policy, gated-test allowlist, orch-ip gate, feature forwarding, module pins and monotonicity, toolchain drift, test inventory                                                                                           |
| `frontend`        | pnpm install, tsc, prettier, eslint, i18n, docs, script self-tests                                                                                                                                                                                                        |
| `frontend-tests`  | the complete vitest suite with coverage                                                                                                                                                                                                                                   |
| `rust-cov`        | test modules from the registry, then `scripts/ci/rust-coverage.sh`                                                                                                                                                                                                        |
| `rust-lint`       | clippy (product set; embed's clippy covers the core's contributor set), embed and tinyhumans lint, embed gates-off check                                                                                                                                                  |
| `rust-gates-off`  | gates-off checks and gate-contract tests, kernel floor, dep-sim calibration                                                                                                                                                                                               |
| `tauri`           | Tauri clippy and coverage                                                                                                                                                                                                                                                 |
| `pester`          | `install.ps1` tests                                                                                                                                                                                                                                                       |
| `inbound-e2e`     | test modules from the registry, then `tests/inbound_module_e2e.rs` against the verified TinyEventQueues release (its `e2e-test-support` gate keeps it out of `rust-cov`); armed by the `inbound` area only                                                              |

How lanes behave:

- **Selection is by area only.** The changed-area filters in
  `.github/ci-paths-filter.yml` decide whether a whole suite runs. Nothing narrows a suite to the changed files.
- **Nothing stops early.** A failed check never stops the checks after it
  in its lane, and never stops other lanes. Inside the coverage check,
  `scripts/ci/rust-coverage.sh` runs every crate and integration target even
  after one fails, merges the lcov report from what ran, and then lists every
  failure. A check whose dependency failed reports `blocked`. Only a
  cancellation stops a run.
- **One workflow step per lane.** `Start lanes` detaches the scheduler. Hosted
  jobs may execute their independent lanes in parallel. Each `Lane: …` step streams its
  own lane's log live, with one folded group per check, and passes or fails
  on that lane alone. `Lane summary` carries the overall result.
- **Changed-line coverage** must be at least 80% through
  `scripts/ci/self-hosted/diff-cover.sh`.

The lanes include the TUI's default-feature test suite. Embed doctests, rustdoc
and offline examples run in the lint lane. The PR coverage check sets
`OH_COV_DOCTESTS=0`; core doctests and the TinyJuice host-module regression run
through [the reusable full unit suites](../.github/workflows/test-reusable.yml),
called by [CI Full](../.github/workflows/ci-full.yml) for release validation.
The no-default embed check and gates-off tests compile their respective feature
sets separately from product coverage.

## CI Gate

[CI Gate](../.github/workflows/ci-gate.yml) posts one commit status, `CI Gate`, on
each PR head commit. Branch protection should require this status. The current
flow and decisive job are:

| Flow                                                        | Decisive job                    |
| ----------------------------------------------------------- | ------------------------------- |
| [CI Fast (hosted)](../.github/workflows/ci-fast-hosted.yml) | `Lanes / CI Fast (hosted) Gate` |

The gate workflow and script also recognize legacy `CI Lite` / `PR CI Gate`
runs for compatibility; there is no current `ci-lite.yml` workflow.

- **success** when any observed flow's decisive job passes. Other running flows
  for that commit are then cancelled.
- **pending** while no decisive job has passed and an observed flow is still
  running, or while no flow has appeared.
- **failure** when all observed flows finish without a decisive job passing.
  A skipped job is never a pass.

CI Full separately handles PRs and pushes to `release`, plus manual dispatch.
CI Fast is triggered by PR events, and the CI Gate updater ignores non-PR
workflow runs.

The workflow runs from the default branch through `workflow_run`, with
`statuses: write` and `actions: write`. It never checks out PR code; the
decision (`scripts/ci/ci-gate.mjs`, tested in
`scripts/__tests__/ci-gate.test.mjs`) reads run and job state from the API.
`CI_GATE_DRY_RUN=1` with `GH_TOKEN`, `REPO` and `HEAD_SHA` prints the verdict
for any commit without posting it.

## Job groups

Lanes are grouped into jobs sized for 4-core, ~14 GB runners: `checks`,
`rust-lint` (lint and gates-off in turn), `rust-cov-core`, `rust-cov-product`,
`rust-cov-facades` and `rust-cov-tui` (the `rust-cov` lane in four jobs, each a whole
section of `scripts/ci/rust-coverage.sh`, picked by `OH_COV_SHARD`; the gate merges their
lcov files and fails if any is missing), `tauri`, `pester` and `inbound-e2e`.
Groups whose areas are untouched don't start.

## Measuring

```bash
node scripts/ci/self-hosted/compare-runs.mjs            # historical CI Lite comparisons
node scripts/ci/self-hosted/compare-runs.mjs --lanes    # include historical lane times
```

The comparison script requires a matching legacy CI Lite run, so it does not
measure commits that only have the current lane flows. For current measurements,
use workflow elapsed time and the uploaded per-check timing records.

Each run uploads `ci-out-*` artifacts with per-lane logs, `ci-timings.json`
(per-check timings and target-dir sizes), linkage proof, and lcov. See the suite inventory
and measurement record in [ci-private-validation.md](ci-private-validation.md).

## Local use

```bash
CI_AREA_RUST_CORE=true node scripts/ci/self-hosted/lanes.mjs --profile hosted --dry-run
CI_AREA_DOCS=true node scripts/ci/self-hosted/lanes.mjs --profile hosted --lanes static,frontend
```
