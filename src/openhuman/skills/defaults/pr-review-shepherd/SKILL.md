# PR Review Shepherd

Drive a single open GitHub PR all the way to **ready-for-merge** — CI green, every actionable reviewer/bot comment addressed, approvals in. This is autonomous Phase-6 work: iterate the **check → fix → push → re-check** loop until both gates close, or surface a real blocker and stop.

## Tool split — Composio for GitHub state, local git for the working tree

GitHub state operations — PR details, comments (top-level and inline review), check runs, status rollups, comment replies, labels — go through Composio via `composio_execute({tool: "GITHUB_<ACTION>", arguments: {...}})`. The working tree — clone the fork branch, edit files, run tests, commit, force-with-lease push to the fork — stays on local `git`. Composio is the single authoritative GitHub identity (the user's connected account, gated by the skill's `[github]` preflight); the local working tree is where the actual fix lands. Do **not** shell out to `gh` for state operations — the preflight checked Composio but not gh's credential store, so a `gh` call may silently route through a different account.

## When this skill is "done"

Both must hold:
1. **CI green** — every required check on PR `#{pr}` is `success` (or explicitly waived by a maintainer in the thread).
2. **All actionable comments resolved** — every comment from a human reviewer or bot (CodeRabbit, Codecov, etc.) is either (a) addressed by a follow-up commit AND replied to on the thread, or (b) intentionally deferred with a one-line reason replied on the thread.

Also stop if the PR is **merged** (success) or **closed without merge** (note the reason and report).

## Steps

1. **Snapshot the PR state** for `#{pr}` on `{repo}` via Composio. Issue these in parallel where the model can — each is a read-only state op:
   ```
   composio_execute({
     "tool": "GITHUB_GET_A_PULL_REQUEST",
     "arguments": { "owner": "<upstream-owner>", "repo": "<upstream-repo-name>", "pull_number": {pr} }
   })
   composio_execute({
     "tool": "GITHUB_LIST_REVIEW_COMMENTS_ON_A_PULL_REQUEST",
     "arguments": { "owner": "<upstream-owner>", "repo": "<upstream-repo-name>", "pull_number": {pr} }
   })
   composio_execute({
     "tool": "GITHUB_LIST_ISSUE_COMMENTS",
     "arguments": { "owner": "<upstream-owner>", "repo": "<upstream-repo-name>", "issue_number": {pr} }
   })
   composio_execute({
     "tool": "GITHUB_LIST_REVIEWS_FOR_A_PULL_REQUEST",
     "arguments": { "owner": "<upstream-owner>", "repo": "<upstream-repo-name>", "pull_number": {pr} }
   })
   composio_execute({
     "tool": "GITHUB_GET_THE_COMBINED_STATUS_FOR_A_SPECIFIC_REFERENCE",
     "arguments": { "owner": "<upstream-owner>", "repo": "<upstream-repo-name>", "ref": "<head-sha-from-pull-request>" }
   })
   composio_execute({
     "tool": "GITHUB_LIST_CHECK_RUNS_FOR_A_GIT_REFERENCE",
     "arguments": { "owner": "<upstream-owner>", "repo": "<upstream-repo-name>", "ref": "<head-sha-from-pull-request>" }
   })
   ```
   PRs in the GitHub API are addressable both as `pull_number` (the PR-specific endpoints) and `issue_number` (top-level comments live on the issue surface). The check-rollup endpoints take the head SHA from the PR response.

   Derive `<fork-owner>` from the PR's `head.repo.owner.login` (or use `{fork}` if provided). Note the head branch name as `<branch>`. Record: failing-check ids, unresolved comment threads (with their body + author + path/line if inline), approval count, merge state, PR state (`OPEN` / `MERGED` / `CLOSED`).

   _TODO(composio-catalog): if the exact slugs above drift in the Composio catalog, swap to whatever the current name is. The shapes (owner/repo/pull_number/issue_number/ref) are stable; the slug casing is what occasionally changes._

2. **Check terminal conditions first.**
   - PR `state` is `MERGED` → report `"merged: <url>"` and stop.
   - PR `state` is `CLOSED` (not merged) → report `"closed: <one-line reason from the latest comment>"` and stop.
   - All required checks `success` AND zero unresolved actionable threads AND at least one approval → report `"ready for merge: <url>"` and stop.
   - Otherwise → continue.

3. **Clone the fork branch fresh** to a unique local directory (skip this if the directory from a prior round in this same run already exists and is on the right HEAD). Clone + identity-pin are local-git working-tree ops:
   ```
   git clone --branch <branch> https://github.com/<fork-owner>/<repo-name> /tmp/<repo-name>-pr{pr}-<rand>
   ```
   Pin the local git identity in the clone so any new commits are verified under the authed account. Use the `login` and `id` from a one-time `GITHUB_GET_THE_AUTHENTICATED_USER` call:
   ```
   composio_execute({ "tool": "GITHUB_GET_THE_AUTHENTICATED_USER", "arguments": {} })
   # then with <login> + <id> from the response:
   git -C <dir> config user.name  "<login>"
   git -C <dir> config user.email "<id>+<login>@users.noreply.github.com"
   ```

