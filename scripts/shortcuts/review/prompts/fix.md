You are finishing PR #__PR__ on `__REPO__`.

The branch `pr/__PR__` is already checked out locally with `main` merged in and upstream tracking set — skip any fetch / checkout / merge phases.

`pushRemote` is already wired to the contributor's fork — `git push` from this branch will land directly on the PR's head branch (`__HEAD_REPO__:__HEAD_BRANCH__`), updating the actual PR. Do not push to `origin` explicitly; plain `git push` is correct.

__CONFLICT_BLOCK__

# Your job

Two phases, both required:

1. **Review and apply fixes**: Do a CodeRabbit-style review of the diff, identify real issues (blockers, major, suggestions), and apply the fixes directly to the working tree. Also address every existing reviewer/bot comment on the PR (CodeRabbit, maintainers, inline threads) — apply each actionable item rather than describing it.
2. **Quality suite, commit, push**: Run formatters / typecheck / lint / tests, commit everything in focused commits, and push back to the PR branch.

**Your job is to finish the PR, not to report on it.** A response that only lists what *should* be done is a failure mode.

# Workflow

## 0. Sanity check the working state

```bash
git status --short                  # should be empty or only your own staged work
git branch --show-current           # should be pr/__PR__
git rev-parse --abbrev-ref @{u}     # upstream must be set
git log --oneline -5
```

If working tree is dirty in a way you didn't expect, stop and ask — never stash/discard.

## 1. Fetch PR metadata and existing review comments

```bash
gh pr view __PR__ -R __REPO__ --json number,title,headRefName,headRepositoryOwner,headRepository,baseRefName,isCrossRepository,state,author,url,body,mergeable,statusCheckRollup
gh pr diff __PR__ -R __REPO__

# Top-level reviews (CodeRabbit summaries, maintainer overall reviews)
gh pr view __PR__ -R __REPO__ --json reviews --jq '.reviews[] | {author: .author.login, state: .state, body: .body, submittedAt: .submittedAt}'

# Inline code review comments
gh api repos/__REPO__/pulls/__PR__/comments --paginate

# General PR conversation comments
gh api repos/__REPO__/issues/__PR__/comments --paginate
```

Confirm PR is **open**. Abort on closed/merged unless the user says otherwise.

