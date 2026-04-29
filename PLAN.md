# Backend Overhaul — Controller Registry Consolidation

Branch: `feat/backend-overhaul`
Base: `main` @ 515c4f4b

## Why

Project rule (CLAUDE.md): domain functionality exposed to CLI + JSON-RPC **only** via the controller registry (`schemas.rs` per domain + `src/core/all.rs`). Current state is 90% there — 34 domains fully migrated — but six CLI adapters and one JSON-RPC fallback still live in the transport layer. Finishing this lets us delete ~95KB of adapter code and enforce the rule with a lint/grep check.

## Goals

1. Zero domain-specific code in `src/core/cli.rs`, `src/core/jsonrpc.rs`, `src/rpc/dispatch.rs`.
2. Every CLI command and JSON-RPC method dispatched through the shared registry.
3. No behavior change — all existing CLI flags and RPC methods preserve names, params, outputs.
4. Unregistered infrastructure modules documented as intentionally transport-agnostic.

## Non-goals

- No new domain functionality.
- No changes to skills QuickJS runtime exposure (stays runtime-only).
- No renames of existing CLI flags or RPC method names.
- No touching memory-tree domain (sanil-23 has in-flight PRs #732 #733).

## Current leakage

| File | LOC | What's there | Target |
|---|---|---|---|
| `src/core/agent_cli.rs` | 21.4K | agent CLI adapter | `openhuman/agent/schemas.rs` |
| `src/core/memory_cli.rs` | 15.3K | memory CLI adapter | `openhuman/memory/schemas.rs` |
| `src/core/text_input_cli.rs` | 12.4K | text_input CLI adapter | `openhuman/text_input/schemas.rs` |
| `src/core/screen_intelligence_cli.rs` | 26.5K | screen_intelligence CLI adapter | `openhuman/screen_intelligence/schemas.rs` |
| `src/core/tree_summarizer_cli.rs` | 13.2K | tree_summarizer CLI adapter | `openhuman/tree_summarizer/schemas.rs` |
| `src/core/cli.rs` (voice block) | ~2K | `run_voice_server_command()` | `openhuman/voice/schemas.rs` |
| `src/rpc/dispatch.rs` | 76 | `openhuman.security_policy_info` fallback | `openhuman/security/schemas.rs` (or about_app) |

Total: ~95KB of adapter code to fold into registry.

## Exposure rule (decision tree)

Two legitimate shapes for exposing a domain capability. Pick by command shape, not by convenience:

1. **Controller registry** (`src/openhuman/<domain>/schemas.rs` + `src/core/all.rs`).
   Use for **request/response RPC-style capabilities** — take params, do work, return JSON, done. The vast majority of domain methods. CLI access is free via the generic namespace dispatcher (`openhuman <domain> <verb> --args`).

2. **Domain-owned CLI adapter** (`src/openhuman/<domain>/cli.rs`).
   Use only for **standalone long-running / blocking operational commands** — e.g. `openhuman voice` runs a hotkey listener forever, holds the process, logs to stderr. These don't fit the registry's fire-and-return JSON contract. The adapter lives *inside* the domain (not in `src/core/cli.rs`) so transport layer stays generic.

Hard rule: no domain-specific imports or logic in `src/core/cli.rs`, `src/core/jsonrpc.rs`, or `src/rpc/dispatch.rs`. The only allowed reference is the dispatch line that routes `args[0]` to the domain's CLI function. Everything else lives in the domain.

Don't invent a third shape. If a capability feels like it needs one, it probably belongs in one of the two above with a thin adjustment.

## Approach

Each migration is a self-contained diff:

1. Confirm domain already has `schemas.rs` with `all_<domain>_registered_controllers()`.
2. For each CLI subcommand in `<domain>_cli.rs`:
   - Identify the underlying domain function it calls.
   - Add a `handle_<verb>` fn in `schemas.rs` that takes `Map<String, Value>` and returns `ControllerFuture`.
   - Register it in `all_<domain>_registered_controllers()`.
3. Delete the `<domain>_cli.rs` file.
4. Remove its import from `src/core/cli.rs` / `mod.rs`.
5. Verify CLI flags still work via generic dispatcher — the registry already exposes them by method name.
6. `cargo check` + `cargo test -p openhuman` per step.

The generic CLI adapter in `cli.rs` already knows how to route `<domain> <verb> --key value` to registry method `openhuman.<domain>_<verb>`. Migration is mostly deletion once handlers exist.

## Phases

### Phase 1 — Smallest adapter first (shakedown)

**voice** (~2K inline in `cli.rs`). Lowest risk: one server command, confirms the approach end-to-end before touching bigger files.

- Commit: `refactor(voice): move CLI adapter to controller registry`

### Phase 2 — Mechanical migrations (one PR each)

Order = smallest file first, so each PR is reviewable and CI catches issues early.

1. `text_input` (12.4K) — likely thin wrappers.
2. `tree_summarizer` (13.2K).
3. `memory_cli` (15.3K) — **coordinate with sanil-23 memory-tree PRs**; rebase after #732/#733 merge or pair-review.
4. `agent_cli` (21.4K).
5. `screen_intelligence` (26.5K) — largest, leave for last.

Each: one commit, one PR, self-contained. No bundling.

### Phase 3 — JSON-RPC dispatch fallback

Move `openhuman.security_policy_info` from `src/rpc/dispatch.rs` into whichever domain owns security policy (likely `about_app` or a new `security` module). Delete `rpc/dispatch.rs` if it becomes empty, else leave as a thin generic dispatch shim.

### Phase 4 — Enforce the rule

Add a build-time check or CI grep. The rule must forbid domain **imports/types** in the transport layer while still allowing the dispatcher to call the generic `::cli::run_*` entrypoints that live inside each domain:

```bash
# Forbid `use crate::openhuman::<domain>::…` imports in transport files.
! grep -rE '^\s*use\s+crate::openhuman::[a-z_]+::' \
    src/core/cli.rs src/core/jsonrpc.rs src/rpc/dispatch.rs

# Forbid any other domain reference that is NOT a call into ::cli::run_*(…).
# (The dispatcher is allowed to invoke `crate::openhuman::<domain>::cli::run_<x>(...)`.)
! grep -rE 'crate::openhuman::[a-z_]+::' \
    src/core/cli.rs src/core/jsonrpc.rs src/rpc/dispatch.rs \
  | grep -vE 'crate::openhuman::[a-z_]+::cli::run_[a-z_]+\('
```

Fails if any domain-specific import or non-dispatcher reference creeps back in. Commit as `chore(core): fence domain imports out of transport layer`.

### Phase 5 — Document unregistered modules

Add a short table to `CLAUDE.md` listing intentionally-unregistered infra modules (accessibility, approval, context, embeddings, integrations, node_runtime, overlay, providers, routing, tokenjuice, tool_timeout) with one-liner "why not in registry" for each. Also document skills as runtime-only.

## Risks

- **agent_cli / screen_intelligence size** — large files may hide subtle flag-parsing logic not obvious from a first read. Mitigation: test-drive each CLI subcommand manually after migration; add CLI smoke tests to the registry dispatcher if missing.
- **memory_cli collision with sanil-23** — memory-tree PRs touch adjacent paths. Mitigation: do phase 2.3 last, rebase on top of merged PRs.
- **CLI flag drift** — generic adapter parses flags by schema. If current `_cli.rs` does custom parsing (e.g. positional args, subcommand nesting), need to extend schema before deleting. Mitigation: read each CLI file top-to-bottom before writing handler.
- **Behavior diff at registry boundary** — registry handlers return `ControllerFuture` with JSON; `_cli.rs` may print formatted text. Mitigation: keep a thin formatter in the generic CLI path, not per-domain.

## Verification per step

```bash
cargo check --manifest-path Cargo.toml
cargo test -p openhuman --test json_rpc_e2e
# Manual CLI smoke for migrated domain:
./target/debug/openhuman <domain> <verb> --args...
```

Plus `pnpm test:rust` for the full mock-backed integration suite before each PR.

## Milestones

- M1: Phase 1 done, shakedown merged.
- M2: Phases 2.1–2.5 done, all `*_cli.rs` deleted.
- M3: Phase 3 done, `rpc/dispatch.rs` cleaned.
- M4: Phase 4+5 done, rule enforced, docs updated.

Estimate: M1 = 0.5d, M2 = 3–4d (one domain/day), M3 = 0.5d, M4 = 0.5d. Total ~1 week of focused work.
