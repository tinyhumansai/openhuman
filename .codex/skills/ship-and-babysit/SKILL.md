---
name: ship-and-babysit
description: "End-to-end PR shipping workflow for tinyhumansai/openhuman: commit local changes, push to the user's fork, open or reuse a PR against main, then babysit CI and CodeRabbit feedback until the PR is green and clean. Use when the user asks to ship, open a PR, monitor CI, address review comments, or 'babysit' a branch."
---

# Ship and Babysit

Use this skill for `tinyhumansai/openhuman` when the user wants a branch shipped end to end:

- commit the local changes
- push the branch to the user's fork
- open or reuse a PR against `tinyhumansai/openhuman:main`
- proactively run likely merge-gate validation and start fixing issues immediately
- monitor CI and review feedback in a polling loop without waiting idly for every check to finish
- address actionable review comments and push follow-up fixes
- stop only when the PR is green and clean

## Preconditions

- Work from the repository root.
- Follow repo rules from `AGENTS.md`, including validation and PR checklist requirements.
- Assume `origin` is the user's writable fork and `upstream` points to `tinyhumansai/openhuman`.
- Resolve the fork owner once near the start and reuse it:
  - `FORK_OWNER=$(git remote get-url origin | sed -E 's#.*[:/]([^/]+)/[^/]+(\.git)?$#\1#')`
- If `origin` resolves to `tinyhumansai`, stop and ask the user to add a fork remote. Never push branches to upstream.
- If work starts on local `main`, create a new descriptive branch before committing so the changes leave `main` immediately.
- Never push directly to `main`.
- Never push to `upstream`.
- Never amend or rewrite commits that are already pushed unless the user explicitly asks for it.
- Never bypass hooks for breakage introduced by your own changes.
- Default to autonomous execution. Do not stop to ask the user process questions when a reasonable safe default exists.
- Only ask the user a question when the workflow is genuinely blocked by missing access, missing credentials, or an irreversible choice that cannot be inferred from repo context.

## Workflow

### Phase 1: Inspect and Commit

1. Inspect the branch before changing anything. Prefer parallel reads:
   - `git status --short`
   - `git diff --stat`
   - `git diff --cached --stat`
   - `git log --oneline --decorate -n 12`
2. Determine the current branch:
   - `git rev-parse --abbrev-ref HEAD`
3. Confirm the branch normally follows `feat/`, `fix/`, `refactor/`, `chore/`, `docs/`, or `test/`.
   - If the current branch is `main`, create a new descriptive branch immediately and continue there.
   - If the name does not follow convention and it is already a non-`main` branch, keep using it unless it is still local and trivially safe to rename without disrupting a pushed branch.
4. If there are uncommitted changes, carry them onto the new branch before doing anything else so local `main` stays free of agent-authored commits.
5. If there are uncommitted changes, run the smallest meaningful local validation for the touched area before committing.
6. Stage only relevant files and create a focused conventional commit message.
7. If there are no local changes, continue without creating a commit.

### Phase 2: Push

1. Push the current branch to `origin`.
2. If upstream tracking is missing, push with `-u`.
3. If a pre-push hook fails on your own changes, fix the issue and push again.
4. If a pre-push hook fails only because of unrelated pre-existing breakage, push with `--no-verify` and record that explicitly in the PR body.
5. After every later fix commit in the babysit loop, push again. Do not stop at a local commit.

### Phase 3: Open or Reuse the PR

1. Verify remotes with `git remote -v` and confirm `upstream` points at `tinyhumansai/openhuman`.
2. Check for an existing PR for the exact branch:
   - `gh pr list --repo tinyhumansai/openhuman --head <fork-owner>:<branch> --state open --json number,url`
3. If a PR exists, capture its number and URL and reuse it.
4. If no PR exists:
   - inspect `git log main..HEAD` and `git diff main...HEAD`
   - fill `.github/PULL_REQUEST_TEMPLATE.md` exactly
   - create the PR against `tinyhumansai/openhuman:main` with `--head <fork-owner>:<branch>`
5. Print the PR URL to the user.
6. Immediately after opening or reusing the PR, start proactive validation based on the touched area instead of waiting for remote CI to finish:
   - run the smallest set of likely merge-gate commands that cover the changed code
   - prioritize fast failure detectors first, such as format, typecheck, lint, targeted tests, and cargo checks relevant to touched files
   - fix locally discovered failures right away, then commit and push again before the next CI poll

### Phase 4: Babysit Loop

