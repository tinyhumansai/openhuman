---
name: Feature
description: Propose a new capability with tests, code documentation, and modular design
title: "[Feature] "
labels:
  - enhancement
---

## Summary

What we’re building and the user-visible outcome.

## Problem / motivation

What’s missing today, who it hurts, and constraints (platform, privacy, performance).

## Proposed scope

Rough boundaries: Rust core, React/Tauri app, or both. Keep the first slice small.

## Module design (Unix-style)

How work splits into **small modules with one sharp responsibility**:

- **New or extended modules:** …
- **Public API / boundaries:** …
- **Explicitly out of scope:** …

New Rust work belongs under `src/openhuman/<domain>/` per repo conventions—not new loose files at `src/openhuman/` root.

## Testing

Check what applies before closing the feature:

- [ ] **Unit tests** — Vitest (`app/`) and/or `cargo test` (core) for logic you add or change
- [ ] **E2E / integration** — Where behavior is user-visible or crosses UI → Tauri → sidecar → JSON-RPC; use existing harnesses (`app/test/e2e`, mock backend, `tests/json_rpc_e2e.rs` as appropriate)
- [ ] **N/A** — If truly not applicable, say why (e.g. documentation-only)

**Commands / notes (optional):**

## Code documentation

- [ ] **Doc comments** — `///` / `//!` (Rust), JSDoc or brief file/module headers (TS) on public APIs and non-obvious modules
- [ ] **Inline comments** — Where logic, invariants, or edge cases aren’t clear from names alone (keep them grep-friendly; avoid restating the code)

**Notes (optional):**

## Acceptance criteria

Bullet list of “done when …” items reviewers can verify.

## Related

Links to issues, PRs, or prior discussion.
