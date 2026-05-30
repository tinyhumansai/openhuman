# meet

Google Meet integration domain. Lets a user ask the agent to join a Google Meet call as an **anonymous guest**. The core's responsibility is deliberately narrow: validate that the supplied URL is a Google Meet meeting URL, validate/trim the guest display name, and mint a `request_id` that the desktop (Tauri) shell uses to label the per-call CEF webview window and its data directory. All platform-specific work — opening the webview, driving Meet's join page over CDP, surfacing a virtual camera — lives in the Tauri shell (`app/src-tauri/src/...`), keeping this domain platform-agnostic.

## Responsibilities

- Validate that `meet_url` is `https://meet.google.com/<code>` (three lowercase-letter groups, each ≥3 chars, separated by `-`) or `https://meet.google.com/lookup/<id>` (single non-empty segment). Reject any other scheme, host, or path.
- Trim and validate `display_name`: non-empty, ≤64 chars, no control characters.
- Mint a stable UUID `request_id` for each accepted join attempt.
- Return a normalized echo (`ok`, `request_id`, normalized `meet_url`, `display_name`) for the shell to act on.
- Keep the meeting code (a credential) out of logs — only host + display-name length are logged.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/meet/mod.rs` | Export-focused. Module docstring + re-exports `all_meet_controller_schemas` / `all_meet_registered_controllers` and `types::*`. |
| `src/openhuman/meet/types.rs` | Serde request/response types: `MeetJoinCallRequest`, `MeetJoinCallResponse`. |
| `src/openhuman/meet/ops.rs` | Pure validation helpers: `validate_meet_url`, `validate_display_name` (plus private `is_meet_code` / `is_lookup_path`). Inline `tests` module. |
| `src/openhuman/meet/rpc.rs` | Async JSON-RPC handler `handle_join_call` — parses params, validates, mints `request_id`, returns CLI-compatible JSON via `RpcOutcome`. |
| `src/openhuman/meet/schemas.rs` | Controller schema definitions, `MEET_CONTROLLER_DEFS` table, `all_controller_schemas` / `all_registered_controllers` / `schemas`, and the `handle_join_call_wrap` handler wrapper. |
| `src/openhuman/meet/ops_tests.rs` | Sibling test file wired into `ops.rs` via `#[path = "ops_tests.rs"]`. |

## Public surface

- `types::MeetJoinCallRequest` — `{ meet_url: String, display_name: String }`.
- `types::MeetJoinCallResponse` — `{ ok: bool, request_id: String, meet_url: String, display_name: String }`.
- `all_meet_controller_schemas()` / `all_meet_registered_controllers()` (re-exported from `schemas`).
- `ops::validate_meet_url`, `ops::validate_display_name` (pure helpers, reachable for tests/CLI without the shell).

## RPC / controllers

| Method | Inputs | Outputs |
| --- | --- | --- |
| `openhuman.meet_join_call` (namespace `meet`, function `join_call`) | `meet_url` (string, required), `display_name` (string, required) | `ok` (bool), `request_id` (string UUID), `meet_url` (normalized string), `display_name` (trimmed string) |

Validates the URL and mints `request_id`; returns immediately. The actual webview lifecycle is the Tauri shell's job, keyed by `request_id`. Registered into the global controller registry via `src/core/all.rs` — no branches in `cli.rs` / `jsonrpc.rs` / `dispatch.rs`.

## Dependencies

- `crate::rpc::RpcOutcome` (`rpc.rs`) — wraps the handler result into CLI-compatible JSON.
- `crate::core::all::{ControllerFuture, RegisteredController}` (`schemas.rs`) — controller registry types for the boxed-future handler.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` (`schemas.rs`) — shared controller schema contract.
- External crates: `serde`/`serde_json` (types + handler), `url` (URL parsing/validation), `uuid` (request_id), `tracing` (diagnostics).

No dependencies on other `openhuman::*` domains.

## Used by

- `src/core/all.rs` — registers the controllers (`all_meet_registered_controllers`, line 255) and schemas (`all_meet_controller_schemas`, line 370) into the global registry.
- The desktop shell (`app/src-tauri/src/...`) consumes the minted `request_id` to open/label the per-call CEF webview (out of this crate).

> Note: `config::MeetConfig` (`src/openhuman/config/schema/`) is a separate config submodule named `meet`, unrelated to this domain.

## Notes / gotchas

- **Not a generic "open any URL in CEF" entrypoint** — host is hard-pinned to `meet.google.com`, scheme to `https`, and the path must match a Meet code or a single-segment `lookup/<id>`. Nested `lookup/` paths and look-alike hosts (e.g. `meet.google.evil.com`) are rejected.
- The meeting code is treated as a **credential**; it is intentionally kept out of logs (only host + display-name char count are emitted).
- Core does no webview/CDP work and persists no state — there is no `store.rs`, no `bus.rs`, and no agent tools (`tools.rs`). The domain is stateless validation + an ID mint.
