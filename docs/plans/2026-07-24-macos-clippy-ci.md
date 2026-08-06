# Issue #5019: macOS Clippy Coverage in CI Full — Implementation Plan

> **Approved design:** `docs/specs/2026-07-24-macos-clippy-ci-design.md` at
> `c4085c446e12d022d6275c1a92cc98a702b0db49`
>
> **Implementation scope:** one workflow file and one implementation commit.
> The design/spec and this plan are already committed separately and must not be
> included in the implementation commit.

## Objective

Add the repository's existing aggregate Rust lint command to the existing
`build-macos-full` job so CI Full checks macOS-gated code in both the root-core
and Tauri Cargo worlds, denies warnings through the package scripts already in
place, remains cancellation-aware, and refuses to publish the macOS E2E build
artifact when linting fails.

## Critical review of the approved design

The selected change is compatible with the current repository:

- `.github/workflows/e2e-reusable.yml` defines `build-macos-full` on
  `macos-latest` with `timeout-minutes: 60`.
- `Build E2E app` precedes `Package build artifact`; inserting Clippy between
  them makes a non-zero lint exit prevent packaging and upload.
- `e2e-macos-full` already has `needs: build-macos-full`, so all macOS shards
  remain blocked if Clippy fails.
- `.github/workflows/ci-full.yml` treats the reusable `e2e-desktop` workflow as
  a dependency of `CI Full Gate`, so no gate-topology edit is needed.
- Root `package.json` defines `pnpm rust:clippy` as root
  `cargo clippy -p openhuman -- -D warnings` followed by the app workspace's
  `rust:clippy`; `app/package.json` defines the latter as Tauri
  `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`.
- The workflow already grants `actions: read` and exports `GH_TOKEN`, which
  lets `scripts/ci-cancel-aware.sh` propagate GitHub run cancellation.

One performance nuance must be measured, not papered over: the preceding Tauri
app build warms `app/src-tauri/target`, including the path-dependent core as
compiled in the Tauri Cargo graph. It does not compile the independent root
Cargo graph into `target/`; that graph benefits from the existing two-workspace
Rust cache but can still be cold on a cache miss. This does not invalidate the
design, but it is why the initial implementation must leave the 60-minute
timeout unchanged and the CI Full run time must be recorded.

No dedicated workflow validator such as `actionlint` or `yamllint` is installed
in the current checkout. The validation below therefore combines Prettier's
YAML parse/check, Ruby's YAML parser, and a focused structural assertion. Do not
add a validator dependency for this one-step workflow change.

## Task 1: Add the macOS Clippy gate before artifact packaging

**Files:**

- Modify: `.github/workflows/e2e-reusable.yml`

### Step 1: Prove the required workflow structure is absent

From the worktree root, run this focused structural check before editing:

```bash
node --input-type=module <<'NODE'
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const workflow = readFileSync(".github/workflows/e2e-reusable.yml", "utf8");
const jobStart = workflow.indexOf("\n  build-macos-full:\n");
const jobEnd = workflow.indexOf("\n  e2e-macos-full:\n", jobStart);
assert.notEqual(jobStart, -1, "build-macos-full job must exist");
assert.notEqual(jobEnd, -1, "e2e-macos-full boundary must exist");

const job = workflow.slice(jobStart, jobEnd);
const build = job.indexOf("      - name: Build E2E app\n");
const clippy = job.indexOf("      - name: Clippy (macOS-gated Rust)\n");
const packageArtifact = job.indexOf("      - name: Package build artifact\n");

assert.notEqual(build, -1, "Build E2E app step must exist");
assert.notEqual(clippy, -1, "macOS Clippy step must exist");
assert.notEqual(packageArtifact, -1, "Package build artifact step must exist");
assert.ok(build < clippy, "Clippy must run after Build E2E app");
assert.ok(clippy < packageArtifact, "Clippy must run before artifact packaging");
assert.equal(
  (job.match(/      - name: Clippy \(macOS-gated Rust\)\n/g) ?? []).length,
  1,
  "build-macos-full must contain exactly one macOS Clippy step",
);
assert.match(
  job,
  /      - name: Clippy \(macOS-gated Rust\)\n        run: bash scripts\/ci-cancel-aware\.sh pnpm rust:clippy\n/,
  "Clippy must use the aggregate package script through the cancellation wrapper",
);
assert.match(job, /    runs-on: macos-latest\n/);
assert.match(job, /    timeout-minutes: 60\n/);
NODE
```

Expected result: the command exits non-zero with
`AssertionError: macOS Clippy step must exist`. A different failure means the
workflow shape has drifted; stop and reconcile the plan with the current file
instead of inserting by an assumed line number.

### Step 2: Insert the smallest workflow change

In `.github/workflows/e2e-reusable.yml`, add exactly this step immediately after
`Build E2E app` in `build-macos-full` and immediately before
`Package build artifact`:

```yaml
- name: Clippy (macOS-gated Rust)
  run: bash scripts/ci-cancel-aware.sh pnpm rust:clippy
```

Do not:

- add conditions to the step;
- duplicate either Cargo command in YAML;
- alter `timeout-minutes: 60`;
- change caches, toolchains, environment setup, packaging, artifact upload, or
  shard dependencies;
