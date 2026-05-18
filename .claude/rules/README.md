# `.claude/rules/`

This directory is intentionally near-empty.

Authoritative docs for AI agents and contributors:

- **[`CLAUDE.md`](../../CLAUDE.md)** — repo layout, runtime scope, commands, frontend/Tauri/Rust conventions, testing, debug logging, feature workflow.
- **[`AGENTS.md`](../../AGENTS.md)** — RPC controller patterns, `RpcOutcome<T>` contract.
- **[`.claude/memory.md`](../memory.md)** — project memory: fixes, gotchas, strict rules, workflow notes.
- **[`gitbooks/developing/architecture.md`](../../gitbooks/developing/architecture.md)** — narrative architecture, dual-socket sync.
- **[`gitbooks/developing/architecture/frontend.md`](../../gitbooks/developing/architecture/frontend.md)** — frontend layout.
- **[`gitbooks/developing/architecture/tauri-shell.md`](../../gitbooks/developing/architecture/tauri-shell.md)** — Tauri shell.
- **[`gitbooks/developing/architecture/agent-harness.md`](../../gitbooks/developing/architecture/agent-harness.md)** — agent harness / tool surface.
- **[`gitbooks/developing/e2e-testing.md`](../../gitbooks/developing/e2e-testing.md)** — WDIO/Appium testing.
- **[`gitbooks/developing/cef.md`](../../gitbooks/developing/cef.md)** — CEF runtime notes.
- **[`gitbooks/developing/testing-strategy.md`](../../gitbooks/developing/testing-strategy.md)** — testing strategy.
- **[`gitbooks/developing/agent-observability.md`](../../gitbooks/developing/agent-observability.md)** — agent observability.

## When to add a file here

Only add a `*.md` file in this directory if you need **path-gated context** loaded conditionally by Claude Code (via the `paths:` frontmatter) for a narrow part of the tree, AND the content is not already covered in `CLAUDE.md`.

Each file added here ships in every agent context that matches its `paths:` glob — so keep them small, current, and non-overlapping with `CLAUDE.md`. Stale rules actively mislead agents.
