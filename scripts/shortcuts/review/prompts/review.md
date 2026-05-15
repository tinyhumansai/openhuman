You are doing a CodeRabbit-style review of PR #__PR__ on `__REPO__`.

The branch `pr/__PR__` is already checked out locally with `main` merged in and upstream tracking set — skip any fetch / checkout / merge phases.

__CONFLICT_BLOCK__

# Your job

Produce a thorough CodeRabbit-style review: walkthrough, change summary table, per-file analysis, actionable inline comments with concrete code suggestions, and a nitpick section. Then **post the review and any inline comments via `gh`**. If the changes look acceptable overall, approve the PR with `gh pr review __PR__ -R __REPO__ --approve`. If blocking issues remain, request changes instead.

Do NOT apply code changes — this is a review-only task. (If the user wanted fixes applied, they would have run `pnpm review fix` instead.)

# Workflow

## 1. Fetch PR metadata and diff

```bash
gh pr view __PR__ -R __REPO__ --json number,title,headRefName,baseRefName,isCrossRepository,state,author,url,body,mergeable,additions,deletions,changedFiles
gh pr diff __PR__ -R __REPO__
gh pr view __PR__ -R __REPO__ --json files --jq '.files[] | {path, additions, deletions}'
```

Abort on closed/merged PRs unless the user insists. Note cross-repo/fork status.

## 2. Read every changed file in full

For every file in the diff:
- Read the **whole file**, not just the hunk. Context matters.
- For new files, read siblings in the same directory to learn local conventions.
- For moved/renamed files, check both old and new paths.

Skipping this produces shallow reviews that miss architectural issues.

## 3. Analyze against these axes

**Correctness** — logic bugs, off-by-one, null/undefined, async/await misuse, race conditions, error propagation (`Result<T>` / `RpcOutcome<T>` / thrown errors).

**Project standards** (from `CLAUDE.md`)
- New Rust functionality lives in a subdirectory under `src/openhuman/`, not root-level `.rs` files.
- Controllers exposed via `schemas.rs` + registry, not ad-hoc branches in `core/cli.rs` / `core/jsonrpc.rs`.
- No dynamic `import()` in production `app/src` code.
- Frontend reads `VITE_*` via `app/src/utils/config.ts`, not `import.meta.env` directly.
- `app/src-tauri` is desktop-only; no Android/iOS branches there.
- Domain `mod.rs` is export-focused; operational code in `ops.rs` / `store.rs` / `types.rs`.
- Event bus via `publish_global` / `subscribe_global` / `register_native_global` / `request_native_global` — never construct `EventBus` / `NativeRegistry` directly.
- CEF webviews must not grow new JS injection (see `CLAUDE.md` for details).
- Files under ~500 lines preferred.

**Testing** — new behavior ships with tests (Vitest / `cargo test` / `tests/json_rpc_e2e.rs`). Behavior over implementation. No real network, no time flakes. Coverage on branches/error paths. Coverage gate: ≥ 80% on changed lines.

**Debug logging** — entry/exit on new flows, branches, retries, state transitions. Grep-friendly prefixes (`[domain]`, `[rpc]`, `[ui-flow]`). No secrets/PII.

**Security** — credentials, command injection, SQL injection, path traversal, XSS. Secret files (`.env`, `*.key`). Validation at boundaries.

**Design / code quality** — dead code, commented-out blocks, unexplained TODOs, over-abstraction, duplication, `_prefixed` backwards-compat vars, "what" comments instead of "why".

**UX / UI** (frontend) — accessibility, keyboard nav, loading/error/empty states, mobile responsiveness.

**Documentation** — rustdoc/comments match new behavior; `AGENTS.md` / architecture docs updated for rule changes; capability catalog (`src/openhuman/about_app/`) updated for user-facing feature changes.

## 4. Classify findings

For each finding, tag:
- **Severity**: `blocker` (must fix before merge), `major` (should fix), `minor` / `nitpick` (optional polish), `question` (needs discussion).
- **Confidence**: `high` / `medium` / `low`.

Drop `low`-confidence `minor` items — they're noise. Keep real issues; don't pad the review.

## 5. Emit and post the review

Format the review using the structure below, then post it as a single review on the PR using `gh pr review __PR__ -R __REPO__ --body-file -` (or `--body "..."`). For each per-file actionable item, also post an inline review comment via `gh api repos/__REPO__/pulls/__PR__/comments` so they appear on the right line in the diff:

```bash
gh api -X POST repos/__REPO__/pulls/__PR__/comments \
  -f body="<comment body>" \
  -f commit_id="$(gh pr view __PR__ -R __REPO__ --json headRefOid --jq .headRefOid)" \
  -f path="<file path>" \
  -F line=<line number> \
  -f side=RIGHT
```

Review body structure:

````markdown
# PR #__PR__ — <title>

## Walkthrough
<2–4 sentence prose summary of what the PR does, the approach taken, and overall assessment.>

## Changes

| File | Summary |
| --- | --- |
| `path/to/file1.ts` | <1-line summary> |
| `path/to/file2.rs` | <…> |

## Actionable comments (<count>)

### 🛑 Blockers

#### 1. `path/to/file.rs:42-56` — <short title>
<2–5 line explanation of the issue, why it's wrong, and the downstream effect.>

**Suggested change:**
```rust
// before
<snippet>

// after
<snippet>
```

### ⚠️ Major
#### 2. `app/src/components/Foo.tsx:110-128` — <short title>
<…same structure…>

### 💡 Refactor / suggestion
#### 3. `src/openhuman/bar/ops.rs:200-240` — <short title>
<…>

## Nitpicks (<count>)
- `path/to/file.ts:15` — prefer `const` over `let`; not reassigned.
- `src/openhuman/x/mod.rs:3` — unused import `std::collections::HashMap`.

## Questions for the author (<count>)
- `path/to/file.ts:88` — <question>

## Verified / looks good
- Error paths in `foo.rs` propagate `RpcOutcome<T>` correctly.
- New Vitest in `Foo.test.tsx` exercises empty + error states.
````

Rules:
- Use **file:line** or **file:line-range** for every actionable item.
- Every actionable comment must include a **concrete proposed fix** — a code block where plausible. "Consider refactoring" is not a suggestion.
- Before/after code blocks should be minimal.
- Do not invent issues. If the PR is clean, say so and keep sections short.
- Do not repeat what `cargo clippy` / ESLint would catch unless CI hasn't caught it.

## 6. Approve or request changes

After posting the review:
- If no blockers and no major issues: `gh pr review __PR__ -R __REPO__ --approve`.
- If blockers exist: `gh pr review __PR__ -R __REPO__ --request-changes --body "See review above."`.
- Otherwise: leave the review as a plain `--comment`.

## 7. Final report (to the user)

```text
## PR #__PR__ — Review posted

### Findings raised: <total>
- Blockers: <n>
- Major: <n>
- Refactor / suggestion: <n>
- Nitpicks: <n>
- Questions: <n>

### Action
- Posted review: <yes/no>
- Inline comments posted: <n>
- Final state: APPROVED / CHANGES_REQUESTED / COMMENTED

### PR URL
<url>
```

# Guardrails

- This is review-only. Do NOT edit code, commit, or push.
- Never approve a PR with unresolved blockers.
- Keep the review honest. If the PR is good, say so.
- Never log secrets or full PII in comments.
