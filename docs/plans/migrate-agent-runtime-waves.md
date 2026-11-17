# Agent runtime migration waves

> **Status, verified 2026-10-07:** live. Gate 4 is unmet; the boundary
> baseline is still checked in. See [`README.md`](README.md) for the whole set
> and how to re-check it.

This is the execution and validation companion to
[`migrate-agent-runtime-to-tinyagents.md`](migrate-agent-runtime-to-tinyagents.md).
The boundary remains normative in
[`../specs/agent-runtime-upstream-boundary.md`](../specs/agent-runtime-upstream-boundary.md).

## Ownership rule

Ownership below is exclusive for the duration of a wave. An owner may edit only
the listed paths. Cross-owner consumer edits wait for the integration wave;
owners hand off tested commits rather than editing another owner's files. All
agents share the superproject worktree, preserve concurrent edits and automatic
checkpoint commits, and never create nested worktrees or squash commits.

## Wave 0: Enforcement

One OpenHuman enforcement owner owns only `scripts/ci/check-agent-runtime-boundary.mjs`,
its package/CI registration, its exact temporary baseline, and migration docs.
The checker must fail for both OpenHuman and TinyAgents compatibility facades,
forwarding modules, aliases, wrappers, and public re-exports of moved APIs.

**Gate 0:** the checker fails without the exact baseline, passes with it, and
TinyAgents dependency-boundary tests pass. Record the baseline count and paths
in the handoff message.

## Wave 1: Leaf contracts

These owners may work concurrently because their paths do not overlap:

| Owner | Exclusive paths | Tasks |
| --- | --- | --- |
| TinyTools leaf owner | `vendor/tinyagents/vendor/tinytools/**` | canonical tool vocabulary and tool-call protocols from Tasks 1–2 |
| TinyInference leaf owner | `vendor/tinyagents/vendor/tinyinference/**` | model metadata/decorators from Task 3 and embedding contracts from Task 2d |

Neither owner edits TinyAgents gitlinks, Cargo manifests outside its nested
repository, or OpenHuman consumers.

**Gate 1:** each leaf workspace is formatted, tested, linted, committed, pushed,
and represented by a ready upstream PR. Each handoff names the commit SHA and
the exact public imports that replace the old paths.

## Wave 2: TinyAgents consumers and primitives

One central dependency integrator exclusively owns TinyAgents' TinyTools and
TinyInference gitlinks, every Cargo manifest and lockfile, crate roots, and
workspace integration tests. It first pins the Gate 1 commits, then applies
component handoffs and performs the residual mechanical direct-import consumer
sweep. No component owner edits those paths.

Before coupled work begins, the context owner gives the tool-loop owner an
explicit context/tool API handoff. Context and tool-loop owners may then develop
concurrently on non-overlapping paths. Registry/graph conversion follows that
published context API:

| Owner | Exclusive paths | Tasks |
| --- | --- | --- |
| Tool-loop owner | harness `src/tool/**`, all harness `src/agent_loop/**`, and their tests | Tasks 1–2 direct imports; no facade |
| Context owner | harness `src/context/**`, `src/prompt/**`, `src/multimodal/**`, `src/retriever/**`, `src/subagent/**`, and their tests | Tasks 2a–2d and only the harness-context portion of Task 4 |
| Registry/graph owner | `vendor/tinyagents/crates/tinyagents-{registry,graph}/src/**` except `lib.rs`, and their module tests | graph-recursion portion of Task 4 and Tasks 6–7 |

Component owners provide exact API, export, and dependency handoffs; the
integrator applies them after their commits. If two tasks need the same
`agent_loop` file, the context owner hands its required interface to the
tool-loop owner instead of editing that file. The integrator retains each
crate's `Cargo.toml`, `lib.rs`, and integration-test roots. Finally, the
integrator deletes `tool_calling` and duplicate exports outright: no shim,
facade, alias, or compatibility re-export remains.

**Gate 2:** the TinyAgents workspace passes format, tests, clippy, and the
dependency-boundary suite. Its public API directly names leaf owners, contains
no compatibility module/re-export, and its commit records both Gate 1 gitlinks.

