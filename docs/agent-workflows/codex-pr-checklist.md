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

## Duplicate PR Cleanup

Canonical PR rule: keep the PR whose head branch is the active issue branch and whose head commit contains the intended final work. If two PRs contain equivalent work, keep the PR already linked from Linear or already carrying useful review/CI history. If neither has history, keep the older PR number to reduce churn. Do not choose by recency alone; compare the heads first and move any useful commits or PR body details onto the canonical PR before closing the duplicate.

Lightweight comparison and close recipe:

```bash
BASE_REPO=tinyhumansai/openhuman # or jwalin-shah/openhuman for fork-targeted PRs
BASE_REMOTE=upstream             # remote matching BASE_REPO
KEEP=123                         # canonical PR number
CLOSE=124                        # duplicate PR number

gh pr view "$KEEP" --repo "$BASE_REPO" --json number,url,state,baseRefName,headRefName,headRefOid
gh pr view "$CLOSE" --repo "$BASE_REPO" --json number,url,state,baseRefName,headRefName,headRefOid

git fetch "$BASE_REMOTE" "refs/pull/$KEEP/head:refs/tmp/pr-$KEEP"
git fetch "$BASE_REMOTE" "refs/pull/$CLOSE/head:refs/tmp/pr-$CLOSE"
git log --oneline --left-right --cherry-pick "refs/tmp/pr-$KEEP...refs/tmp/pr-$CLOSE"
git diff --stat "refs/tmp/pr-$KEEP...refs/tmp/pr-$CLOSE"
git diff --name-status "refs/tmp/pr-$KEEP...refs/tmp/pr-$CLOSE"

gh pr close "$CLOSE" --repo "$BASE_REPO" --comment "Closing as a duplicate of #$KEEP for <ISSUE-KEY>. Kept #$KEEP because <canonical reason>."

git update-ref -d "refs/tmp/pr-$KEEP"
git update-ref -d "refs/tmp/pr-$CLOSE"
```

If the duplicate has unique, useful commits, cherry-pick or manually port them onto the canonical branch, push that branch, then repeat the comparison before closing anything.

Record the cleanup in Linear before handoff:

- Canonical PR kept: `<PR URL>` with head SHA `<sha>`.
- Duplicate PR closed: `<PR URL>` with reason.
- Comparison evidence: command summary, for example `git log --left-right --cherry-pick` showed no unique commits in the duplicate.
- Any moved commits or remaining blockers.

Pattern from the SYM-92 incident: two agent launches produced overlapping PRs for the same Linear issue. The cleanup was to compare both heads, keep the PR that represented the active issue branch/final handoff, close the stale duplicate with a pointer to the kept PR, and record both PRs in Linear. Treat that as the reusable pattern; the kept PR is still selected by branch, head diff, and handoff evidence for the current issue.

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