4. **Address each signal in turn.** Process every open item before pushing — group changes into one push per round:

   - **CI check failed** — read the failure detail from the check-runs response in step 1 (look at `output.summary` / `output.text` on the failed run). Locate the cause (start with `codegraph_search` on the failing test name or error string), apply the minimal fix, run the targeted test locally to confirm green (`cargo test -p <crate> <name>` / `pnpm test <pattern>` etc.), commit with a message that names the failing check:
     ```
     git -C <dir> add <only-the-fixed-files>
     git -C <dir> commit -m "fix(<scope>): <one line> (CI: <check-name>)"
     ```
     Do **not** bypass with `--no-verify` unless the failure is verifiably unrelated to this PR.

   - **Reviewer asks for a code change (actionable, human or bot)** — make the edit, commit referencing the comment: `git commit -m "address review: <one-line> (#{pr} review)"`. The reply on the thread happens after the push in step 6.

   - **Bot comment (CodeRabbit / Codecov / etc.)** — treat as actionable by default. If clearly a false positive, plan a thread reply (in step 6) with a one-line reason instead of a spurious code change.

   - **Reviewer requests deferral / accepts a known limitation** — plan a thread reply acknowledging, file a follow-up issue if appropriate, and persist it as "deferred" in the round summary.

5. **Push the round's fixes** to the fork in one push. Pushing the branch is a working-tree op, so it stays on local git:
   ```
   git -C <dir> push --force-with-lease "https://github.com/<fork-owner>/<repo-name>" <branch>
   ```
   Use `--force-with-lease` (never plain `--force`) so a concurrent push from someone else aborts the push instead of clobbering. If `--force-with-lease` refuses because the remote moved, re-run step 1 (the remote diverged — handle the new commits before pushing).

6. **Reply to every addressed comment** by id so reviewers know it's been handled — even when the fix is obvious from the diff. All replies are Composio calls:

   - **Inline review comment** (file:line, has `id` from step 1):
     ```
     composio_execute({
       "tool": "GITHUB_CREATE_A_REPLY_FOR_A_REVIEW_COMMENT",
       "arguments": {
         "owner": "<upstream-owner>",
         "repo":  "<upstream-repo-name>",
         "pull_number": {pr},
         "comment_id":  <comment-id>,
         "body": "Fixed in <short-sha>. <one-line description>"
       }
     })
     ```
   - **Top-level review or general thread**:
     ```
     composio_execute({
       "tool": "GITHUB_CREATE_AN_ISSUE_COMMENT",
       "arguments": {
         "owner": "<upstream-owner>",
         "repo":  "<upstream-repo-name>",
         "issue_number": {pr},
         "body": "<reply>"
       }
     })
     ```
   - **Deferred / disagreed**: reply with the one-line reason instead of a code change, using the same `GITHUB_CREATE_A_REPLY_FOR_A_REVIEW_COMMENT` (inline) or `GITHUB_CREATE_AN_ISSUE_COMMENT` (top-level) call.

7. **Wait for CI to re-run on the new commits** before declaring the round done. Polling is a Composio loop — re-issue the check-runs read every ~30s on the new head SHA until every required check has reached a terminal state:
   ```
   composio_execute({
     "tool": "GITHUB_LIST_CHECK_RUNS_FOR_A_GIT_REFERENCE",
     "arguments": { "owner": "<upstream-owner>", "repo": "<upstream-repo-name>", "ref": "<new-head-sha>" }
   })
   ```
   Stop polling when every required check's `status` is `completed` (look at `conclusion` to decide success/failure). Cap the poll at ~30 minutes per round so a stuck CI doesn't pin this run indefinitely.

8. **Re-loop to step 1.** If `{max_rounds}` rounds (default 5) have run without both gates closing, exit with `"blocked after N rounds — surfacing for human review"` plus the still-failing checks and still-open comment ids.

## Rules
- **GitHub state via Composio, working tree via local git.** Never shell to `gh` — the preflight gates Composio, not `gh`'s credential store, so a `gh` call can silently use the wrong identity.
- **Scope:** only fixes for *this PR's* review feedback or CI failures. No unrelated refactors, no scope creep, no other issues.
- **`--force-with-lease`, never `--force`.** Preserve anyone else's pushes.
- **Don't bypass CI** with `--no-verify` unless the failure is verifiably unrelated to this PR AND that's been justified in the round summary.
- **Reply to every actionable signal** — addressed-and-pushed comments still need a thread reply so the reviewer knows.
- **CI green ≠ done.** Comments still matter; both gates must close.
- **Approvals don't auto-merge.** Note the approval and keep monitoring until the PR is actually merged or closed.
- **Don't push to upstream.** Pushes go to the fork only.
- **Stop** when both gates close, the PR is merged/closed, the round cap is hit, or you've identified a blocker that needs a human — report status plainly either way.
