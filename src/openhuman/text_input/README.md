# text_input

Text input intelligence — read, insert, and preview ("ghost text") suggestions in the OS-focused input field. A thin orchestration layer: it owns the request/response types, RPC controller surface, and a standalone CLI, but delegates **all** platform work (focus inspection, text application, overlay rendering) to `accessibility::*`. Consumed by autocomplete, voice control, and other text-aware features.

## Responsibilities

- Read the currently focused text field's contents, app name, role, selection, and (optionally) screen bounds; flag whether the field belongs to a terminal app.
- Insert text into the focused field, with optional focus validation (expected app / role) before writing.
- Show a ghost-text overlay near the focused field with a TTL auto-dismiss, and dismiss it.
- Accept a ghost suggestion atomically (dismiss overlay → validate focus → insert).
- Expose all of the above as `text_input.*` JSON-RPC controllers.
- Provide a standalone `openhuman text-input` CLI for testing read/insert/ghost/dismiss flows and a lightweight dev JSON-RPC server, without launching the full desktop app.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/text_input/mod.rs` | Export-only. Re-exports `ops` (as `rpc`), `types`, and the schema registry pair (`all_text_input_controller_schemas` / `all_text_input_registered_controllers`). |
| `src/openhuman/text_input/types.rs` | Serde request/response types (`ReadFieldParams`/`Result`, `InsertTextParams`/`Result`, `ShowGhostTextParams`/`Result`, `DismissGhostTextResult`, `AcceptGhostTextParams`/`Result`, `FieldBounds`) + `FieldBounds ↔ accessibility::ElementBounds` conversions. |
| `src/openhuman/text_input/ops.rs` | Business logic (canonical handler file). Async fns `read_field`, `insert_text`, `show_ghost`, `dismiss_ghost`, `accept_ghost` returning `RpcOutcome<T>`; all delegate to `accessibility::*`. |
| `src/openhuman/text_input/schemas.rs` | Controller schemas + `handle_*` wrappers that deserialize params and call `ops`; defines `all_controller_schemas` / `all_registered_controllers`. |
| `src/openhuman/text_input/cli.rs` | `openhuman text-input <subcommand>` CLI: `run` (dev JSON-RPC server on port 7798), `read`, `insert`, `ghost`, `dismiss`. |

## Public surface

From `mod.rs` re-exports:

- `ops::*` (also aliased as `rpc`): `read_field`, `insert_text`, `show_ghost`, `dismiss_ghost`, `accept_ghost`.
- `types::*`: `ReadFieldParams`, `ReadFieldResult`, `InsertTextParams`, `InsertTextResult`, `ShowGhostTextParams`, `ShowGhostTextResult`, `DismissGhostTextResult`, `AcceptGhostTextParams`, `AcceptGhostTextResult`, `FieldBounds`.
- `all_text_input_controller_schemas()` / `all_text_input_registered_controllers()` — registry entry points.
- `cli::run_text_input_command(args)` — `pub(crate)` CLI dispatch.

## RPC / controllers

Namespace `text_input` (5 controllers):

| Method | Inputs | Output |
| --- | --- | --- |
| `text_input.read_field` | `include_bounds?` (bool) | `ReadFieldResult` (`app_name`, `role`, `text`, `selected_text`, `bounds?`, `is_terminal`) |
| `text_input.insert_text` | `text` (required), `validate_focus?`, `expected_app?`, `expected_role?` | `InsertTextResult` (`inserted`, `error?`) |
| `text_input.show_ghost` | `text` (required), `ttl_ms?` (default 3000), `bounds?` (JSON `{x,y,width,height}`) | `ShowGhostTextResult` (`shown`, `error?`) |
| `text_input.dismiss_ghost` | — | `DismissGhostTextResult` (`dismissed`) |
| `text_input.accept_ghost` | `text` (required), `validate_focus?`, `expected_app?`, `expected_role?` | `AcceptGhostTextResult` (`inserted`, `error?`) |

Registered into the global registry via `src/core/all.rs` (`controllers.extend(...)` and `schemas.extend(...)`). The capability catalog (`about_app` via `all.rs`) describes it as "Read, insert, and preview text in the OS-focused input field."

## Dependencies

- **`openhuman::accessibility`** — does all platform work: `focused_text_context_verbose` (read field), `is_terminal_app`, `apply_text_to_focused_field` (insert), `show_overlay` / `hide_overlay` (ghost text), `validate_focused_target` (focus validation), and `ElementBounds` (bounds type mirrored by `FieldBounds`).
- **`core::all`** — `ControllerFuture`, `RegisteredController` for controller registration.
- **`core::{ControllerSchema, FieldSchema, TypeSchema}`** — schema definitions.
- **`rpc::RpcOutcome`** — handler return wrapper.
- **`core::logging`**, **`core::types`** (`RpcRequest`/`RpcSuccess`/`RpcFailure`/`RpcError`), **`core::jsonrpc`** (`default_state`, `invoke_method`) — used only by the dev CLI in `cli.rs`.

## Used by

- `src/core/all.rs` — registers the controllers/schemas and the capability-catalog description.
- `src/core/cli.rs` — dispatches the `text-input` top-level subcommand to `cli::run_text_input_command`.

(Note: `src/openhuman/voice/server.rs` references its own `super::text_input` symbol, not this domain module.)

## Notes / gotchas

- **Pure orchestration, no state.** No `store.rs`, no persistence, no event-bus subscribers, no agent tools, no config reads — every call resolves against live OS focus at request time.
- **Error contract differs per method.** `insert_text` / `accept_ghost` wrap platform failures as `Ok(RpcOutcome { value: { inserted: false, error: Some(..) } })` (never `Err`), so JSON-RPC callers always get a structured result. `read_field` and `show_ghost` may bubble accessibility errors up as `Err` (e.g. when reading focused-field bounds fails). Empty `text` is rejected with `Err` up front.
- **`dismiss_ghost` is idempotent** — it discards any `hide_overlay()` error and always returns `dismissed: true`.
- **Focus validation triggers** when `validate_focus` is true **or** either `expected_app` / `expected_role` is set.
- **`show_ghost` bounds fallback:** if no `bounds` provided, it reads the focused field; if that has no bounds, defaults to `{0,0,200,24}`.
- **`read_field` params tolerate omitted `include_bounds`** (`Option<bool>` + `#[serde(default)]`); its handler uses `deserialize_params_or_default` so an empty param map yields defaults rather than erroring.
- **Dev CLI** (`text-input run`) stands up a minimal Axum server (`/health`, `/rpc`) routing through `core::jsonrpc::invoke_method`, default port 7798. Useful for exercising the domain from a terminal without the desktop app. Unit tests deliberately only pin guard-clause/validation logic since post-guard paths require a live OS display.
