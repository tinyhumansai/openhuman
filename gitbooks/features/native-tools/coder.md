---
description: A complete toolset for working on real codebases - read, write, edit, search, git, lint, test.
icon: code
---

# Coder

The coder family is what makes OpenHuman a viable coding partner instead of a chat window that *pretends* to know the codebase.

## Tools in the family

| Tool             | What it does                                                      |
| ---------------- | ----------------------------------------------------------------- |
| `file_read`      | Read a file (with line numbers, like `cat -n`).                   |
| `file_write`     | Write a new file.                                                 |
| `edit_file`      | Targeted edits - match-and-replace with strict uniqueness checks. |
| `apply_patch`    | Apply a unified diff.                                             |
| `glob_search`    | Find files by glob pattern.                                       |
| `grep`           | Ripgrep-style search across the tree.                             |
| `list_files`     | Walk a directory tree.                                            |
| `read_diff`      | Diff between two files or revisions.                              |
| `git_operations` | Status, diff, log, blame, branch, commit.                         |
| `run_linter`     | Run the project's linter.                                         |
| `run_tests`      | Run the project's test command.                                   |
| `csv_export`     | Export query results as CSV.                                      |

## Why these are native, not shell-only

A shell tool plus `cat`/`sed`/`awk` could *technically* do all of this. The native tools exist because:

* Edits go through a uniqueness check, so the agent can't accidentally clobber the wrong line.
* Reads come back with line numbers the agent can refer to in follow-ups.
* Git operations parse output into structured data, instead of leaving the agent to scrape porcelain.
* Lint and test runs are wired to the project's actual commands, not generic guesses.

## Workspace scoping

Filesystem tools respect a workspace boundary - the agent can't read or write outside it without explicit permission. Same boundary the rest of the app uses for `OPENHUMAN_WORKSPACE`.

## See also

* [System & Utilities](system-and-utilities.md) - `shell`, `node_exec`, `npm_exec`, `python_exec` for the rest of the dev loop.
* [Agent Coordination](agent-coordination.md) - `todo_write`, `spawn_subagent` for larger refactors.
