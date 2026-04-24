# `.claude/rules/`

This directory is intentionally near-empty.

Authoritative docs for AI agents and contributors:

- **[`CLAUDE.md`](../../CLAUDE.md)** — repo layout, runtime scope, commands, frontend/Tauri/Rust conventions, testing, debug logging, feature workflow.
- **[`AGENTS.md`](../../AGENTS.md)** — RPC controller patterns, `RpcOutcome<T>` contract.
- **[`docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md)** — narrative architecture, dual-socket sync.
- **[`docs/DESIGN_GUIDELINES.md`](../../docs/DESIGN_GUIDELINES.md)** — visual language.
- **[`docs/E2E-TESTING.md`](../../docs/E2E-TESTING.md)** — WDIO/Appium testing.
- **[`docs/src/README.md`](../../docs/src/README.md)** — frontend.
- **[`docs/src-tauri/README.md`](../../docs/src-tauri/README.md)** — Tauri shell.

## When to add a file here

Only add a `*.md` file in this directory if you need **path-gated context** loaded conditionally by Claude Code (via the `paths:` frontmatter) for a narrow part of the tree, AND the content is not already covered in `CLAUDE.md`.

Each file added here ships in every agent context that matches its `paths:` glob — so keep them small, current, and non-overlapping with `CLAUDE.md`. Stale rules actively mislead agents.
