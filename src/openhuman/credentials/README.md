# credentials

Credential management for the OpenHuman app session and provider/OAuth auth profiles. Owns the on-disk **auth-profiles** store (encrypted JSON + OS keychain), the `app-session` JWT lifecycle (login / logout / session-state), per-provider token storage (e.g. API keys, OAuth token sets), the backend OAuth connect/handoff flows, and the Composio direct-mode (BYO key) credential slot. Exposes everything under the `auth.*` JSON-RPC / CLI namespace and runs the canonical session-teardown when a `SessionExpired` event fires.

## Responsibilities

- Store and validate the app session JWT (`app-session` provider, `default` profile), including local offline sessions and backend `GET /auth/me` validation.
- On login: activate the user-scoped openhuman directory, purge pre-login (anonymous) conversation threads on first activation, bind memory/conversation persistence, bootstrap subconscious, and start login-gated services (local AI, voice, dictation, screen intelligence, autocomplete).
- On logout / session-expiry: remove the JWT, clear the active-user marker, stop login-gated services, reset subconscious, and flip the scheduler-gate signed-out override.
- Persist arbitrary provider credentials (token + metadata fields) as named auth profiles; list/remove/set-active; prefix-list profiles for grouped namespaces (e.g. `channel:*`).
- Run backend OAuth flows: connect URL, list integrations, fetch integration handoff tokens, fetch one-time client key, revoke integration.
- Store/read/clear the Composio direct-mode API key (`composio-direct` provider).
- Encrypt/decrypt arbitrary secrets via the `SecretStore`.
- Manage the on-disk profile store with secret-at-rest handling: OS keychain when available, ChaCha20-Poly1305 encrypted JSON fallback otherwise, with legacy cipher migration, keychain promotion, corrupt-store quarantine, and crash-safe file locking.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Export-focused. Re-exports `core::*`, `ops` (also as `rpc`), Composio-direct helpers, schema controllers (`all_credentials_controller_schemas` / `all_credentials_registered_controllers`), and backend OAuth REST types from `crate::api::rest`. |
| `core.rs` | `AuthService` facade over `AuthProfilesStore` — store/get/remove/set-active profiles, resolve bearer token, profile-id selection logic (override → active → default → any-for-provider), provider normalization, state-dir derivation. |
| `profiles.rs` | The persistence engine. `AuthProfile` / `TokenSet` / `AuthProfileKind` / `AuthProfilesData` types and `AuthProfilesStore` — atomic JSON read/write, keychain vs encrypted-JSON secret handling, legacy migration, corrupt-store quarantine, PID-aware stale-lock recovery. |
| `ops.rs` | Business logic + RPC entry points (returns `RpcOutcome<T>`). Session lifecycle (`store_session`/`clear_session`/`auth_get_*`), login/logout service orchestration, provider-credential CRUD, OAuth flows, Composio-direct key helpers, secret encrypt/decrypt. Re-exported as `rpc`. |
| `schemas.rs` | `auth.*` controller schemas + `handle_*` dispatchers delegating to `ops`. Defines `all_controller_schemas` / `all_registered_controllers`. |
| `session_support.rs` | Session/auth helpers: `build_session_state`, `get_session_token`, `load_app_session_profile`, `summarize_auth_profile`, local-session detection/slug, field parsing. Shared by RPC and the HTTP host. |
| `responses.rs` | Response DTOs: `AuthStateResponse`, `AuthProfileSummary`. |
| `cli.rs` | CLI auth entrypoints (`cli_auth_login/logout/status/list`) that branch on `app-session` vs provider; `--field key=value` parsing. |
| `bus.rs` | `SessionExpiredSubscriber` — `EventHandler` for `DomainEvent::SessionExpired`. |
| `ops_tests.rs`, `profiles_tests.rs`, `schemas_tests.rs` | Sibling test suites (`#[path = ...]`). Plus inline `#[cfg(test)]` tests in `core.rs`, `session_support.rs`, `bus.rs`, `cli.rs`. |

