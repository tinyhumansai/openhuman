# credentials

Credential management for the backend credential the core authenticates with and for provider/OAuth auth profiles. Owns the on-disk **auth-profiles** store (encrypted JSON + OS keychain), the `app-session` / `api-key` credential slots and everything the core does when one is installed or removed, per-provider token storage (e.g. API keys, OAuth token sets), the backend OAuth connect/handoff flows, and the Composio direct-mode (BYO key) credential slot. Exposes everything under the `auth.*` JSON-RPC / CLI namespace and runs the canonical sign-out teardown when a `SessionExpired` event fires.

**The core never obtains, validates, exchanges or refreshes a credential.** Login-token exchange, `GET /auth/me` validation and the current-user cache are the host's job — the Tauri shell and the TUI through `openhuman_tinyhumans::session`, an embedder through `openhuman_embed::Auth`, an operator through the CLI or the boot env vars. They hand the result over with `auth.set_credential`.

## Responsibilities

- Install a backend credential (`set_credential`): a session JWT (with the user id / payload the host resolved — or the JWT's subject claim), a TinyHumans API key, or the offline local token. A JWT whose `exp` is already past is refused; a session with no resolvable user id is refused.
- On install: activate the user-scoped openhuman directory, purge pre-login (anonymous) conversation threads on first activation, bind memory/conversation persistence, start the credential-gated services (local AI, voice server, dictation listener, always-on voice), open the scheduler gate, scope Sentry and the prompt-layer identity. A repeat install with the **same token and user** is a cheap refresh of the stored user payload (this is how a host replaces a `pendingBackendValidation` placeholder with its later `/auth/me` answer); a **different user** is signed out first.
- On removal / session-expiry (`clear_credential`): remove the profile, clear the active-user marker, stop the gated services, rebind process globals to the signed-out workspace, and close the scheduler-gate override.
- Boot seeding for headless hosts: `OPENHUMAN_BACKEND_API_KEY` / `OPENHUMAN_BACKEND_SESSION_TOKEN` install a credential on a fresh store (never overwrite one).
- Persist arbitrary provider credentials (token + metadata fields) as named auth profiles; list/remove/set-active; prefix-list profiles for grouped namespaces (e.g. `channel:*`).
- Run backend OAuth flows (bearer-only): connect URL, list integrations, fetch integration handoff tokens, fetch one-time client key, revoke integration; mint channel link tokens.
- Store/read/clear the Composio direct-mode API key (`composio-direct` provider).
- Encrypt/decrypt arbitrary secrets via the `SecretStore`.
- Manage the on-disk profile store with secret-at-rest handling: OS keychain when available, ChaCha20-Poly1305 encrypted JSON fallback otherwise, with legacy cipher migration, keychain promotion, corrupt-store quarantine, and crash-safe file locking.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Export-focused. Re-exports `core::*`, `ops` (also as `rpc`), Composio-direct helpers, schema controllers (`all_credentials_controller_schemas` / `all_credentials_registered_controllers`), and backend OAuth REST types from `crate::api::rest`. |
| `core.rs` | `AuthService` facade over `AuthProfilesStore` — store/get/remove/set-active profiles, resolve bearer token, profile-id selection logic (override → active → default → any-for-provider), provider normalization, state-dir derivation. |
| `profiles.rs` | The persistence engine. `AuthProfile` / `TokenSet` / `AuthProfileKind` / `AuthProfilesData` types and `AuthProfilesStore` — atomic JSON read/write, keychain vs encrypted-JSON secret handling, legacy migration, corrupt-store quarantine, PID-aware stale-lock recovery. |
| `api_key.rs` | The `api-key` profile: `store_api_key[_in]`, `get_api_key[_in]`, `has_api_key[_in]`, `clear_api_key`. |
| `ops.rs` + `ops/` | Business logic + RPC entry points (returns `RpcOutcome<T>`). `ops/credential.rs` (`set_credential` / `clear_credential`, plus the historical `store_session` / `clear_session` names), `ops/user_scope.rs` (user-dir activation and process-global rebinding), `ops/gated_services.rs` (credential-gated services), `ops/boot_env.rs` (env seeding), `ops/session_query.rs` (`auth_get_state`, `auth_get_session_token_json`), `ops/oauth.rs` (`oauth_fetch_client_key` only — its route has no SDK method yet), `ops/provider_credentials.rs`, `ops/composio.rs`, `ops/secrets.rs`. Re-exported as `rpc`. |
| `schemas.rs` | `auth.*` controller schemas + `handle_*` dispatchers delegating to `ops`. Defines `all_controller_schemas` / `all_registered_controllers`. |
| `session_support.rs` | `CredentialKind`, `BackendCredential`, `resolve_backend_credential`, `require_live_session_token`, `has_backend_credential`, `build_session_state`, `get_session_token`, `load_app_session_profile`, `user_id_from_jwt_claims`, local-session detection/slug, field parsing. Shared by RPC and the HTTP host. |
| `identity.rs` | The signed-in user's identity slot (`peek_credential_user_identity`), seeded from the host-supplied payload for prompts and Sentry. |
| `responses.rs` | Response DTOs: `AuthStateResponse`, `AuthProfileSummary`. |
| `bus.rs` | `SessionExpiredSubscriber` — `EventHandler` for `DomainEvent::SessionExpired`. |
| `tools.rs` | Agent tools `credential_list`, `session_state`, `oauth_connect_url`, `oauth_list` (these two dispatch the hosted `auth_oauth_*` controllers by wire name through the registry and answer `BACKEND_UNAVAILABLE:` without the hosted layer). |
| `*_tests.rs` | Sibling test suites (`#[path = ...]`). |

## Public surface

- **`AuthService`** (`core.rs`) — `from_config`, `new`, `load_profiles`, `store_provider_token`, `set_active_profile`, `remove_profile`, `get_profile`, `get_provider_bearer_token`.
- **Constants** — `APP_SESSION_PROVIDER` (`"app-session"`), `DEFAULT_AUTH_PROFILE_NAME` (`"default"`), `COMPOSIO_DIRECT_PROVIDER` (`"composio-direct"`), `api_key::API_KEY_PROVIDER` (`"api-key"`).
- **Credential kinds** — `session_support::CredentialKind {Session, ApiKey, Local}` (wire values `"session"`, `"api-key"`, `"local"`). `session_support::resolve_backend_credential` returns `BackendCredential::ApiKey` when a key is stored (before any session classification) and `BackendCredential::Session` otherwise; `BackendOAuthClient::authed_json` sends an API key as `x-api-key` and a session as `Authorization: Bearer`, while `OpenHumanBackendModel::resolve_bearer` sends the key as the bearer for managed inference. `has_backend_credential` is the boot-time "signed in?" question the scheduler gate asks; `SessionExpired` is ignored for an API-key runtime and for a local session.
- **Helpers** — `normalize_provider`, `default_profile_id`, `select_profile_id`, `state_dir_from_config`, `profile_id`.
- **Types** (`profiles.rs`) — `AuthProfile`, `AuthProfileKind` (`OAuth`/`Token`), `TokenSet`, `AuthProfilesData`, `AuthProfilesStore`.
- **Ops/RPC** (`ops`, re-exported as `rpc`) — `set_credential`, `clear_credential`, `store_session`, `clear_session`, `SetCredentialRequest`, `seed_api_key_from_env`, `seed_session_from_env`, `auth_get_state`, `auth_get_session_token_json`, `store_provider_credentials`, `remove_provider_credentials`, `list_provider_credentials`, `list_provider_credentials_by_prefix`, `oauth_fetch_client_key`, `encrypt_secret`, `decrypt_secret`, `start_credential_gated_services`, `stop_credential_gated_services`.
- **Composio-direct** — `store_composio_api_key`, `get_composio_api_key`, `clear_composio_api_key`, `rpc_store_composio_api_key`.
- **Backend OAuth re-exports** (from `crate::api::rest`) — `BackendOAuthClient`, `ConnectResponse`, `IntegrationSummary`, `IntegrationTokensHandoff`, `decrypt_handoff_blob`, `user_id_from_profile_payload`.
- **Schema controllers** — `all_credentials_controller_schemas`, `all_credentials_registered_controllers`.

## RPC / controllers

Namespace `auth` (JSON-RPC `openhuman.auth_*` / CLI `openhuman-core auth <function>`). Defined in `schemas.rs`:

| Method | Description |
| --- | --- |
| `auth_set_credential` | Install the backend credential: `{ token, kind?: "session" \| "api-key" \| "local", userId?, user? }`. |
| `auth_clear_credential` | Remove the credential of one `kind`, or every kind. |
| `auth_get_state` | Current auth state (`AuthStateResponse`: `isAuthenticated`, `userId`, `user`, `profileId`, `credential`, `expiresAt`). |
| `auth_get_session_token` | Read stored app session token. |
| `auth_store_provider_credentials` | Store provider credentials for a profile. |
| `auth_remove_provider_credentials` | Remove provider credentials. |
| `auth_list_provider_credentials` | List stored provider credentials (optional provider filter). |
| `auth_oauth_fetch_client_key` | Fetch one-time client key share for an encrypted integration. |

The account-bound `auth` methods `auth_create_channel_link_token`, `auth_oauth_connect`, `auth_oauth_list_integrations`, `auth_oauth_fetch_integration_tokens` and `auth_oauth_revoke_integration` keep their wire names but are served by `openhuman-tinyhumans` (`hosted::{channel_link, oauth}`, on the TinyHumans SDK), registered only when `openhuman_tinyhumans::install` runs.

`openhuman.auth_store_session` and `openhuman.auth_clear_session` survive as legacy aliases (`core/legacy_aliases.rs`) of the credential pair for bundles that predate it.

Note: `list_provider_credentials_by_prefix` and the Composio-direct/secret helpers are public ops but not registered as `auth.*` controllers here — they are called directly by other domains.

## Agent tools

`tools.rs` — `credential_list`, `session_state` (the stored credential state and user, no token material), `oauth_connect_url`, `oauth_list` (these two dispatch the hosted `auth_oauth_*` controllers by wire name through the registry and answer `BACKEND_UNAVAILABLE:` without the hosted layer).

## Events

`bus.rs` — `SessionExpiredSubscriber` (`name() == "credentials::session_expired_handler"`, domain filter `["auth"]`) **subscribes** to `DomainEvent::SessionExpired`. On a non-local session it flips the scheduler gate to signed-out and drops the rejected credential (`clear_session`); for a local offline session or an API-key runtime it re-enables the gate and no-ops. This module does not publish events directly (publishers of `SessionExpired` are 401-detection sites elsewhere). The host learns of the sign-out through the Socket.IO `auth:session_expired` bridge and `auth.get_state`.

## Persistence

`AuthProfilesStore` (`profiles.rs`) writes `auth-profiles.json` in the config state directory (parent of `config.config_path`, user-scoped after activation). Layout: `schema_version` (current = 1), `updated_at`, `active_profiles` (provider → profile-id), `profiles` (id → profile). The `app-session` profile's metadata carries `user_id`, `user_json` (the host-supplied payload) and `session_expires_at` (the JWT `exp`, for the local expiry precheck). Secret handling:

- **OS keychain** when available (`crate::security::keyring::is_available`): all token fields stored under key `auth:{profile_id}` namespaced by a per-user id derived from the state dir; JSON keeps no secret fields.
- **Encrypted-JSON fallback** (headless/CI): token fields encrypted via `SecretStore` (ChaCha20-Poly1305).
- Loads migrate legacy `enc:`/`enc2:` cipher fields and promote secrets into the keychain; unrecoverable (un-decryptable / bad-`kind`) profiles are dropped rather than poisoning the whole store; unparseable files are quarantined to `auth-profiles.corrupt-<ts>.json` and reset to empty.
- Mutations are guarded by `auth-profiles.lock` (PID-stamped). Stale/leaked/malformed locks are reclaimed by liveness + age checks to avoid the "stuck on Initializing OpenHuman" hang.

## Dependencies

- `crate::config` — `Config`, config load (`load_config_with_timeout`), user-dir activation (`default_root_openhuman_dir`, `user_openhuman_dir`, `read/write/clear_active_user`, `pre_login_user_dir`), onboarding state.
- `crate::security::keyring` — `SecretStore` (encrypt/decrypt) and OS keychain `get`/`set`/`delete`/`is_available`.
- `crate::cron::scheduler_gate` — signed-out override flipped on install/removal/expiry.
- `crate::memory::conversations` — purge pre-login threads, bind conversation persistence after activation.
- `crate::memory` — bind memory client to the active workspace after activation.
- `crate::inference::host_runtime`, `crate::voice::{server,dictation_listener,always_on}` — credential-gated services started/stopped.
- `crate::api::config`, `::jwt`, `::rest` — backend API URL, JWT `exp` decode, `BackendOAuthClient` + OAuth/handoff types.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`), `crate::core` (`ControllerSchema`/`FieldSchema`/`TypeSchema`), `crate::core::events::DomainEvent` + `tinybus::EventHandler`, `crate::rpc::RpcOutcome` — controller registry + RPC envelope + event bus.

## Used by

Many domains consume `AuthService` / session helpers / Composio-direct key, including (paths under `crates/openhuman-core/src/`): `core/{all,auth,jsonrpc}.rs` and `core/runtime/{context,services}.rs` (controller wiring, auth gate, gated services, boot seeding, identity seeding), `api/jwt.rs`, `desktop/app_state/ops/` (snapshot), `channels/controllers/ops/*` and `channels/runtime/startup/credentials.rs` (managed credentials), `integrations/composio/{client/direct,ops/direct_mode}.rs` (BYO key), `config/ops/model.rs`, `config/migrations/unify_ai_provider_settings.rs`, `embeddings/{cloud_adapter,factory,rpc/api_keys}.rs`, `security/encryption/ops.rs`, `http_host/auth.rs`, `inference/{openai_oauth,provider}/*` (provider auth, OpenAI OAuth), `hosted/{announcements,billing,team}/ops.rs`, `flows/*`, `modules/{connectors,memory_host}.rs`, `web3/wallet/ops.rs`, and the prompt-layer identity readers (`agent/tinyagents/host/context_composer.rs`, `agent/session_host/turn/context.rs`, …).

## Notes / gotchas

- `mod.rs` re-exports `ops` both as `ops::*` and as `pub use ops as rpc` — call sites use `credentials::rpc::*`; this is the documented `rpc.rs`-equivalent exception (no separate `rpc.rs` file exists).
- `set_credential` does heavy orchestration beyond just storing a token (directory activation, thread purge, service startup). Treat it as the install funnel, not a thin setter — except on the same-token/same-user refresh path, which only rewrites the stored payload.
- An embedder host (`CoreContext::current_embedder_config()` is set) keeps the credential under its own `config_path` scope and never touches the operator's global `active_user.toml`.
- Local offline sessions are detected purely by the JWT signature segment being literally `local` (`is_local_session_token`); they are never sent anywhere and are never treated as expired.
- Secrets are never logged — debug lines record only lengths/markers, honoring the CLAUDE.md redaction rule.
