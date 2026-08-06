# macOS Clippy Coverage in CI Full

## Context

Issue #5019 identifies a platform gap in Rust lint coverage. CI Lite runs the
root-core and Tauri clippy commands on Ubuntu, so code compiled only on macOS is
not checked with warnings denied. The existing `build-macos-full` job builds the
Tauri app on `macos-latest`, warming `app/src-tauri/target` (including the
path-dependent core in that Cargo graph), and feeds its result into the blocking
CI Full gate. Its Rust cache covers both Cargo workspaces, but the independent
root `target/` can remain cold on a cache miss.

The repository already owns the authoritative aggregate lint command:

```text
pnpm rust:clippy
```

That command runs clippy for the root `openhuman` crate and then the Tauri crate,
with `-D warnings` configured by the existing package scripts.

## Goals

- Compile and lint macOS-gated Rust with warnings denied.
- Cover both the root-core and Tauri Cargo worlds.
- Reuse the existing macOS build job, two-workspace Rust cache, and warmed Tauri
  artifacts.
- Preserve cancellation-aware behavior for the long-running command.
- Make lint failures block CI Full through its existing gate topology.

## Non-goals

- Adding macOS runners to CI Lite.
- Creating a separate macOS lint job that repeats setup and can cold-build both
  Cargo graphs on a cache miss.
- Changing Rust lint flags or package scripts.
- Fixing warnings that the new CI step may discover.
- Changing the existing 60-minute job timeout without runtime evidence.

## Considered approaches

### 1. Lint inside `build-macos-full` after the app build

Add one step after `Build E2E app`:

```yaml
- name: Clippy (macOS-gated Rust)
  run: bash scripts/ci-cancel-aware.sh pnpm rust:clippy
```

This reuses the existing runner setup and two-workspace Rust cache. The
preceding build warms the Tauri target, while the independent root target may
still require compilation on a cache miss. The step adds no new job dependency
and naturally blocks the existing macOS build and CI Full gates.

This is the selected approach.

### 2. Add a separate macOS clippy job

This makes lint timing and failures more visible, but it requires another runner,
another dependency setup sequence, and likely another cold build of both Cargo
worlds.

### 3. Run macOS clippy in CI Lite

This gives earlier feedback on every relevant pull request, but materially
increases the cost and latency of the fast lane. CI Full is already the intended
cross-platform gate.

## Design

Modify only `.github/workflows/e2e-reusable.yml`. In `build-macos-full`, insert
the macOS clippy step immediately after `Build E2E app` and before packaging the
artifact.

The step must:

- invoke the existing root `pnpm rust:clippy` script rather than duplicating its
  two Cargo commands in workflow YAML;
- run through `scripts/ci-cancel-aware.sh`;
- retain the existing Rust toolchain, dependency installation, cache, and
  environment setup from `build-macos-full`;
- leave `timeout-minutes: 60` unchanged initially.

Running after the build is deliberate: the build is the job's primary artifact
producer, and it warms the Tauri target before clippy recompiles its
lint-specific units. The independent root target benefits from the existing
two-workspace cache but may still be cold on a cache miss. Running before
artifact packaging also prevents a failed lint from publishing an artifact that
downstream shards cannot consume.

## Failure behavior

A clippy warning or error exits the macOS build job unsuccessfully. Downstream
macOS shards remain blocked by their existing `needs: build-macos-full`
dependency, and the existing CI Full gate reports the failure. Cancellation is
propagated by `scripts/ci-cancel-aware.sh`.

If a cold-cache run shows that the extra lint step approaches the 60-minute
ceiling, timeout adjustment is a separate evidence-driven follow-up in the same
PR. The initial implementation does not speculate by increasing it.

## Validation

Local validation:

1. Run Prettier's check against `.github/workflows/e2e-reusable.yml`.
2. Run any repository workflow/YAML validation available in the checkout.
3. Confirm the diff changes only the selected job and documentation/plan files.

GitHub validation:

1. Open the PR against `tinyhumansai/openhuman:main`.
2. Run CI Full for the branch.
3. Confirm `Build (macOS full)` executes both root-core and Tauri clippy through
   `pnpm rust:clippy`.
4. Confirm the macOS build artifact and downstream shards still complete.
5. Record the job runtime and adjust the timeout only if the observed cold-cache
   runtime requires it.

## Acceptance criteria

- CI Full executes `pnpm rust:clippy` on `macos-latest`.
- The command is cancellation-aware.
- Both Cargo worlds run with warnings denied through their existing scripts.
- A lint failure blocks the existing CI Full gate.
- Successful runs still package the macOS artifact and fan out to the full-suite
  shards.
- CI Lite behavior and workflow scope remain unchanged.
