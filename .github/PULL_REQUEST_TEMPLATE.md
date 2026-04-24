## Summary

- What changed and why.
- Keep this to 3-6 bullets focused on user-visible or architecture-impacting changes.

## Problem

- What issue or risk this PR addresses.
- Include context needed for reviewers to evaluate correctness quickly.

## Solution

- How the implementation solves the problem.
- Note important design decisions and tradeoffs.

## Submission Checklist

- [ ] **Unit tests** — Vitest (`app/`) and/or `cargo test` (core) for logic you add or change
- [ ] **E2E / integration** — Where behavior is user-visible or crosses UI → Tauri → sidecar → JSON-RPC; use existing harnesses (`app/test/e2e`, mock backend, `tests/json_rpc_e2e.rs` as appropriate)
- [ ] **N/A** — If truly not applicable, say why (e.g. change is documentation-only)
- [ ] **Doc comments** — `///` / `//!` (Rust), JSDoc or brief file/module headers (TS) on public APIs and non-obvious modules
- [ ] **Inline comments** — Where logic, invariants, or edge cases aren’t clear from names alone (keep them grep-friendly; avoid restating the code)

(Any feature related checklist can go in here)

## Impact

- Runtime/platform impact (desktop/mobile/web/CLI), if any.
- Performance, security, migration, or compatibility implications.

## Related

<!--
Use a closing keyword so GitHub auto-closes the issue on merge. One per line.
Supported (case-insensitive): close/closes/closed, fix/fixes/fixed, resolve/resolves/resolved.
A bare "#123" reference is just a link — it does NOT close the issue.

  Closes #123
  Fixes  #456
-->

- Closes:
- Follow-up PR(s)/TODOs:
