# 00 — Baseline: crate, features, native links

Current status (2026-07-02): baseline dependency alignment is complete in both
Cargo worlds. `tinyagents 1.3.0` is resolved with the `sqlite` feature,
OpenHuman pins `rusqlite = "=0.40.0"`, both worlds patch through
`vendor/rusqlite-0.40.0` and `vendor/libsqlite3-sys-0.38.0`, and the SDK-gaps
inventory has been refreshed against the published 1.3.0 crate source.

## Steps

1. **Bump `tinyagents` to `"1.3"`** (done in both Cargo worlds — root and
   `app/src-tauri/`). Known 1.1→1.2 break already handled
   (`MessageDelta::text` ctor). Note: the `openai` crate feature was removed
   after 1.2.0 (1.2.1+ features are only `sqlite`/`repl`) — we never enabled
   it, so no impact. See "1.3.0 delta" below for new API this plan uses.
2. **Align rusqlite to 0.40** in both worlds (`Cargo.toml` root and
   `app/src-tauri/Cargo.toml`). OpenHuman pins `rusqlite = "=0.40.0"` and
   enables `tinyagents = { version = "1.3", features = ["sqlite"] }`.
   Compatibility notes:
   - `rusqlite 0.40` and `libsqlite3-sys 0.38` are consumed directly from
     crates.io. Their build scripts use the `cfg_select!` macro (stable from
     Rust 1.96 — `rust-toolchain.toml` pins `1.96.1`), so the earlier vendored
     copies under `vendor/` that backported `cfg_select!` to `#[cfg]` for the
     old 1.93 toolchain have been deleted.
   - The `channel-matrix` feature (and its `matrix-sdk` dependency) was dropped,
     which removes `matrix-sdk-sqlite` from the tree entirely. That crate pinned
     `rusqlite 0.37` and previously had to be vendored/patched onto the `0.40`
     line to avoid a second native sqlite chain; with Matrix gone the patch is
     deleted.
   - `whatsapp-rust/sqlite-storage` is disabled because its Diesel storage
     links sqlite independently; `whatsapp-web` temporarily uses
     `wacore::store::InMemoryBackend` and logs the non-durable session mode.
3. **Unlocks:** crate `SqliteCheckpointer` → later deletion of
   `src/openhuman/tinyagents/checkpoint.rs` (`SqlRunLedgerCheckpointer`,
   250 lines) once graphs are re-pointed and `graph_checkpoints` rows are
   migrated or expired (see `04-sessions/`).
4. **Do NOT enable `openai` feature** — OpenHuman providers stay the product
   source of truth for credentials/billing (spec non-goal).
5. Update the "TinyAgents crate: features & compatibility" section in
   `gitbooks/developing/architecture/agent-harness.md` and the Cargo.toml
   comment (lines 46–55) after the flip.
6. Mark `docs/tinyagents-sdk-gaps.md` items 1, 2, 3, 4, 7, 10, 11, 12
   as shipped in 1.2.0–1.3.0 (verified against crate source); keep only the
   residuals, re-verified against 1.3.0: no free-form `ToolSchema` metadata
   map, no reasoning field on the middleware-facing `harness::model::ModelDelta`,
   no `root_run_id` on `RunConfig`, no USD field on `Usage`.

## 1.3.0 delta (verified from the published crate source)

Same `rusqlite ^0.40 bundled` pin and feature set as 1.2.1. OpenHuman currently
pins the compatible patch release locally. New API this plan's workstreams
should use directly:

- `AgentEvent::ToolsFiltered { by, excluded, remaining }` — exposure decisions
  are now event-native (01.3).
- `AgentEvent::{BudgetReserved, BudgetReconciled}` + `BudgetLimits.
  max_cached_input_tokens` + reservation tracking — pre-spend
  reserve/reconcile (06).
- `AgentEvent::ControlApplied` + `MiddlewareControl::kind()/precedence()` —
  typed middleware control outcomes with defined precedence (sdk-gaps §13
  closed).
- `ContextualToolSelectionMiddleware::inheriting(...)` — parent→child
  narrowing composition built in (01.3, 07).
- `ToolPolicyMiddleware::{require_sandbox, require_approval,
  enforce_result_bytes}` builders (01.1).
- `ParallelOptions::{with_item_timeout, with_total_timeout,
  with_cancellation}` (08.1/08.2).
- `graph::testkit` TaskStore conformance contracts
  (`taskstore_concurrent_contract`, `taskstore_replay_contract`) (07.2, 11).
- `WorkspaceDescriptor::enforce(path, events)` — violation check that also
  emits events (08.5).
- `ModelSelection.allow_retired`; OpenAI-compat runtime model listing
  (`ModelListing`/`ModelListWire`) for catalog discovery (02.1/02.4).
- Registry: `ComponentKind::{Middleware, Checkpointer, TaskStore, Listener}`,
  alias diagnostics (`AliasBinding`, cross-kind name-reuse) (10).
- `EventRecord::with_stream_id` — stream ids on event records (05.1).

## Acceptance

- Both Cargo worlds `cargo check` clean with `sqlite` feature on:
  `cargo check --manifest-path Cargo.toml`,
  `cargo check --manifest-path Cargo.toml --all-features`, and
  `cargo check --manifest-path app/src-tauri/Cargo.toml`.
- One duplicate-free `cargo tree -i libsqlite3-sys` per world, rooted at
  `vendor/libsqlite3-sys-0.38.0`.
- Docs updated; sdk-gaps marked.

Verified commands:

- `cargo check --manifest-path Cargo.toml --message-format=short`
- `cargo check --manifest-path Cargo.toml --all-features --message-format=short`
- `cargo check --manifest-path app/src-tauri/Cargo.toml --message-format=short`
- `cargo tree --manifest-path Cargo.toml --all-features -i libsqlite3-sys`
- `cargo tree --manifest-path app/src-tauri/Cargo.toml --all-features -i libsqlite3-sys`
