# Codex PR Checklist

Use this checklist for Codex web sessions, Linear-launched implementation agents, and any other remote agent that opens OpenHuman PRs.

## Required Preflight

Run the scriptable preflight wrapper (recommended):

```bash
node scripts/codex-pr-preflight.mjs --strict-path --lightweight
```

Use `--lightweight` when you only need environment/repo checks plus changed-surface validation recommendations (it skips heavier runtime validations).

Run this before editing files:

```bash
pwd
git status --porcelain
git branch --show-current
git remote -v
test -f AGENTS.md
test -f docs/src/README.md
test -f Cargo.toml
test -f app/package.json
```

Expected repository path in Codex web is `/workspace/openhuman`. If the checkout is missing or the command shows another project, stop immediately and report the environment binding problem. Do not edit files in the wrong repository.

## Launch Trigger Rule

Use exactly one Codex trigger per Linear issue.

Preferred launch pattern:

```md
@Codex use the Codex environment for jwalin-shah/openhuman.

Work issue <ISSUE-KEY>.
Expected path: /workspace/openhuman.
Start from latest origin/main.
Create branch codex/<ISSUE-KEY>-<short-title>.
Follow docs/agent-workflows/codex-pr-checklist.md exactly.
Do not open duplicate PRs. If validation is blocked, report exact command and error in the PR body and Linear.
```

Do not also set `delegate: Codex` when posting an explicit `@Codex` launch comment. Linear delegate metadata can start its own Codex thread, so combining both mechanisms can double-trigger the same issue.

If using `delegate: Codex` as the only trigger for an integration that requires it, do not add an `@Codex` comment. Record in the issue which trigger was used.

## Branch And PR Rules

- Start from latest `origin/main` unless the Linear issue explicitly says otherwise.
- Use one branch and one PR per Linear issue.
- Name branches `codex/<ISSUE-KEY>-<short-title>`.
- Do not open duplicate PRs for the same issue. If a retry is needed, update the existing PR branch or close the stale duplicate and state which PR is canonical.
- PRs should target `jwalin-shah/openhuman:main` unless upstream permissions allow `tinyhumansai/openhuman:main`.

## Validation Before PR

Run the smallest checks that prove the changed surface, plus the relevant merge gates:

```bash
# Always run for app or docs-visible app changes
pnpm --filter openhuman-app format:check
pnpm typecheck

# Focused app tests for changed TS/React behavior
pnpm --dir app exec vitest run <changed-test-files> --config test/vitest.config.ts

# Root Rust changes
cargo fmt --manifest-path Cargo.toml --all --check

# Tauri shell changes
cargo fmt --manifest-path app/src-tauri/Cargo.toml --all --check
```

For Rust behavior changes, prefer focused tests through the repo wrappers where available:

```bash
pnpm debug rust <test-filter>
```

If a command cannot run because the environment lacks vendored files or system packages, do not claim it passed. Copy the exact command and blocker into the PR body.

## Refactor Parity Rules

For behavior extraction and architecture refactors:

- Identify the old guard order, fallback order, dispatch contract, or public API being preserved.
- Add focused parity tests when the behavior can be tested without broad integration setup.
- Do not reorder guards, fallback layers, RPC methods, or dispatch paths unless the issue explicitly asks for a behavior change.
- When adding a drift guard, verify it checks the source of truth as it exists in this repo. Do not assume generated strings are written literally in source files.

## PR Body Requirements

Every AI-authored PR must include:

- Linear issue key and URL.
- Branch name.
- Commit SHA.
- Files changed summary.
- Validation commands run.
- Validation commands blocked, with exact error text.
- Behavior intentionally changed, or `No intended behavior change`.
- Any duplicate/stale PRs that were closed or superseded.

## Review Before Handoff

Before handing off:

- Re-check GitHub CI status for the PR.
- Pull failed check logs before guessing.
- Fix format/type/test failures that are local to the PR.
- Leave broad system dependency or environment failures as explicit blockers.
- Update the Linear issue with PR URL, commit SHA, validations, and blockers.
