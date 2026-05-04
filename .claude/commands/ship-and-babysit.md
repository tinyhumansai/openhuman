---
description: Commit, push to origin (fork), open PR to tinyhumansai/openhuman:main, then poll every ~5min for CodeRabbit comments and CI failures, resolve them, and exit when clean.
allowed-tools: Bash, Read, Edit, Write, Agent, Skill
---

You are running an end-to-end ship-and-babysit flow for the **openhuman** repo. Follow these phases in order. Be concise in user-facing text — one short sentence per phase transition is enough.

Repo facts (from `CLAUDE.md`):
- Upstream: `tinyhumansai/openhuman` (not a fork). PRs target **`main`**.
- Push branches to **`origin`** (the user's own fork of `tinyhumansai/openhuman`). Treat `upstream` as fetch-only.
- PRs are opened with `--head <fork-owner>:<branch>` against `tinyhumansai/openhuman:main`.
- PR template: `.github/PULL_REQUEST_TEMPLATE.md`. Issue templates under `.github/ISSUE_TEMPLATE/`.

**Resolve the fork owner once at the start** and reuse it for the rest of the flow:
```bash
FORK_OWNER=$(git remote get-url origin | sed -E 's#.*[:/]([^/]+)/[^/]+(\.git)?$#\1#')
```
The flow is **fork-only**: `origin` must be the user's fork. If `origin` resolves to `tinyhumansai` (the upstream org), stop and ask the user to add a fork remote — never push branches to the upstream repo.

## Phase 1 — Commit

1. Run `git status`, `git diff` (staged + unstaged), and recent `git log` in parallel to understand pending changes and the repo's commit message style.
2. If there are no changes to commit AND the branch is already pushed AND a PR already exists, skip to Phase 4.
3. If there are uncommitted changes, stage relevant files (avoid secrets / large binaries / `.env`), then create a commit using a conventional prefix (`feat:`, `fix:`, `refactor:`, `chore:`, `docs:`, `test:`). Use a HEREDOC for the message.
4. Never use `--no-verify` to bypass commit hooks for your own changes. If a hook fails on your changes, fix the underlying issue and create a NEW commit (do not amend pushed commits).

## Phase 2 — Push

1. Determine current branch with `git rev-parse --abbrev-ref HEAD`. Confirm it follows the `feat/|fix/|refactor/|chore/|docs/|test/` prefix convention. Never push directly to `main`. If the branch doesn't match the convention, stop and ask the user to either rename it or confirm the deviation — don't auto-rename pushed branches.
2. Push to **`origin`** with `-u` if upstream tracking is missing. Never push to `upstream`. Never force-push to `main`.
3. **Pre-push hook policy** (per `CLAUDE.md`): if a pre-push hook fails on something unrelated to your changes (pre-existing breakage on `main` in code you didn't touch), push with `--no-verify` and call it out in the PR body. If the hook fails on your own changes, fix and re-push. Don't ask — just do the right thing and tell the user what you did.

## Phase 3 — Open PR

1. Verify upstream remote with `git remote -v`. It should point at `tinyhumansai/openhuman`. If missing, ask the user before adding it.
2. Check whether a PR already exists for this branch:
   `gh pr list --repo tinyhumansai/openhuman --head <fork-owner>:<branch> --state open --json number,url`
   - **If a PR exists**, capture its `number` and `url`, print the URL, skip steps 3–5, and proceed straight to Phase 4 with that PR#.
3. If none exists, draft a title (<70 chars) and a body that follows `.github/PULL_REQUEST_TEMPLATE.md` exactly. Inspect commits with `git log main..HEAD` and the diff with `git diff main...HEAD` to write the summary. If you bypassed a pre-push hook, note it in the PR body.
   - When filling the Submission Checklist, write each item as `- [ ] N/A: <reason>` (the item text MUST start with `N/A:` for `scripts/check-pr-checklist.mjs` to count it as satisfied; trailing `— N/A: ...` won't match), or `- [x] <text>` for genuinely checked items.
4. Create the PR:
   ```bash
   gh pr create --repo tinyhumansai/openhuman --base main --head <fork-owner>:<branch> \
     --title "..." --body "$(cat <<'EOF'
   ...template-filled body...
   EOF
   )"
   ```
5. Add appropriate labels/type if conventional for this repo.
6. Capture the PR number and URL — you will need them in Phase 4. Print the URL to the user.

## Phase 4 — Babysit loop (~5 minutes)

Repeat the following loop until the exit condition is met. Use `ScheduleWakeup` to pace at **270s** (stays inside the prompt-cache window) — re-enter this phase each tick by passing the same `/ship-and-babysit` invocation back as the prompt.

**Hard cap: 12 ticks (~60 minutes).** After that, stop the loop and ask the user, including PR URL, current CI snapshot, and any unresolved CodeRabbit threads. Maintain an explicit `tickCount` that increments by 1 on every loop entry (regardless of whether you commit or only wait on CI), and pass it through in the `ScheduleWakeup` `reason` (e.g. `"tick 5/12: waiting on CI for PR #1115"`) so the counter is visible across ticks and can't drift if a tick produces no commits.

Each tick:

1. **Fetch CI status**:
   `gh pr checks <PR#> --repo tinyhumansai/openhuman --json name,state,link,description`
   - `gh pr checks --json` returns a `link` field (an Actions URL like `…/actions/runs/<id>/job/<jobId>`), not a run id directly. Extract the run id with a regex that's robust to trailing slashes (`sed -nE 's#.*/actions/runs/([0-9]+)/.*#\1#p'`) — positional `awk -F/` is brittle when the URL has a trailing slash. Or skip URL parsing entirely and call `gh run list --repo tinyhumansai/openhuman --branch <branch> --json databaseId --limit 1 --jq '.[0].databaseId'`.
   - If any check is `FAILURE` or `CANCELLED`, branch by check type: when `link` matches `/actions/runs/<id>/` (Actions-backed), extract `<id>` and fetch logs with `gh run view <id> --log-failed --repo tinyhumansai/openhuman`; when it doesn't (e.g. the `CodeRabbit` virtual check or any other status posted directly via the Checks API without an Actions run), skip `gh run view` and work from the `name`/`state`/`description` fields plus any review comments. Then fix the underlying issue: edit code, commit (conventional prefix), push to `origin`. Do NOT skip hooks or disable failing tests to make CI green.
   - For local repro of common failures before pushing fixes:
     - Frontend: `pnpm typecheck`, `pnpm lint`, `pnpm format:check`, `pnpm test:unit`.
     - Rust: `cargo check --manifest-path Cargo.toml`, `cargo check --manifest-path app/src-tauri/Cargo.toml`, `pnpm test:rust`.
     - Coverage gate is **≥ 80% on changed lines** (`.github/workflows/coverage.yml`) — if coverage fails, add tests for changed lines, not just happy path.
2. **Fetch CodeRabbit review comments**:
   `gh api repos/tinyhumansai/openhuman/pulls/<PR#>/comments --paginate`
   Filter for comments authored by `coderabbitai` / `coderabbitai[bot]`. Also check issue-level comments: `gh api repos/tinyhumansai/openhuman/issues/<PR#>/comments --paginate`.
   - For each unresolved CodeRabbit suggestion: read the file/line referenced and apply the fix if it is correct and in scope. If a suggestion is wrong or out of scope, reply *inside the existing thread* (so the reply attaches to the same conversation, not a brand-new review) before resolving:
     ```bash
     gh api repos/tinyhumansai/openhuman/pulls/comments/<comment_id>/replies \
       -X POST \
       -f body='**Dismissed:** <reason>'
     ```
     (`<comment_id>` is the top-level review-comment id from `gh api repos/tinyhumansai/openhuman/pulls/<PR#>/comments`. `POST /pulls/<PR#>/reviews` would create a *new* review thread, not a reply.)
   - After fixing, commit and push to `origin`.
   - Mark the corresponding review thread as resolved via the GraphQL API:
     ```bash
     gh api graphql -f query='mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{isResolved}}}' -f id=<threadId>
     ```
     To list thread IDs (paginated — `reviewThreads` caps at 100 per page, so loop on `pageInfo.hasNextPage` / `endCursor` and feed back as `$cursor` until exhausted, otherwise threads past page 1 silently slip past the exit condition):
     ```bash
     gh api graphql -f query='query($owner:String!,$repo:String!,$num:Int!,$cursor:String){repository(owner:$owner,name:$repo){pullRequest(number:$num){reviewThreads(first:100, after:$cursor){pageInfo{hasNextPage endCursor} nodes{id isResolved comments(first:1){nodes{author{login} body}}}}}}}' -F owner=tinyhumansai -F repo=openhuman -F num=<PR#> -F cursor=
     ```
3. **Exit condition** — stop the loop when ALL of these are true:
   - All required checks are `SUCCESS`. `PENDING` keeps the loop running, no exceptions — no "green" claim while CI is mid-run.
   - No unresolved CodeRabbit review threads remain.
   - No new CodeRabbit issue comments since the last tick that request changes. Track this by remembering the highest CodeRabbit issue-comment `id` seen on the previous tick (the GitHub issue-comment id is monotonic) and only treating ids strictly greater than that marker as new on the current tick.
   When the exit condition holds, do NOT call `ScheduleWakeup` — return a final one-line summary with the PR URL and current status.
4. **Pacing**: if exiting, stop. Otherwise call `ScheduleWakeup` with `delaySeconds: 270`, `prompt: "/ship-and-babysit"`, and a specific `reason` like "waiting on CI for PR #123" or "applied 2 CodeRabbit fixes, re-checking".

## Guardrails

- Never push to `upstream` (`tinyhumansai/openhuman`) — only to `origin` (the user's fork). Treat upstream as fetch-only.
- Never force-push to `main`. Never amend pushed commits.
- Never use `--no-verify` to bypass hooks failing on your own changes. The only sanctioned bypass is a pre-push hook failing on pre-existing unrelated breakage — call it out in the PR body when you do.
- Never resolve a CodeRabbit thread without actually addressing it (or replying with a reasoned dismissal).
- If you hit a blocker that needs human input (auth failure, ambiguous CodeRabbit suggestion, conflicting feedback, merge conflict, vendored `tauri-cli` missing), stop the loop and ask the user instead of guessing.
- Do not merge the PR. Stop at "green and clean".
