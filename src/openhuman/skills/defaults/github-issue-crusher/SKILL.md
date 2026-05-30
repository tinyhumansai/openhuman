# GitHub Issue Crusher

Fix the **single** GitHub issue named in the inputs, end to end, then open a **DRAFT** pull request via the **fork workflow** — issue on upstream `{repo}`, fix pushed to a fork, cross-repo draft PR back to `{repo}`. Stay strictly in scope; this is autonomous, so work until the draft PR is open or you hit a real blocker, then stop.

## Tool split — Composio for GitHub state, local git for the working tree

GitHub state operations — issues, PRs, comments, reviews, checks, labels, branches as remote refs, repository metadata — go through Composio via `composio_execute({tool: "GITHUB_<ACTION>", arguments: {...}})`. The working tree — clone, checkout, edit, `git status`/`diff`, run tests, commit locally, push the branch to your fork — stays on local `git`. Composio is the single authoritative GitHub identity (the user's connected account, gated by the skill's `[github]` preflight); the local working tree is where the actual code change happens. Do **not** shell out to `gh` for state operations — the preflight checked Composio but not gh's credential store, so a `gh` call may silently route through a different account.

## The two repos

- **Upstream** = `{repo}` — where `#{issue}` lives and where the draft PR is opened (base = `{pr_base}`, or the upstream's default branch).
- **Fork** = `{fork}` if provided, otherwise the existing fork of `{repo}` under the authed GitHub account. Resolve the authed account once at the top:
  ```
  composio_execute({
    "tool": "GITHUB_GET_THE_AUTHENTICATED_USER",
    "arguments": {}
  })
  ```
  The response's `login` is `<fork-owner>`. If no fork exists yet, create one:
  ```
  composio_execute({
    "tool": "GITHUB_CREATE_A_FORK",
    "arguments": { "owner": "<upstream-owner>", "repo": "<upstream-repo-name>" }
  })
  ```

## Steps

1. **Read the issue.** Fetch issue `#{issue}` in `{repo}` (title, body, comments) via Composio:
   ```
   composio_execute({
     "tool": "GITHUB_GET_AN_ISSUE",
     "arguments": { "owner": "<upstream-owner>", "repo": "<upstream-repo-name>", "issue_number": {issue} }
   })
   composio_execute({
     "tool": "GITHUB_LIST_ISSUE_COMMENTS",
     "arguments": { "owner": "<upstream-owner>", "repo": "<upstream-repo-name>", "issue_number": {issue} }
   })
   ```
   Identify the exact files/changes it asks for.

2. **Ensure the fork.** Resolve `<fork-owner>` via `GITHUB_GET_THE_AUTHENTICATED_USER` (cache for the run). Create the fork via `GITHUB_CREATE_A_FORK` if it doesn't already exist (idempotent — a no-op when the fork is already there).

3. **Clone fresh.** Clone `{repo}` locally to a unique directory (e.g. `/tmp/<repo-name>-{issue}-<rand>`). If the directory already exists from a previous run, remove it first so the clone starts clean. This is a local-git operation:
   ```
   git clone https://github.com/{repo} /tmp/<repo-name>-{issue}-<rand>
   ```

4. **Pin the local git identity** in the clone so commits are verified under the authed account. Use the `login` and `id` already on hand from step 2 — never `--global`, never clobber the host's global config:
   ```
   git -C <dir> config user.name  "<login>"
   git -C <dir> config user.email "<id>+<login>@users.noreply.github.com"
   ```

5. **Locate the cause.** Start with `codegraph_search` on the issue's key symbols / error strings / literal phrases — it auto-indexes on first call (~30–90s on a fresh clone, this is normal not a hang). Inspect the result:
   - `coverage: full` → read the top hits and confirm the exact edit site.
   - `coverage: partial` → refine with `grep` scoped to the directories codegraph returned.
   - `coverage: none` or zero hits → fall back to a blind `grep` / `glob`.

6. **Apply the minimal fix.** Edit only the files identified in step 5. Re-read each file or `git diff` to confirm the change matches the intent — never trust memory.

7. **Verify.** Run the test/lint commands that apply to the changed files (e.g. `pnpm i18n:check` for i18n, `cargo test -p <crate>` for Rust, `pnpm test <pattern>` for TS). Skip if the change is pure docs / strings.

8. **Branch, commit, push to the fork** via local git — pushing is a working-tree operation so it stays on git:
   ```
   git -C <dir> checkout -b fix/{issue}-<short-slug>
   git -C <dir> add <only-the-changed-files>          # never git add -A
   git -C <dir> commit -m "<type>(scope): <short description> (#{issue})"
   git -C <dir> push -u "https://github.com/<fork-owner>/<repo-name>" fix/{issue}-<short-slug>
   ```

9. **Open the DRAFT cross-repo PR via Composio.** This is the canonical Composio call for cross-repo PRs — the `head` value `<fork-owner>:<branch>` tells GitHub to take the branch from the fork:
   ```
   composio_execute({
     "tool": "GITHUB_CREATE_A_PULL_REQUEST",
     "arguments": {
       "owner": "<upstream-owner>",
       "repo":  "<upstream-repo-name>",
       "title": "<type>(scope): <short description> (#{issue})",
       "body":  "Closes #{issue}\n\n## Root cause\n<one paragraph>\n\n## Fix\n<one paragraph>\n\n## Verified\n<what you ran>",
       "head":  "<fork-owner>:fix/{issue}-<short-slug>",
       "base":  "{pr_base}",
       "draft": true
     }
   })
   ```
   `draft: true` is non-negotiable for autonomous runs — CI runs and a human reviews before promotion to ready.

10. **Hand off Phase 6 to the shepherd, then exit.** Once the draft PR URL is in hand, invoke the `pr-review-shepherd` skill as a fresh background run so the CI + review loop continues autonomously while *this* skill exits cleanly:
    ```
    run_skill {
      "skill_id": "pr-review-shepherd",
      "inputs": { "repo": "{repo}", "pr": <pr-number-just-opened> }
    }
    ```
    The call returns immediately with the shepherd's `run_id` + `log` path. Include both in your final response so the user can track the shepherd, then stop — do not stay around polling CI yourself, that's the shepherd's job.

## Rules
- **GitHub state via Composio, working tree via local git.** Never shell to `gh` — the preflight gates Composio, not `gh`'s credential store, so a `gh` call can silently use the wrong identity.
- **Scope:** only changes that fix `#{issue}`. No unrelated cleanup, no other issues.
- **Source of truth** is the filesystem + `git` + `codegraph` — re-read / re-search rather than relying on recall.
- **codegraph_search first** for every locate step (it auto-indexes); `grep` / `glob` are refinement or fallback only.
- **DRAFT always** — never open a PR as ready-to-merge from an autonomous run.
- **Stop** when the draft PR is open or surface a real blocker and stop — don't thrash.
