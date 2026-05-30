# team

Team management RPC adapters. This domain is a **thin proxy to the hosted backend**: every operation forwards an authenticated HTTP request to the OpenHuman backend (`/teams/*`) and returns the raw JSON response verbatim. It owns **no local state, no domain types, and no server-side authorization** — the backend enforces team ownership, role permissions, and tenant isolation; non-authorized callers receive the backend's 401/403 surfaced as an RPC error string. Covers team CRUD, membership, role changes, invites, usage, and active-team switching.

## Responsibilities

- Fetch the current user's active-team usage (`/teams/me/usage`).
- List teams for the authenticated user, fetch a single team, create / update / delete teams.
- Switch the active team, leave a team, join a team via invite code.
- List, create, and revoke team invites.
- Remove members and change member roles.
- Validate inputs (non-empty trimmed ids/names/codes) **before** any network call, with deterministic field-precedence in error messages.
- Percent-encode path segments safely (no path-injection via team/user ids).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/team/mod.rs` | Export-only: declares `ops`/`schemas`, re-exports all ops fns and the controller-schema pair. |
| `src/openhuman/team/ops.rs` | Business logic — async fns that build a path, attach the session JWT, call the backend, and wrap the response in `RpcOutcome::single_log`. Contains URL-path builder + id-normalization helpers and their unit tests. |
| `src/openhuman/team/schemas.rs` | Controller schemas (`all_team_controller_schemas`, `all_team_registered_controllers`, `team_schemas`), param structs, and `handle_*` adapters that load config, deserialize params, and delegate to `ops`. |
| `src/openhuman/team/schemas_tests.rs` | Sibling test module for `schemas.rs` (wired via `#[path]`). |

## Public surface

Re-exported from `mod.rs`:

- **Ops (all `async fn(... ) -> Result<RpcOutcome<Value>, String>`):** `get_usage`, `list_members`, `list_teams`, `get_team`, `create_team`, `update_team`, `delete_team`, `switch_team`, `leave_team`, `join_team`, `create_invite`, `remove_member`, `change_member_role`, `list_invites`, `revoke_invite`.
- **Schemas:** `all_team_controller_schemas`, `all_team_registered_controllers`, `team_schemas`.

Internal helpers in `ops.rs` (`require_token`, `normalize_id`, `build_api_path`, `get_authed_value`) are private.

## RPC / controllers

Namespace `team`. Registered controllers (RPC method `openhuman.team_<function>`):

| Method | Backend call | Required inputs | Optional |
| --- | --- | --- | --- |
| `team_get_usage` | `GET /teams/me/usage` | — | — |
| `team_list_members` | `GET /teams/:teamId/members` | `teamId` | — |
| `team_list_teams` | `GET /teams` | — | — |
| `team_get_team` | `GET /teams/:teamId` | `teamId` | — |
| `team_create_team` | `POST /teams` | `name` | — |
| `team_update_team` | `PUT /teams/:teamId` | `teamId` | `name` |
| `team_delete_team` | `DELETE /teams/:teamId` | `teamId` | — |
| `team_switch_team` | `POST /teams/:teamId/switch` | `teamId` | — |
| `team_leave_team` | `POST /teams/:teamId/leave` | `teamId` | — |
| `team_join_team` | `POST /teams/join` | `code` | — |
| `team_create_invite` | `POST /teams/:teamId/invites` | `teamId` | `maxUses`, `expiresInDays` |
| `team_list_invites` | `GET /teams/:teamId/invites` | `teamId` | — |
| `team_revoke_invite` | `DELETE /teams/:teamId/invites/:inviteId` | `teamId`, `inviteId` | — |
| `team_remove_member` | `DELETE /teams/:teamId/members/:userId` | `teamId`, `userId` | — |
| `team_change_member_role` | `PUT /teams/:teamId/members/:userId/role` | `teamId`, `userId`, `role` | — |

Outputs are raw backend JSON (`result` field; array for list endpoints). Params are camelCase; `handle_*` fns deserialize via `serde_json::from_value` and surface `invalid params: …` on failure.

## Agent tools

None. This domain exposes no agent tools.

## Events

None. No `bus.rs`; publishes/subscribes to no `DomainEvent`s.

## Persistence

None local. State lives in the hosted backend. The only stored value it reads is the app-session JWT, fetched via `crate::api::jwt::get_session_token` (written by `auth_store_session`); it is sent as `Authorization: Bearer …` and never logged.

## Dependencies

- `crate::api::config::effective_backend_api_url` — resolves the backend base URL from `Config.api_url`.
- `crate::api::jwt::get_session_token` — pulls the session JWT used to authenticate every request.
- `crate::api::BackendOAuthClient` — HTTP client; `authed_json(token, method, path, body)` performs the authed call.
- `crate::openhuman::config::Config` — config passed into every op; `config::rpc::load_config_with_timeout` loads it inside each `handle_*`.
- `crate::core::all::{ControllerFuture, RegisteredController}` and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registry types.
- `crate::rpc::RpcOutcome` — return wrapper (`single_log`).
- `reqwest` (`Method`, `Url`) for HTTP + path building; `serde` / `serde_json` for params and bodies.

## Used by

- `src/core/all.rs` — registers `all_team_registered_controllers()` into the controller registry and `all_team_controller_schemas()` into the schema list (the standard controller-only exposure path). No domain branches in `cli.rs` / `jsonrpc.rs`.

## Notes / gotchas

- **Pure proxy.** No type modeling of teams/members/invites — everything is `serde_json::Value` passthrough; the backend is the source of truth.
- **Error rendering:** `get_authed_value` maps failures with `{e:#}` (full anyhow chain), deliberately not `e.to_string()`, so the underlying cause (DNS/TLS/timeout/non-2xx) is preserved for Sentry rather than the truncated `backend request GET /teams` label (see the in-code note referencing OPENHUMAN-TAURI-AD / TAURI-B2).
- **Path safety:** `build_api_path` clears and pushes segments through `url::path_segments_mut`, percent-encoding reserved chars, spaces, and Unicode — ids like `team/with?reserved` cannot escape their segment.
- **Validation precedence is deterministic** (tested): `team_id` is normalized before `user_id` before `role`, so error messages are stable regardless of which other fields are also invalid.
- `update_team` only includes `name` in the body when present and non-empty after trimming; an empty/whitespace name is dropped rather than sent.
- Missing/blank session token yields `no backend session token; run auth_store_session first` before any network attempt.
