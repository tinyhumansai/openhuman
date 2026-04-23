# scripts/review

Helpers for working through PRs on this repo. Runnable directly — no zshrc
integration needed.

| Script       | What it does                                                                      |
| ------------ | --------------------------------------------------------------------------------- |
| `sync.sh`    | Fetch PR head, check out as `pr/<num>`, merge `main`, wire push/upstream.         |
| `review.sh`  | `sync` + hand off to the `pr-reviewer` agent to review, comment, and approve.     |
| `fix.sh`     | `sync` + `pr-reviewer` (apply fixes) + `pr-manager-lite` (commit & push).         |
| `merge.sh`   | LLM-summarized squash body + filtered Co-authored-by trailers + `gh pr merge`.    |

## LLM flags

- `review` / `fix`: `--executor-llm <tool>` (default `claude`). Picks the CLI that
  drives the agent prompt.
- `merge`: `--summary-llm <tool>` (default `gemini`). The LLM that condenses the PR
  body + commit messages into a concise squash commit body. Use `--summary-llm none`
  to skip summarization and keep the raw PR body.

Any tool that accepts `-p "<prompt>"` and prints its response to stdout works.

## Usage

Via yarn (preferred):

```sh
yarn review sync 123
yarn review review 123
yarn review fix 123
yarn review merge 123              # --squash
yarn review merge 123 --rebase
yarn review --help
```

Or invoke the scripts directly:

```sh
scripts/review/sync.sh 123
scripts/review/review.sh 123
scripts/review/fix.sh 123
scripts/review/merge.sh 123
```

## Config

- Repo is derived from the `upstream` remote (falls back to `origin`). Override
  with `REVIEW_REPO=owner/name`.
- `REVIEW_BANNED_COAUTHOR_RE` overrides the substring regex used to drop
  `Co-authored-by:` entries (default filters copilot / codex / cursor / claude /
  anthropic / openai / chatgpt / `[bot]` / `noreply@github` /
  `users.noreply.github.com`; matched case-insensitively on name or email).
- Requires `git`, `gh`, `jq`. `review` / `fix` also require the executor LLM CLI
  (default `claude`); `merge` also requires the summary LLM CLI (default `gemini`)
  unless `--summary-llm none`.
