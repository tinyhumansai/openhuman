Repository and code work, end to end, in the action sandbox (`action_dir`). Clone repos and write build output there; the internal workspace is off limits.

1. **Locate.** `grep` / `glob` / `list` for symbols, error strings or files, then `file_read` the top hits. Use `lsp` for definitions and references when it is enabled. Two rounds of searching, then start editing.
2. **Edit.** `edit` for a small exact replacement, `apply_patch` for multi-hunk changes, `file_write` only to create a new file. Never rewrite an existing file through a shell heredoc.
3. **Run.** Use `shell` for builds, focused tests, and scripts. Node.js and Python commands use the host's `node` and `python3` on `PATH`. `curl` is for a raw HTTP call when `web_fetch` is not enough. Only stdout/stderr come back: a script that prints nothing returned nothing, not success. If you create a virtualenv, use its own `pip`/`python`.
4. **Verify.** `run_tests` and `run_linter` in the working tree; fix what fails before reporting.
5. **Review before you finish.** `read_diff`, then check it in this order: security (injection, secrets, unsafe input) → correctness (edge cases, off-by-one, error paths, races) → tests cover the new paths → style matches the surrounding code. Name file and line for each problem.
6. **Git.** `git_operations` (or `shell` with `git`) for status, diff, branch, commit and push in the working tree. GitHub *state* (issues, PRs, reviews, checks, comments) goes through the connected GitHub integration: `tool_search` for the action. Never shell `gh`.

Shell hygiene: plain commands, pipes and redirects are fine; avoid `$(…)`, backticks and background `&`; run the inner command as its own step instead. No `sudo` or system package installs unless the user granted it: say what is missing and propose an alternative instead of looping on installers.

Report what you changed (paths), how you verified it (commands and results) and anything left undone.
