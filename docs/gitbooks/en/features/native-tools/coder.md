---
description: >-
  A complete toolset for working on real codebases: read, write, edit, search,
  git, lint and test.
icon: code
---

# Coder

The coder tools make OpenHuman a real coding partner and not a chat window that only pretends to know your codebase.

## Tools in this family

| Tool             | What it does                                                          |
| ---------------- | --------------------------------------------------------------------- |
| `file_read`      | Reads a file, with line numbers like `cat -n`.                        |
| `file_write`     | Writes a new file.                                                    |
| `edit`           | Makes targeted edits by match and replace, with strict uniqueness checks. |
| `apply_patch`    | Applies a unified diff.                                               |
| `glob`           | Finds files by glob pattern.                                          |
| `grep`           | Searches the tree, ripgrep style.                                     |
| `list`           | Walks a directory tree.                                               |
| `read_diff`      | Shows the diff between two files or revisions.                        |
| `git_operations` | Status, diff, log, blame, branch and commit.                          |
| `run_linter`     | Runs the project's linter.                                            |
| `run_tests`      | Runs the project's test command.                                      |
| `csv_export`     | Exports query results as CSV.                                         |

## Why these are native

A shell tool plus `cat`, `sed` and `awk` could technically do all of this. The native tools exist because:

- Edits go through a uniqueness check, so the agent can't clobber the wrong line.
- Reads come back with line numbers the agent can refer to in follow-ups.
- Git output is parsed into structured data, so the agent doesn't scrape porcelain output.
- Lint and test runs use the project's real commands, not generic guesses.

## Workspace scoping

File tools act inside the agent's working folder (`action_dir`). The workspace directory holds internal state and is never a tool target.

With the autonomy policy on, `workspace_only` confines the tools to the working folder, and anything outside it needs an explicit trusted root. With the policy off (the default), that confinement is not enforced. One floor holds either way: credential stores (`~/.ssh`, `~/.gnupg`, `~/.aws`) and system roots are unreachable, as are `..` traversal and null bytes in a path. See [Approval gate](../approval-gate.md).

## See also

- [System and utilities](system-and-utilities.md): `shell` for the rest of the dev loop, including Node.js, npm and Python commands available on the host's `PATH`.
- [Agent coordination](agent-coordination.md): `todo` and `spawn_subagent` for larger refactors.