For cross-repo forks where the local `pr/__PR__` was pushed to your own `origin` (not the contributor's fork), pushes will update your origin copy — **not the actual PR**. Flag this clearly in the final report.

## 2. Read every changed file in full

For every file in the diff, read the **whole file** (not just the hunk). Context matters. Read siblings for new files; check both paths for moves/renames. Skipping this produces shallow fixes.

## 3. Analyze + classify each existing comment and your own findings

Run a CodeRabbit-style review against these axes:

**Correctness** — logic bugs, off-by-one, null/undefined, async/await misuse, race conditions, error propagation (`Result<T>` / `RpcOutcome<T>`).

**Project standards** (from `CLAUDE.md`)
- New Rust functionality under `src/openhuman/<domain>/`, not root-level `.rs` files.
- Domain exposure via `schemas.rs` + registry — not ad-hoc branches in `src/core/cli.rs` / `src/core/jsonrpc.rs`.
- No dynamic `import()` in production `app/src` code.
- Frontend `VITE_*` reads via `app/src/utils/config.ts`.
- `app/src-tauri` is desktop-only.
- Event bus via `publish_global` / `subscribe_global` / `register_native_global` / `request_native_global` — never construct `EventBus` / `NativeRegistry` directly.
- CEF webviews must not grow new JS injection.
- Debug logging on new flows (entry/exit, branches, retries); grep-friendly prefixes; no secrets/PII.
- Files preferably ≤ ~500 lines.
- Capability changes update `src/openhuman/about_app/`.

**Testing** — new behavior ships with tests; coverage gate is ≥ 80% on changed lines.

**Security** — credentials, command injection, SQL injection, path traversal, XSS.

Classify each existing comment and each new finding:
- `actionable-trivial` — typo, rename, formatting, missing import: fix directly.
- `actionable-non-trivial` — logic/architecture/test gap: fix if direction is unambiguous; otherwise defer.
- `already-addressed` — current code satisfies it.
- `stale-outdated` — no longer applies.
- `disagree` / `defer-human` / `question` — surface in the final report and post back as a PR comment via `gh api`; never silently dismiss.

## 4. Apply fixes (REQUIRED)

Apply every `actionable-trivial` and clearly-directed `actionable-non-trivial` fix. Re-read surrounding code before each edit (state may have drifted since the comment was written, especially for CodeRabbit `suggestion` blocks).

One logical concern per commit:

```text
fix(<area>): <what changed> (addresses @<reviewer> on <file>:<line>)
refactor(<area>): <what changed>
test(<area>): <what added>
docs(<area>): <what changed>
chore(pr-fix): apply formatting
chore(pr-fix): lint autofix
```

Skip anything you choose not to apply (with reason captured for the final report). Don't expand scope.

## 5. Run the quality suite

Run in parallel where independent. Skip suites unrelated to the diff; always run formatters + typecheck/lint when code changed.

```bash
# Frontend (if app/ changed)
cd app && pnpm compile
cd app && pnpm lint
cd app && pnpm format       # auto-fix
cd app && pnpm test:unit

# Rust (if src/ or app/src-tauri changed)
cargo fmt --manifest-path Cargo.toml
cargo check --manifest-path Cargo.toml
cargo check --manifest-path app/src-tauri/Cargo.toml
cargo test --manifest-path Cargo.toml
```

If a test fails on apparent flake, rerun once. If it still fails, stop and report.

## 6. Commit any auto-fixes

- `pnpm format` / `cargo fmt` changes → `chore(pr-fix): apply formatting`.
- Non-trivial lint autofixes → `chore(pr-fix): lint autofix`.
- `git status --short` must be empty before push.
- Never `--no-verify` unless a pre-push hook fails on pre-existing breakage unrelated to your changes — in that case, push with `--no-verify` and note it in the final report.
- Never amend published commits. Never force-push without explicit user approval.

## 7. Push (REQUIRED)

```bash
git status --short    # must be empty
git push              # pushes to the contributor's fork (__HEAD_REPO__:__HEAD_BRANCH__) — updates the PR
```

`pushRemote` is already configured to the contributor's fork, so plain `git push` updates the actual PR. **Do not** push to `origin` or invent a different remote.

If rejected (non-fast-forward — the contributor pushed something while you worked): `git pull --rebase` then push. **Never** force-push without explicit user approval.

If the push fails with a permissions error (you lack write access to the contributor's fork): stop, do NOT fall back to `origin`, and report the situation — the user needs to either get access or have the contributor pull the changes themselves.

## 8. Post deferred / disagree / question items back to the PR

For every item you classified as `disagree`, `defer-human`, or `question` — post it as an inline review comment via `gh api` so nothing is lost in chat:

```bash
gh api -X POST repos/__REPO__/pulls/__PR__/comments \
  -f body="<your reply>" \
  -f commit_id="$(gh pr view __PR__ -R __REPO__ --json headRefOid --jq .headRefOid)" \
  -f path="<file path>" \
  -F line=<line number> \
  -f side=RIGHT
```

## 9. Final report (to the user)

```text
## PR #__PR__ - <title>
Branch: pr/__PR__  PR head: <headRefName>  Base: <baseRefName>  Author: <login>

### Preconditions
- Working tree clean at start: yes/no
- Branch / upstream verified: yes/no
- Cross-repo fork: yes/no — push target: <origin/<branch> | contributor-fork>

### Review comments processed (<count>)
- @<reviewer> on <file>:<line> - <one-line> -> fixed / already addressed / deferred / disagree

### New findings raised (<count>)
- <severity> <file>:<line> - <one-line> -> fixed / deferred

### Standards pass
- pass/warn/fail items with file:line

### Checks
| Check | Result |
|---|---|
| typecheck | pass/fail |
| lint | pass/fail (N autofixes) |
| format | pass |
| unit tests | <passed>/<total> |
| cargo check (core) | pass/fail |
| cargo check (tauri) | pass/fail |
| cargo test | <passed>/<total> |

### Commits pushed
- <sha> <subject>

### Outstanding human items
- <list, or none>

### PR
<url>
```

# Guardrails

- **Never** push to `main`, force-push, skip hooks (except documented pre-existing-breakage case), amend published commits, or run destructive git commands without explicit user approval.
- **Never** commit secrets (`.env`, `*.key`, credentials).
- If the working tree is dirty at start in unexpected ways, **stop** — don't stash.
- If tests flake, rerun once; if still failing, report rather than loop.
- Stay on the PR branch; never accidentally commit to `main`.
- Keep findings honest. If the PR is clean, say so. Don't pad with invented issues.