## Wave 3: Host-driven harness

Work is sequential because runtime and middleware meet in the agent loop:

1. The harness runtime owner exclusively edits harness `src/runtime/**`,
   `src/host/**`, remaining `src/agent_loop/**`, `src/middleware/**`,
   `src/structured/**`, `src/no_progress/**`, `src/handoff/**`,
   `src/run_queue/**`, `src/summarization/**`, `src/memory/**`,
   `src/artifacts/**`, and tests beneath or dedicated to those destinations for
   Tasks 5 and 8.
2. After its commit, that owner integrates the Wave 2 context, tool, registry,
   and graph APIs; no Wave 2 owner continues editing harness files.

**Gate 3:** recording-host tests prove all ten capabilities and every terminal
path; recursive children use the same entry point and explicit context; generic
middleware parity tests pass; the full TinyAgents workspace is green. Push a
ready TinyAgents PR and hand off its commit SHA plus migration import map.

## Wave 4: OpenHuman cutover

One OpenHuman integration owner exclusively edits
`crates/openhuman-core/src/agent/**`, related direct consumers, Cargo manifests,
tests, and the TinyAgents gitlink. Sequence the work internally:

1. pin the Gate 3 TinyAgents commit;
2. convert canonical tools/model metadata and the Tasks 2a–2d helpers;
3. complete Task 4's OpenHuman portion by replacing agent task-local reads
   with the explicit `OpenHumanRunContext` passed into the upstream harness and
   graph APIs, then delete each obsolete task-local module;
4. build and test the ten-capability host bundle;
5. switch routes one by one with route parity tests;
6. delete old paths, wrappers, remaining task-locals, and the temporary checker
   baseline.

Other agents may review but must not edit these paths during the cutover.

**Gate 4:** the boundary checker passes with no baseline; all direct-import
searches in Task 11 are empty; OpenHuman narrow and full suites pass; the audit
matrix matches the tree. The TinyAgents commit must already be reachable from
its upstream PR before the OpenHuman gitlink is published.

## Final validation

Run narrow tests after every task. Before any PR is ready, run:

```bash
cargo fmt --manifest-path vendor/tinyagents/vendor/tinytools/Cargo.toml --all -- --check
cargo test --manifest-path vendor/tinyagents/vendor/tinytools/Cargo.toml --workspace
cargo clippy --manifest-path vendor/tinyagents/vendor/tinytools/Cargo.toml --workspace --all-targets -- -D warnings
cargo fmt --manifest-path vendor/tinyagents/vendor/tinyinference/Cargo.toml --all -- --check
cargo test --manifest-path vendor/tinyagents/vendor/tinyinference/Cargo.toml --workspace
cargo clippy --manifest-path vendor/tinyagents/vendor/tinyinference/Cargo.toml --workspace --all-targets -- -D warnings
cargo fmt --manifest-path vendor/tinyagents/Cargo.toml --all -- --check
cargo test --manifest-path vendor/tinyagents/Cargo.toml --workspace
cargo clippy --manifest-path vendor/tinyagents/Cargo.toml --workspace --all-targets -- -D warnings
pnpm rust:layout
pnpm docs:generate
pnpm docs:check
pnpm typecheck
pnpm lint
pnpm i18n:check
cargo check --manifest-path Cargo.toml
cargo check --manifest-path crates/openhuman-app/Cargo.toml
pnpm debug rust
pnpm test
```

Run long CI-equivalent commands through `scripts/ci-cancel-aware.sh`; never
export `CARGO_TARGET_DIR`. Feature changes also require enabled/disabled builds
and `scripts/ci/check-feature-forwarding.mjs`. New root Rust tests need explicit
Cargo `[[test]]` entries.

Finally, `git diff --submodule=log` must show the expected nested commits; every
nested commit must be pushed with a ready canonical-upstream PR; OpenHuman must
contain no moved compatibility export; and durable architecture documentation
must describe the final host-only design.
