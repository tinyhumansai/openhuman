# `.claude/rules/`

This directory is intentionally near-empty.

Authoritative docs for AI agents and contributors:

- **[`AGENTS.md`](../../AGENTS.md)** — all project conventions, commands, architecture, testing, patterns. (`CLAUDE.md` redirects here.)
- **[`docs/gitbooks/en/developing/architecture.md`](../../docs/gitbooks/en/developing/architecture.md)** — narrative architecture, dual-socket sync.
- **[`docs/gitbooks/en/developing/architecture/frontend.md`](../../docs/gitbooks/en/developing/architecture/frontend.md)** — frontend layout.
- **[`docs/gitbooks/en/developing/architecture/tauri-shell.md`](../../docs/gitbooks/en/developing/architecture/tauri-shell.md)** — Tauri shell.
- **[`docs/gitbooks/en/developing/architecture/agent-harness.md`](../../docs/gitbooks/en/developing/architecture/agent-harness.md)** — agent harness / tool surface.
- **[`docs/gitbooks/en/developing/e2e-testing.md`](../../docs/gitbooks/en/developing/e2e-testing.md)** — WDIO/Appium testing.
- **[`docs/gitbooks/en/developing/cef.md`](../../docs/gitbooks/en/developing/cef.md)** — historical CEF runtime notes (the shell now uses Wry).
- **[`docs/gitbooks/en/developing/testing-strategy.md`](../../docs/gitbooks/en/developing/testing-strategy.md)** — testing strategy.
- **[`docs/gitbooks/en/developing/agent-observability.md`](../../docs/gitbooks/en/developing/agent-observability.md)** — agent observability.

## When to add a file here

Only add a `*.md` file in this directory if you need **path-gated context** loaded conditionally by Claude Code (via the `paths:` frontmatter) for a narrow part of the tree, AND the content is not already covered in `AGENTS.md`.

Each file added here ships in every agent context that matches its `paths:` glob — so keep them small, current, and non-overlapping with `AGENTS.md`. Stale rules actively mislead agents.
