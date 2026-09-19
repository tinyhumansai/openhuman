# App State

Aggregator that the React shell polls every few seconds (`openhuman.app_state_snapshot`) to render the OS-level chrome: the stored credential and user, local-AI status, service health, onboarding tasks, keyring status, config-recovery notice. Owns the on-disk `app-state.json` and the merge/patch surface for shell-managed local fields. Does NOT own any of the underlying domain state — it assembles snapshots from peer domains and persists shell-side onboarding metadata.

The snapshot never talks to the backend. The user it reports is the payload the host handed the core with `auth.set_credential` (the host's own `/auth/me` answer); the *live* current user is the session owner's business — the Tauri shell's session cache (`openhuman_tinyhumans::session::CurrentUserCache`) — and the frontend merges it in (`app/src/services/coreStateApi.ts`).

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | `pub use ops::*`, `recovery_signal::{config_recovered_this_session, latch_from_config}`, and the `all_app_state_*` / `app_state_schemas` controller aggregators. |
| `ops.rs` | Aggregator only: declares the submodules below and attaches the sibling test file. |
| `ops/types.rs` | Serde types (`StoredOnboardingTasks`, `StoredAppState`, `AppStateSnapshot`, `RuntimeSnapshot`, `StoredAppStatePatch`). |
| `ops/state_file.rs` | `app-state.json` load/save with corruption quarantine (`load_stored_app_state`, `save_app_state`). |
| `ops/runtime_snapshot.rs` | The local-AI + service half of the snapshot (10s cache, single-flighted, #4249). |
| `ops/snapshot.rs` | `snapshot()` and `update_local_state()`. |
| `recovery_signal.rs` | Process-lifetime latch for "config.toml was recovered from corruption this session" (#5167). |
| `schemas.rs` | `app_state` controller schemas and thin handlers. |
| `*_tests.rs` | Sibling test files attached with `#[cfg(test)] #[path = ...] mod`. |

## Public surface

- `AppStateSnapshot` — composite payload: `auth` (`AuthStateResponse`: `isAuthenticated`, `userId`, `user`, `profileId`, `credential`, `expiresAt`), `session_token`, `current_user` (= `auth.user`), `onboarding_completed`, `chat_onboarding_completed`, `analytics_enabled`, `local_state`, `keyring_status`, `runtime` (`RuntimeSnapshot { local_ai: LocalAiStatus, service: ServiceStatus }`), `health`, `config_recovered`.
- `StoredAppState` — disk schema persisted to `<workspace_dir>/state/app-state.json` (`encryption_key`, `onboarding_tasks`, `keyring_consent`). `StoredOnboardingTasks` holds the shell-tracked onboarding flags plus `enabled_tools` and `connected_sources`.
- `StoredAppStatePatch` — partial update (`Option<Option<_>>` per field) applied by `update_local_state`.
- `pub async fn snapshot() -> Result<RpcOutcome<AppStateSnapshot>, String>` — full snapshot; one auth-profile load per call, no network.
- `pub async fn update_local_state(StoredAppStatePatch) -> Result<RpcOutcome<StoredAppState>, String>` — merge under `APP_STATE_FILE_LOCK` and save atomically.
- `pub(crate) fn load_stored_app_state(&Config)` / `pub fn save_app_state(&Config, &StoredAppState)` — direct disk access; a corrupt file is renamed to `app-state.json.corrupted.<ts>` and replaced with defaults.
- `latch_from_config(&Config)` / `config_recovered_this_session()` — recovery latch.
- RPC `app_state.{snapshot, update_local_state}` via `all_app_state_controller_schemas` / `all_app_state_registered_controllers`.

The signed-in identity for prompts and Sentry is `security::credentials::identity::peek_credential_user_identity()`, seeded from the same stored payload.

## Calls into

- `crate::config` — `config::rpc::load_config_with_timeout`.
- `crate::security::credentials` — `session_support::{load_app_session_profile, session_state_from_profile, session_token_from_profile}`; `crate::security::keyring_consent` for `KeyringStatus` / `ConsentPreference`.
- `crate::inference` — `LocalAiStatus` via `inference::rpc::inference_status`.
- `crate::platform::service` — `ServiceState` / `ServiceStatus`; `crate::platform::health::snapshot`.

## Called by

- `crates/openhuman-core/src/core/all.rs` — registers `all_app_state_registered_controllers()`; the shell reaches them through `coreRpcClient` → `relay_http_rpc`.
- `crates/openhuman-core/src/core/jsonrpc.rs` — `latch_from_config` at runtime bootstrap.
- `crates/openhuman-core/src/agent/session_host/builder/factory.rs` — `load_stored_app_state` to read `onboarding_tasks.enabled_tools` for tool filtering.
- `crates/openhuman-core/src/security/keyring_consent/ops.rs` — persists the consent choice through `update_local_state`.

## Tests

- `ops_tests.rs`, `recovery_signal_tests.rs`, `schemas_tests.rs`.
- JSON-RPC shape: `tests/json_rpc_e2e.rs` (`json_rpc_app_state_snapshot_returns_runtime_shape`), `tests/config_auth_app_state_connectivity_e2e.rs`, and `tests/raw_coverage/app_state_credentials_raw_coverage_e2e.rs`.
