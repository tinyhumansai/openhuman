# CI Fast: the lane flow and the EX63 runners

CI Fast groups pre-merge checks into **lanes**, scheduled sequentially on the
private runner and across a few hosted jobs. Independent lanes within a hosted
job may run in parallel. It is the pull-request lane flow: the `ci-lite.yml`
workflow it was originally built alongside was deleted on 2026-10-01. The one check to require is **CI Gate**
(see [CI Gate](#ci-gate)).

| Routing decision                                | Workflow                              | Where it runs                                                             |
| ----------------------------------------------- | ------------------------------------- | ------------------------------------------------------------------------- |
| author and actor admitted by membership routing | `ci-fast.yml` (`pull_request_target`) | one job on a throwaway Firecracker microVM on the Hetzner EX63            |
| otherwise                                       | `ci-fast-hosted.yml` (`pull_request`) | the same lanes split over GitHub-hosted jobs, with GitHub's Actions cache |

Both call `.github/workflows/ci-lanes.yml`, which runs
`scripts/ci/self-hosted/lanes.mjs`. The plan itself is in
`scripts/ci/self-hosted/lanes-plan.mjs`, and
`scripts/__tests__/self-hosted-lanes.test.mjs` pins its shape.

The host side (microVM supervisor, guest image, firewall, deploy) lives in
the private repo `tinyhumansai/gh-hosted-runner`. Its README covers
provisioning, deploys and the runner token.

## Lanes

| Lane              | What runs                                                                                                                                                                                                                                                                 |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `static`          | fmt, layout, runtime boundary, ignored-tests, TLS policy, gated-test allowlist, orch-ip gate, feature forwarding, module pins and monotonicity, toolchain drift, test inventory                                                                                           |
| `frontend`        | pnpm install, tsc, prettier, eslint, i18n, docs, script self-tests                                                                                                                                                                                                        |
| `frontend-tests`  | the complete vitest suite with coverage                                                                                                                                                                                                                                   |
| `rust-cov`        | test modules from the registry, then `scripts/ci/rust-coverage.sh`                                                                                                                                                                                                        |
| `rust-lint`       | clippy (product set; embed's clippy covers the core's contributor set), embed and tinyhumans lint, embed gates-off check                                                                                                                                                  |
| `rust-gates-off`  | gates-off checks and gate-contract tests, kernel floor, dep-sim calibration                                                                                                                                                                                               |
| `tauri`           | Tauri clippy and coverage, plus a private uninstrumented application build                                                                                                                                                                                                |
| `pester`          | `install.ps1` tests                                                                                                                                                                                                                                                       |

How lanes behave:

- **Selection is by area only.** The changed-area filters in
  `.github/ci-paths-filter.yml` decide whether a whole suite runs. Nothing narrows a suite to the changed files.
- **Nothing stops early.** A failed check never stops the checks after it
  in its lane, and never stops other lanes. Inside the coverage check,
  `scripts/ci/rust-coverage.sh` runs every crate and integration target even
  after one fails, merges the lcov report from what ran, and then lists every
  failure. A check whose dependency failed reports `blocked`. Only a
  cancellation stops a run.
- **One workflow step per lane.** `Start lanes` detaches the scheduler. Private lanes execute sequentially;
  hosted jobs may execute their independent lanes in parallel. Each `Lane: …` step streams its
  own lane's log live, with one folded group per check, and passes or fails
  on that lane alone. `Lane summary` carries the overall result.
- **Changed-line coverage** must be at least 80% through
  `scripts/ci/self-hosted/diff-cover.sh` in each lane profile.

Both lane profiles include the TUI's default-feature test suite. Private core
coverage runs eight configurations: core and CLI integration tests with product
features; embed and TinyHumans with both product and default features; and RPC
and TUI with default features. Default and product configurations remain
separate. Embed doctests, rustdoc and offline examples run in the lint lane.
The PR coverage check sets `OH_COV_DOCTESTS=0`; core doctests and the TinyJuice
host-module regression run through [the reusable full unit suites](../.github/workflows/test-reusable.yml),
called by [CI Full](../.github/workflows/ci-full.yml) for release validation.
The no-default embed check and gates-off tests compile their respective feature
sets separately from product coverage. Private Tauri validation also performs
an uninstrumented application build before its nextest coverage suite.

## CI Gate

[CI Gate](../.github/workflows/ci-gate.yml) posts one commit status, `CI Gate`, on
each PR head commit. Branch protection should require this status whichever
lane profile handled the PR. The current flows and decisive jobs are:

| Flow                                                        | Decisive job                    |
| ----------------------------------------------------------- | ------------------------------- |
| [CI Fast](../.github/workflows/ci-fast.yml)                 | `Lanes / CI Fast (EX63)`        |
| [CI Fast (hosted)](../.github/workflows/ci-fast-hosted.yml) | `Lanes / CI Fast (hosted) Gate` |

The gate workflow and script also recognize legacy `CI Lite` / `PR CI Gate`
runs for compatibility; there is no current `ci-lite.yml` workflow.

- **success** when any observed flow's decisive job passes. Other running
  GitHub-hosted flows for that commit are then cancelled; EX63 is never cancelled
  by the gate.
- **pending** while no decisive job has passed and an observed flow is still
  running, or while no flow has appeared.
- **failure** when all observed flows finish without a decisive job passing.
  A skipped job is never a pass.

Who runs what:

- **Members admitted by the route:** `ci-fast.yml` uses the PR author and actor
  membership check from the trusted base workflow to select EX63.
- **Outsiders:** the EX63 lanes are skipped and `ci-fast-hosted.yml` runs the
  hosted lanes. There is no CI Lite race.
- **Routing fallback:** the hosted workflow polls the EX63 workflow's routing
  decision using `scripts/ci/ci-fast-route.sh`. After about three minutes without
  a decision it falls back to the PR's author association and author/actor
  comparison; that fallback can still classify a member PR for EX63.
- **Release validation:** CI Full separately handles PRs and pushes to `release`,
  plus manual dispatch. CI Fast's workflows are triggered by PR events, and the
  CI Gate updater ignores non-PR workflow runs.

The workflow runs from the default branch through `workflow_run`, with
`statuses: write` and `actions: write`. It never checks out PR code; the
decision (`scripts/ci/ci-gate.mjs`, tested in
`scripts/__tests__/ci-gate.test.mjs`) reads run and job state from the API.
`CI_GATE_DRY_RUN=1` with `GH_TOKEN`, `REPO` and `HEAD_SHA` prints the verdict
for any commit without posting it.

## Profiles

- **`ex63`**: one job selects both the `openhuman-ex63` runner group and label.
  Lanes run sequentially within a 180-minute job budget, including after an
  independent check fails. Compatible
  uninstrumented outputs share `target/ci-private-build`; coverage uses
  `target/ci-private-coverage`. Neither is restored as an Actions artifact cache.
  The supervisor's `JOB_HARD_TIMEOUT_MIN=195` leaves a 15-minute shutdown grace
  beyond the workflow budget; reload changed settings only on drained idle slots.
  - `scripts/ci/self-hosted/guest-image.txt` pins the guest OCI digest. Before
    validation, `verify-runner.sh` compares `/etc/ohci/build-info` with that pin,
    checks preinstalled tools (Python 3.10 with tomli, or stdlib tomllib), and selects `rust-toolchain.toml` explicitly.
    Missing metadata, tools, or the compiler fail; CI does not install replacements.
    The selected compiler's `llvm-cov` and `llvm-profdata` executables are checked,
    and `CARGO_LLVM_COV_SETUP=no` disables implicit component installation.
  - The pinned shared checkout action fetches the PR merge ref with
    `fetch-depth: 0` and `submodule-depth: 0`: full root and recursive submodule
    history for pin ancestry checks, with unused `wiki` submodules excluded.
  - The pinned Tinymold setup/save actions use the image's binary and signed
    remote cache. The supervisor supplies credentials through guest metadata;
    the workflow masks them and needs no additional repository secrets. Cache
    save runs after failures, has a fifteen-minute limit, and cannot replace the
    validation result or indefinitely delay artifact upload.
  - Cargo's target linker points at `/usr/local/bin/rust-mold-linker`. It survives
    coverage clearing `RUSTFLAGS`. `verify-linkage.mjs` checks `.comment` and LLVM
    sections in actual ordinary and instrumented lane executables and archives proof.
    Ordinary private dev/test builds use line-table debug information, retaining
    file:line backtraces with smaller outputs. Coverage keeps its existing
    `CARGO_PROFILE_DEV_DEBUG=0`; local/hosted and release profile settings are unchanged.
  - Coverage runs each selected package/feature set once through nextest, which
    fails when the selected suite has no tests. Process-per-test isolation and
    nextest resource groups retain external-resource serialization.
    All private instrumented checks use `LLVM_PROFILE_FILE_NAME=openhuman-%8m.profraw`:
    LLVM's locked merge pool bounds raw profiles to eight files per binary,
    including the auxiliary gates, Tauri, and storage configurations.
  - Redundant compilations are avoided on purpose; keep them out when editing:
    - `rust-coverage.sh` (nextest runner) builds the product-feature packages
      (`openhuman`, `-embed`, `-tinyhumans`, `-cli`) with one Cargo argument set
      and varies only the nextest filter per suite. Cargo hashes units by
      resolved features and those packages resolve the core differently when
      selected separately, so separate `-p` runs would each compile the core
      again. The `rpc`, `tui` and default-feature suites keep their own
      resolutions on purpose; the default-feature `-embed` and `-tinyhumans`
      suites share one, so that configuration builds the core once. Hosted
      runs this runner too (the workflow installs nextest).
    - Auxiliary coverage suites (gates-off, Tauri) pass `--no-report`; each
      `cargo llvm-cov report` re-merges every profile in the shared coverage
      target, so each lane ends with one report check (`gates-off-coverage-report`,
      `tauri-coverage-report`) that still runs after a failed suite.
    - `storage-schema` builds its dump example in `target/ci-private-build`
      with the plain build's environment and the core product features
      (`STORAGE_SCHEMA_FEATURES=core-product`), shared with the uninstrumented
      Composio regression's `openhuman-cli` build. The schema only depends on
      gates present in both sets.
    - Offline examples run with `--profile dev`, so the release-declared Linux
      fleet measurement shares the dev build instead of a thin-LTO one. Run the
      script without `--profile` to get the declared release measurement.
  - The scratch disk is the scarce resource. It is 320 GB (24 GB RAM, 4 vCPU
    per VM) since 2026-10-20; at 160 GB, EX63 run 39292268284 ran it to 16 MB
    free in `rust-lint`. Instrumented test binaries dominate it: each
    one links the whole core, so the nextest coverage suites build `--tests`
    rather than `--all-targets` (examples run nothing under nextest and each is
    as large as a test binary). Raw profiles are not the cause: the `%8m` pool
    above bounds them. Every lane's log ends with `df -h` of the scratch dir and
    `du -sh` of `target/*` and the top of the scratch dir
    (`diskUsageReport` in `lanes.mjs`, ex63 only); read those before changing
    anything else.
  - Cargo registry, pnpm store, and content-based frontend caches remain on the
    slot cache disk. Rust output caching uses Tinymold alone.
  - VM sizing is a supervisor setting; it is not assumed by the lane scheduler.
    The frontend lint/check lane sets `NODE_OPTIONS=--max-old-space-size=4096`
    (4096 MiB of old-space heap); hosted keeps its default Node budget.
    Vitest currently uses 8 workers.
- **`hosted`**: lanes grouped into jobs sized for 4-core, ~14 GB runners:
  `checks`, `rust-lint` (lint and gates-off in turn), `rust-cov`, `tauri`
  and `pester`. Groups whose areas are untouched don't start.

## Why the EX63 cannot run outsider code

- `ci-fast.yml` runs from `main` (`pull_request_target`), so a PR cannot
  edit its routing.
- The runner group `openhuman-ex63` only admits `ci-fast.yml` and
  `ci-lanes.yml` at `refs/heads/main`. A fork's own `pull_request` workflow
  can't claim the runners even if it names their label.
- The host supervisor re-checks the run's actors against org membership and
  kills the VM for a non-member.

**Optional repo secret `CI_MEMBERSHIP_TOKEN`:** a fine-grained token with only
_Organization → Members: Read_. It lets the route job recognise _private_ org
members. Without it, only the PR's `author_association` is used, and private
members may land on GitHub-hosted runners instead.

## Measuring

```bash
node scripts/ci/self-hosted/compare-runs.mjs            # historical CI Lite comparisons
node scripts/ci/self-hosted/compare-runs.mjs --lanes    # include historical lane times
```

The comparison script requires a matching legacy CI Lite run, so it does not
measure commits that only have the current lane flows. For current measurements,
use workflow elapsed time and the uploaded per-check and cache timing records.

Each run uploads `ci-out-*` artifacts with per-lane logs, `ci-timings.json`
(per-check timings and target-dir sizes), linkage proof, and lcov. Tinymold
setup/save logs contain cache hit/miss and save timings. See the suite inventory
and measurement record in [ci-private-validation.md](ci-private-validation.md).

## Local use

```bash
CI_AREA_RUST_CORE=true CI_SCRATCH_DIR=/scratch node scripts/ci/self-hosted/lanes.mjs --profile ex63 --dry-run
CI_AREA_DOCS=true node scripts/ci/self-hosted/lanes.mjs --profile hosted --lanes static,frontend
```

## Image rollout

Publish and smoke-test the image first. Roll the pinned rootfs into idle slots
without interrupting active jobs, then change `guest-image.txt`. Check the metadata
inside a new guest rather than trusting only the supervisor configuration. The
current image ships Rust 1.99.1 (the pinned toolchain), mold 2.40.4, bubblewrap 0.13.1, Tinymold 0.3.0 and nextest 0.9.148.

Private workflow routing runs from the trusted base branch. A PR run can therefore
exercise the previous workflow; green pre-merge CI alone does not prove a workflow
change is active. Verify the new behavior after merge as well as in an isolated
guest before enabling it. Hosted fallback, macOS/Windows, distinct E2E, security
gates, and release workflows retain their existing ownership and behavior.
