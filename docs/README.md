# docs/

Internal maintainer documentation. Public and contributor docs live in
`docs/gitbooks/en/` (navigation in `docs/gitbooks/en/SUMMARY.md`); agent and contributor rules
live in `AGENTS.md` (`CLAUDE.md` is a symlink to it); per-domain design notes
sit beside the code as `crates/openhuman-core/src/<domain>/README.md`.

Every file in this directory is listed below. Links here are checked in CI by
the `markdown-link-check` job in `.github/workflows/pr-quality.yml`.

## Contributor onboarding

- `CONTRIBUTING-BEGINNERS.md` — step-by-step first-PR guide for newcomers;
  linked from the root `README.md` and `CONTRIBUTING.md`.
- `SUPPORT.md` — where to ask for help; routes each kind of problem to a
  GitHub Discussions category.
- `community/discussions.md` — how maintainers organize and triage those
  Discussions categories.

## Process checklists

The first two are referenced from `.github/PULL_REQUEST_TEMPLATE.md`.

- `TEST-COVERAGE-MATRIX.md` — feature-ID to test-path matrix. Feature IDs are
  validated against `scripts/feature-ids.json` by
  `scripts/check-coverage-matrix.mjs` in the `pr-quality` workflow; test paths
  are not validated, so keep them current by hand. Update rows in the same PR
  that changes a feature.
- `RELEASE-MANUAL-SMOKE.md` — manual smoke checklist run on every release cut;
  the only accepted substitute for a 🚫 matrix row.
- `prompt-evals.md` — the agent-prompt comprehension suite: the case list, how
  to run it, and how to read a regression. Coupled both ways to
  `scripts/prompt-eval.sh`, `scripts/prompt-eval/cases.json` and
  `tests/in_process/agent_prompt_comprehension_e2e.rs`.
- `JAPANESE-UI-VALIDATION.md` — the ja-JP UI pass: what to look at, and the
  layout failures Japanese text exposes that English does not.

## CI

- `ci-self-hosted.md` — the only explainer for the lane architecture: how
  `ci-fast.yml` and `ci-fast-hosted.yml` call `ci-lanes.yml`, what each lane
  covers, which checks land in which lane, and how the Hetzner EX63 microVM
  runners are routed. Read this before adding a check to CI.

## Engineering notes

Resource-footprint work for the embeddable core. The profiling binaries
(`library-profile`, `rss-bench`), their driver scripts and the benchmark
method (`profile/docs/library-benchmarking.md`) live in
[`tinyhumansai/openhuman-benchmarks`](https://github.com/tinyhumansai/openhuman-benchmarks), with the agent-scale, life-scenario
and onboarding stress suites. The core keeps only the `rss-bench` feature they
build against.

- `library-minimal-recipe.md` — the `--no-default-features --features
  "skills,flows"` recipe for a headless library build, with measurements.
- `dep-audit/<date>.md` — archived dependency-audit snapshots. These are
  machine-generated: `pnpm dep:audit` writes a fresh report to
  `target/dep-audit/REPORT.md`, and `pnpm dep:audit --snapshot` archives it
  here. Read `scripts/dep-audit/README.md` before acting on one, and date any
  claim you take from a snapshot.

## Specs and plans

Domain-local design notes live beside their code. Two exceptions sit here
because they coordinate several repositories at once and cannot be owned
accurately by one source directory.

- `specs/memory-v2.md` — the Memory v2 contract: the model, scope, the
  per-turn lifecycle, background jobs, config, the agent tool, the RPC table
  and the UI. The most referenced document in the tree: the Rust core, the
  embed facade, the frontend, a Playwright spec, a root integration test and
  five gitbooks pages all cite it. Treat it as current architecture, not a
  plan.
- `specs/agent-runtime-upstream-boundary.md` — the normative ownership
  boundary for making the core a host of the TinyAgents harness rather than a
  second agent runtime, with deletion criteria.
- `plans/` — the agent-runtime migration work plans, plus one dated
  measurement. `plans/README.md` carries the per-plan landed/outstanding
  status, verified against the tree, and the commands to re-check it. Read
  that before trusting any individual plan's own status line.

## Translated product READMEs

Translations of the root `README.md`, linked from its language bar:
`README.ar.md`, `README.de.md`, `README.ja-JP.md`, `README.ko.md`,
`README.tr.md`, `README.ur-pk.md`, `README.zh-CN.md`.

Nothing compares a translation's structure against the root README, so they
drift silently; `pnpm i18n:check` covers UI strings only. Check the root
README's section list by hand when you touch one.
