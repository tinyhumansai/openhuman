# scripts/work

Automate picking up a GitHub issue: sync `main`, cut a working branch, and
hand the issue off to an LLM CLI to start implementing.

Mirrors the structure of [`scripts/review`](../review) and reuses its
`lib.sh` helpers.

## Usage

```sh
pnpm work 1234                            # default agent: claude
pnpm work 1234 "focus on the retry path"  # extra prompt appended verbatim
pnpm work 1234 --agent codex              # any CLI that takes -p "<prompt>"
pnpm work 1234 --no-checkout              # skip git sync; use current branch
```

The first numeric arg is treated as the issue number, so `pnpm work 1234 …`
and `pnpm work start 1234 …` are equivalent.

## What it does

1. Resolves the target repo from `WORK_REPO`, then falls back to the
   `upstream` remote (or `origin`).
2. Fetches the issue (title, body, labels, URL) with `gh`.
3. Checks out `main`, fast-forwards from `upstream`/`origin`, then creates a
   branch `<prefix>/<issue>-<slug>` (slug derived from the issue title,
   max 40 chars). If the branch already exists it's checked out and `main`
   is merged in.
4. Hands off to the agent CLI with a prompt containing the issue body,
   repo conventions pointers (CLAUDE.md / AGENTS.md), and any trailing
   `extra-prompt`.

## Config

- `WORK_REPO=owner/name` — override the target repo.
- `WORK_BRANCH_PREFIX=issue` — branch is `<prefix>/<num>-<slug>`.
- Requires `git`, `gh`, `jq`, plus the agent CLI (default `claude`).