- add this step to the selected-spec macOS job, Linux/Windows jobs, CI Lite, or
  any other workflow.

The command ordering is intentional. The root package script short-circuits on
root-core Clippy failure; only a successful root lint proceeds to the Tauri
lint. Either failure makes the step and `build-macos-full` fail.

### Step 3: Re-run the focused structural check

Run the exact Node command from Step 1 again.

Expected result: exit status 0 with no output. This proves the named step occurs
exactly once in `build-macos-full`, uses the cancellation wrapper and aggregate
package script, remains between build and packaging, and leaves the runner and
timeout unchanged.

### Step 4: Validate YAML and formatting

Run:

```bash
pnpm --dir app exec prettier --check ../.github/workflows/e2e-reusable.yml
ruby -e 'require "yaml"; YAML.parse_file(".github/workflows/e2e-reusable.yml"); puts "YAML syntax OK"'
git diff --check
```

Expected results:

- Prettier reports `All matched files use Prettier code style!`.
- Ruby prints `YAML syntax OK`.
- `git diff --check` exits 0 with no output.

Do not run Prettier in write mode over the workflow or repository; the intended
diff is two added lines only.

### Step 5: Review scope and topology in the diff

Run:

```bash
git diff -- .github/workflows/e2e-reusable.yml
git diff --name-only
git status --short
```

Confirm all of the following before committing:

- the workflow diff contains only the two-line Clippy step;
- the step is inside `build-macos-full`, after `Build E2E app`, and before
  `Package build artifact`;
- `timeout-minutes: 60` is unchanged;
- no CI Lite, package-script, cache, packaging, upload, or shard dependency
  lines changed;
- the only uncommitted implementation path is
  `.github/workflows/e2e-reusable.yml`.

If the plan file is still uncommitted in the executor's checkout, do not include
it in the implementation commit; this plan's authoring commit is a prerequisite.

### Step 6: Create the atomic implementation commit

Run:

```bash
atomic-commit "ci(e2e): lint macOS Rust before artifact packaging" -- .github/workflows/e2e-reusable.yml
```

Expected implementation commit boundary:

1. `ci(e2e): lint macOS Rust before artifact packaging` — contains only
   `.github/workflows/e2e-reusable.yml`.

There is no second expected implementation commit. Warning fixes discovered by
the new gate are an approved non-goal and must not be folded into this workflow
commit. A timeout change is also excluded unless CI Full supplies concrete
cold-cache runtime evidence; if that evidence requires a timeout follow-up,
pause and obtain scope confirmation before creating a separate atomic commit.

### Step 7: Verify the committed implementation

Run:

```bash
git show --stat --oneline HEAD
git show --format= --name-only HEAD
git status --short
```

Expected result: the commit names only
`.github/workflows/e2e-reusable.yml`, and the worktree is clean.

## GitHub validation after publication

The implementation is not fully validated until CI runs on GitHub's macOS
runner. After the branch is pushed and its PR is opened against
`tinyhumansai/openhuman:main`, manually dispatch `.github/workflows/ci-full.yml`
on the implementation branch (CI Full's automatic PR trigger targets `release`,
not `main`).

Inspect `Build (macOS full)` and confirm:

1. `Build E2E app` succeeds before the new step.
2. `Clippy (macOS-gated Rust)` executes
   `bash scripts/ci-cancel-aware.sh pnpm rust:clippy`.
3. The log shows root `cargo clippy -p openhuman -- -D warnings` and then Tauri
   `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`.
4. `Package build artifact` and `Upload build artifact` run only after Clippy
   succeeds.
5. Every `E2E (macOS full / …)` shard starts after `build-macos-full` and
   completes successfully.
6. `Desktop E2E (full suite, 3 OS)` succeeds and `CI Full Gate` remains green.
7. The `Build (macOS full)` total duration is recorded, including whether the
   Cargo caches were hits or misses.

Do not infer failure-path behavior by intentionally committing a warning to the
PR. The workflow's default step semantics already propagate the non-zero exit,
the packaging steps have no `always()` override, and the macOS shards already
declare `needs: build-macos-full`.

If a real cold-cache run approaches or exceeds 60 minutes, preserve the run URL
and timing as evidence and treat any timeout adjustment as a separately scoped
follow-up. If Clippy exposes existing macOS-only warnings, report their exact
paths and diagnostics separately; fixing them is outside this implementation
plan.

## Definition of done

- The atomic implementation commit changes only
  `.github/workflows/e2e-reusable.yml`.
- `build-macos-full` runs the aggregate `pnpm rust:clippy` command on
  `macos-latest` through `scripts/ci-cancel-aware.sh`.
- Clippy is ordered after the E2E app build and before artifact packaging.
- Both package-script Clippy commands retain `-D warnings`.
- The 60-minute timeout, CI Lite, and existing gate topology are unchanged.
- Local structural, Prettier, YAML syntax, whitespace, and diff-scope checks
  pass.
- A manually dispatched CI Full run proves the macOS build artifact and
  downstream shards still complete, records runtime, and leaves
  `CI Full Gate` green.
