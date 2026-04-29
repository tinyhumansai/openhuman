# App State

Aggregator that the React shell polls every few seconds to render the OS-level chrome (auth user, autocomplete status, accessibility status, local-AI status, service health, onboarding tasks). Owns the on-disk `app-state.json`, an in-memory current-user cache, and the merge/patch surface for shell-managed local fields. Does NOT own any of the underlying domain state — it only assembles snapshots from peer domains and persists shell-side onboarding metadata.

## Public surface

- `pub struct AppStateSnapshot` — `ops.rs` — composite payload returned to the shell (auth user, runtime status, autocomplete, local AI, accessibility, onboarding).
- `pub struct RuntimeSnapshot` — `ops.rs` — runtime sub-section of the snapshot.
- `pub struct StoredAppState` — `ops.rs` — disk schema persisted to `<workspace>/app-state.json`.
- `pub struct StoredAppStatePatch` — `ops.rs` — partial-update payload used by `update_local_state`.
- `pub struct StoredOnboardingTasks` — `ops.rs:42-50` — shell-tracked onboarding completion flags (accessibility permission, local model consent, etc.).
- `pub async fn snapshot() -> Result<RpcOutcome<AppStateSnapshot>, String>` — `ops.rs` — collect the full snapshot.
- `pub async fn update_local_state(...)` — `ops.rs` — apply a `StoredAppStatePatch`.
- RPC `app_state.{snapshot, update_local_state}` — `schemas.rs:20-37` (re-exported via `all_app_state_controller_schemas` / `all_app_state_registered_controllers`).

## Calls into

- `src/openhuman/config/` — `config_rpc::*` for `Config` reads and the workspace dir resolver.
- `src/openhuman/autocomplete/` — `AutocompleteStatus` snapshot.
- `src/openhuman/local_ai/` — `LocalAiStatus` snapshot.
- `src/openhuman/screen_intelligence/` — `AccessibilityStatus` snapshot.
- `src/openhuman/service/` — `ServiceState` / `ServiceStatus` runtime info.
- `src/openhuman/credentials/` — `session_support::build_session_state` for the auth slice.
- `src/api/{config,jwt}` — backend base URL + bearer token used by the cached current-user fetch.

## Called by

- `src/openhuman/agent/harness/session/builder.rs` — agent builder reads cached app state when resolving identity.
- `src/core/all.rs` — registers `all_app_state_*` controllers; the shell hits these via `core_rpc_relay`.
- `app/src/` — Tauri shell consumes the snapshot in its polling loops (out of scope for this README).

## Tests

- This domain has no `*_tests.rs` siblings; coverage is exercised indirectly through controller-registry tests in `src/core/` and through the JSON-RPC harness `tests/json_rpc_e2e.rs`.