## Public surface

- **`AuthService`** (`core.rs`) — `from_config`, `new`, `load_profiles`, `store_provider_token`, `set_active_profile`, `remove_profile`, `get_profile`, `get_provider_bearer_token`.
- **Constants** — `APP_SESSION_PROVIDER` (`"app-session"`), `DEFAULT_AUTH_PROFILE_NAME` (`"default"`), `COMPOSIO_DIRECT_PROVIDER` (`"composio-direct"`).
- **Helpers** — `normalize_provider`, `default_profile_id`, `select_profile_id`, `state_dir_from_config`, `profile_id`.
- **Types** (`profiles.rs`) — `AuthProfile`, `AuthProfileKind` (`OAuth`/`Token`), `TokenSet`, `AuthProfilesData`, `AuthProfilesStore`.
- **Ops/RPC** (`ops`, re-exported as `rpc`) — `store_session`, `clear_session`, `auth_get_state`, `auth_get_session_token_json`, `auth_get_me`, `consume_login_token`, `auth_create_channel_link_token`, `store_provider_credentials`, `remove_provider_credentials`, `list_provider_credentials`, `list_provider_credentials_by_prefix`, `oauth_connect`, `oauth_list_integrations`, `oauth_fetch_integration_tokens`, `oauth_fetch_client_key`, `oauth_revoke_integration`, `encrypt_secret`, `decrypt_secret`, `start_login_gated_services`, `stop_login_gated_services`.
- **Composio-direct** — `store_composio_api_key`, `get_composio_api_key`, `clear_composio_api_key`, `rpc_store_composio_api_key`.
- **Backend OAuth re-exports** (from `crate::api::rest`) — `BackendOAuthClient`, `ConnectResponse`, `IntegrationSummary`, `IntegrationTokensHandoff`, `decrypt_handoff_blob`, `user_id_from_auth_me_payload`, `user_id_from_profile_payload`.
- **Schema controllers** — `all_credentials_controller_schemas`, `all_credentials_registered_controllers`.

## RPC / controllers

Namespace `auth` (JSON-RPC `openhuman.auth_*` / CLI). Defined in `schemas.rs`:

| Method | Description |
| --- | --- |
| `auth_store_session` | Store + validate app session JWT. |
| `auth_clear_session` | Remove stored app session credentials. |
| `auth_get_state` | Current auth/session state (`AuthStateResponse`). |
| `auth_get_session_token` | Read stored app session token. |
| `auth_get_me` | Fetch current authenticated backend user profile. |
| `auth_consume_login_token` | Consume one-time login handoff token → session JWT. |
| `auth_create_channel_link_token` | Short-lived channel link token (telegram/discord). |
| `auth_store_provider_credentials` | Store provider credentials for a profile. |
| `auth_remove_provider_credentials` | Remove provider credentials. |
| `auth_list_provider_credentials` | List stored provider credentials (optional provider filter). |
| `auth_oauth_connect` | Create OAuth connect URL for a provider. |
| `auth_oauth_list_integrations` | List OAuth integrations for the session. |
| `auth_oauth_fetch_integration_tokens` | Fetch integration handoff tokens. |
| `auth_oauth_fetch_client_key` | Fetch one-time client key share for an encrypted integration. |
| `auth_oauth_revoke_integration` | Revoke an OAuth integration. |

Note: `list_provider_credentials_by_prefix` and the Composio-direct/secret helpers are public ops but not registered as `auth.*` controllers here — they are called directly by other domains.

## Agent tools

None. This module owns no agent tools (`tools.rs` does not exist).

## Events

`bus.rs` — `SessionExpiredSubscriber` (`name() == "credentials::session_expired_handler"`, domain filter `["auth"]`) **subscribes** to `DomainEvent::SessionExpired`. On a non-local session it flips the scheduler gate to signed-out and calls `clear_session`; for a local offline session it re-enables the gate and no-ops. This module does not publish events directly (publishers of `SessionExpired` are 401-detection sites elsewhere).

