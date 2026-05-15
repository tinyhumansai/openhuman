# scripts/shortcuts

Workflow shortcuts — high-level pnpm commands that orchestrate routine
contributor tasks (picking up an issue, reviewing a PR, applying fixes,
merging, resetting the workspace). Each lives in its own subdirectory with
its own README and prompt templates.

| Shortcut       | pnpm command         | What it does                                                                 |
| -------------- | -------------------- | ---------------------------------------------------------------------------- |
| `review/`      | `pnpm review`        | Sync a PR locally and drive review / fix / coverage / merge via an LLM CLI.  |
| `work/`        | `pnpm work`          | Pick up a GitHub issue, cut a branch, hand off to an LLM CLI.                |
| `ws-reset.sh`  | `pnpm reset`         | Hard-reset local `main` to `upstream/main` and refresh submodules.           |

All shortcuts share `review/lib.sh` for repo resolution, PR sync, and the
colored `pass/fail/warn/info` helpers.

## Design

Each shortcut's agent prompt lives in a Markdown template
(`<shortcut>/prompts/*.md`) with placeholders (`__PR__`, `__REPO__`,
`__ISSUE__`, etc.). The shell wrapper handles repo state (git fetch /
checkout / merge), substitutes placeholders via `awk`, and hands the final
prompt to the chosen LLM CLI (`--agent`, default `claude`). This keeps the
workflow **agent-agnostic** — works with `codex`, `gemini`, `cursor-agent`,
or any CLI that accepts a single positional prompt argument.