Run an explicit poll loop until the PR is green and clean. Do not treat this as a one-shot status check, and do not sit idle waiting for all checks to complete before acting.

- Poll about every 5 minutes.
- Stay in the loop for up to 12 ticks, about 60 minutes total.
- If the environment does not support durable wakeups, remain in-session and use repeated polling with `sleep 270`.
- On each tick, post a short progress update to the user.
- Between ticks, prefer useful work over passive waiting:
  - inspect completed failures as soon as they appear
  - inspect review comments and unresolved threads immediately
  - run likely local validations on changed areas while remote checks are still pending
  - push fixes as soon as they are ready instead of batching them behind the full CI timeline

Each tick:

1. Fetch CI status:
   - `gh pr checks <pr-number> --repo tinyhumansai/openhuman --json name,state,link,description`
2. Treat `PENDING` as still in progress. Do not claim success while checks are still running.
3. If any completed check is `FAILURE` or `CANCELLED`:
   - if the `link` is a GitHub Actions run URL, extract the run id and inspect failing logs with `gh run view <id> --log-failed --repo tinyhumansai/openhuman`
   - otherwise work from the check name, state, and description
   - make the smallest correct fix
   - rerun targeted validation
   - commit
   - push
4. If checks are still mostly `PENDING`, do not wait for the whole matrix to finish before taking action:
   - inspect the changed files and recent commit diff
   - run the most relevant local merge-gate commands proactively
   - fix any locally reproduced failure immediately
   - commit and push as soon as the fix is validated
5. Fetch PR review comments:
   - `gh api repos/tinyhumansai/openhuman/pulls/<pr-number>/comments --paginate`
6. Fetch issue-level PR comments:
   - `gh api repos/tinyhumansai/openhuman/issues/<pr-number>/comments --paginate`
7. Inspect review threads via GraphQL, not just flat comments, so unresolved discussions do not slip through:
   - query `reviewThreads` with pagination until `hasNextPage` is false
8. Specifically inspect bot feedback from `coderabbitai` and `coderabbitai[bot]`, but also check for human actionable review comments.
9. For each actionable review comment or unresolved review thread:
   - read the referenced file and line
   - apply the smallest correct fix
   - rerun targeted validation
   - commit
   - push
10. For incorrect, stale, or out-of-scope review feedback:
   - reply in the existing review thread with concrete reasoning
   - do not open a new unrelated review
   - resolve or dismiss only when the reasoning is explicit and the platform supports it
11. After addressing a review thread, resolve it through the GitHub review-thread API when appropriate.
12. Track whether new issue-level CodeRabbit comments appeared since the previous tick so the loop does not exit while fresh bot feedback is waiting.
13. Exit the loop only when all of these are true:
   - all required checks are `SUCCESS`
   - no unresolved actionable review threads remain
   - no new actionable CodeRabbit issue comments remain
   - the latest fixes are committed and pushed to the PR branch

If the loop reaches the hard cap, stop and report the PR URL, current CI snapshot, and any unresolved review threads or comments.

## Useful Checks

- `pnpm typecheck`
- `pnpm lint`
- `pnpm format:check`
- `pnpm test:unit`
- `cargo check --manifest-path Cargo.toml`
- `cargo check --manifest-path app/src-tauri/Cargo.toml`
- `pnpm test:rust`

Prefer targeted test commands when the touched area is narrow, but do not claim validation passed if a command was not run.

## Notes

- Do not merge the PR unless the user explicitly asks.
- Reuse an existing PR when one already exists for the branch.
- Always push follow-up commits so the PR actually updates after fixes.
- If invoked from `main`, branch first, then ship. Do not make the user clean up agent commits from `main`.
- Checking `gh pr checks --watch` once is not sufficient babysitting. The skill should actively re-poll CI and review surfaces until the exit condition is met.
- The skill should not ask the user for confirmation about routine workflow choices such as branch naming, whether to start fixing CI, or whether to act on obvious actionable failures.
- The skill should assume the user wants active babysitting: inspect, fix, commit, and push continuously until blocked or green.
- Review handling must include:
  - PR review comments
  - issue-level PR comments
  - unresolved review threads
- If CI or review surfaces reveal unrelated pre-existing breakage, call it out clearly and avoid masking it as fixed.
- If GitHub auth, remotes, or branch protection do not allow the workflow, report the exact blocker and stop at the first blocked step.

## Invocation Hints

- `Use $ship-and-babysit for this branch`
- `Ship this and babysit the PR`
- `Open the PR and stay on CI until it is green`