## Persistence

`AuthProfilesStore` (`profiles.rs`) writes `auth-profiles.json` in the config state directory (parent of `config.config_path`, user-scoped after login). Layout: `schema_version` (current = 1), `updated_at`, `active_profiles` (provider → profile-id), `profiles` (id → profile). Secret handling:

- **OS keychain** when available (`crate::openhuman::keyring::is_available`): all token fields stored under key `auth:{profile_id}` namespaced by a per-user id derived from the state dir; JSON keeps no secret fields.
- **Encrypted-JSON fallback** (headless/CI): token fields encrypted via `SecretStore` (ChaCha20-Poly1305).
- Loads migrate legacy `enc:`/`enc2:` cipher fields and promote secrets into the keychain; unrecoverable (un-decryptable / bad-`kind`) profiles are dropped rather than poisoning the whole store; unparseable files are quarantined to `auth-profiles.corrupt-<ts>.json` and reset to empty.
- Mutations are guarded by `auth-profiles.lock` (PID-stamped). Stale/leaked/malformed locks are reclaimed by liveness + age checks to avoid the "stuck on Initializing OpenHuman" hang.

## Dependencies

- `crate::openhuman::config` — `Config`, config load (`load_config_with_timeout`), user-dir activation (`default_root_openhuman_dir`, `user_openhuman_dir`, `read/write/clear_active_user`, `pre_login_user_dir`), onboarding state.
- `crate::openhuman::keyring` — `SecretStore` (encrypt/decrypt) and OS keychain `get`/`set`/`delete`/`is_available`.
- `crate::openhuman::scheduler_gate` — signed-out override flipped on login/logout/session-expiry.
- `crate::openhuman::memory_conversations` — purge pre-login threads, bind conversation persistence after login.
- `crate::openhuman::memory` — bind memory client to the active workspace after login.
- `crate::openhuman::subconscious` — post-login bootstrap / user-switch reset.
- `crate::openhuman::inference`, `::voice`, `::screen_intelligence`, `::autocomplete` — login-gated services started/stopped.
- `crate::api::config`, `::jwt`, `::rest` — backend API URL, session-token read, `BackendOAuthClient` + OAuth/handoff types.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`), `crate::core` (`ControllerSchema`/`FieldSchema`/`TypeSchema`), `crate::core::event_bus` (`DomainEvent`/`EventHandler`), `crate::rpc::RpcOutcome` — controller registry + RPC envelope + event bus.

## Used by

Many domains consume `AuthService` / session helpers / Composio-direct key, including: `src/core/{all,auth,jsonrpc}.rs` (controller wiring + auth gate), `src/api/jwt.rs`, `app_state/ops.rs` (session snapshot), `channels/*` (managed credentials), `composio/{client,ops}.rs` (BYO key), `config/schema/*`, `embeddings/cloud.rs`, `encryption/ops.rs`, `http_host/auth.rs`, `inference/*` (provider auth, OpenAI OAuth), `migrations/unify_ai_provider_settings.rs`, `referral/ops.rs`, `subconscious/engine.rs`, and `webhooks`.

## Notes / gotchas

- `mod.rs` re-exports `ops` both as `ops::*` and as `pub use ops as rpc` — call sites use `credentials::rpc::*`; this is the documented `rpc.rs`-equivalent exception (no separate `rpc.rs` file exists).
- Doc comments in `responses.rs` / `session_support.rs` reference `crate::core_server` / `crate::core_server::types`; those paths are historical — the actual transport crate is `src/core/`.
- `store_session` does heavy orchestration beyond just storing a token (directory activation, thread purge, service startup). Treat it as the login funnel, not a thin setter.
- Local offline sessions are detected purely by the JWT signature segment being literally `local` (`is_local_session_token`); they skip backend validation and are never treated as expired.
- Secrets are never logged — debug lines record only lengths/markers, honoring the CLAUDE.md redaction rule.
