# credentials

Host credential custody for the backend credential the core authenticates with and for provider/OAuth auth profiles. Owns the encrypted auth-profile document, the `app-session` / `api-key` credential slots and everything the core does when one is installed or removed, per-provider token storage (e.g. API keys, OAuth token sets), and read-side helpers for backend OAuth credentials. Composio direct-mode (BYO key) credentials are owned by `integrations::composio`. The `auth.*` JSON-RPC / CLI names remain stable, and this module runs the canonical sign-out teardown when a `SessionExpired` event fires.

**The core never obtains, validates, exchanges or refreshes a credential.** Login-token exchange, `GET /auth/me` validation and the current-user cache are the host's job: the Tauri shell and the TUI through `openhuman_tinyhumans::session`, an embedder through `openhuman_embed::Auth`, an operator through the CLI or the boot env vars. They hand the result over with `auth.set_credential`.

## Responsibilities

- Install a backend credential (`set_credential`): a session JWT (with the user id / payload the host resolved, or the JWT's subject claim), a TinyHumans API key, or the offline local token. A JWT whose `exp` is already past is refused; a session with no resolvable user id is refused.
- On install: activate the user-scoped openhuman directory, purge pre-login (anonymous) conversation threads on first activation, bind memory/conversation persistence, start credential-gated services, open the scheduler gate, scope Sentry and the prompt-layer identity. A repeat install with the **same token and user** is a cheap refresh of the stored user payload (this is how a host replaces a `pendingBackendValidation` placeholder with its later `/auth/me` answer); a **different user** is signed out first.
- On removal / session-expiry (`clear_credential`): remove the profile, clear the active-user marker, stop the gated services, rebind process globals to the signed-out workspace, and close the scheduler-gate override.
- Boot seeding for headless hosts: `OPENHUMAN_BACKEND_API_KEY` / `OPENHUMAN_BACKEND_SESSION_TOKEN` install a credential on a fresh store (never overwrite one).
- Persist arbitrary provider credentials (token + metadata fields) as named auth profiles; list/remove/set-active; prefix-list profiles for grouped namespaces (e.g. `channel:*`).
- Run backend OAuth flows (bearer-only): connect URL, list integrations, fetch integration handoff tokens, fetch one-time client key, revoke integration; mint channel link tokens.
- Encrypt/decrypt arbitrary secrets via the `SecretStore`.
- Manage the profile document with TSD `EncryptedFileSecrets`, profile-secret keyring slots when available, corrupt-legacy-document quarantine, and TSD local leases.

## Key files

| File                                         | Role                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| -------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`mod.rs`](./mod.rs)                         | Exposes an explicit credential API, schema controllers (`all_credentials_controller_schemas` / `all_credentials_registered_controllers`), and `jwt::user_id_from_profile_payload`.                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| [`core.rs`](./core.rs)                       | `AuthService` facade over `AuthProfilesStore`: store/get/remove/set-active profiles, resolve bearer token, profile-id selection logic (override → active → default → any-for-provider), provider normalization, state-dir derivation.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| [`profiles.rs`](./profiles.rs)               | The persistence engine. `AuthProfile` / `TokenSet` / `AuthProfileKind` / `AuthProfilesData` types and `AuthProfilesStore`: TSD-encrypted document persistence, optional keyring secret slots, one-time import of the legacy JSON document, corrupt-document quarantine, and local lease coordination.                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| [`api_key.rs`](./api_key.rs)                 | The `api-key` profile: `store_api_key[_in]`, `get_api_key[_in]`, `has_api_key[_in]`, `clear_api_key`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| [`ops.rs`](./ops.rs) + `ops/`                | Business logic + RPC entry points (returns `RpcOutcome<T>`). Credential operations are split into `ops/credential/{request,set,clear,teardown}.rs`; other modules cover gated services, env seeding, auth-state queries, provider credentials and secrets. Historical `store_session` / `clear_session` names remain available.                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| [`schemas.rs`](./schemas.rs)                 | `auth.*` controller schemas + `handle_*` dispatchers delegating to `ops`. Defines `all_controller_schemas` / `all_registered_controllers`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| [`session_state.rs`](./session_state.rs) | Session kind, profile metadata, JWT claims, expiration classification, auth state responses, and session profile reads. |
| [`backend_credential.rs`](./backend_credential.rs) | Resolves a usable session or API-key credential for backend transports and publishes locally detected expiry events. |
| [`identity.rs`](./identity.rs)               | The signed-in user's identity slot (`peek_credential_user_identity`), seeded from the host-supplied payload for prompts and Sentry.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| [`responses.rs`](./responses.rs)             | Response DTOs: `AuthStateResponse`, `AuthProfileSummary`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| [`bus.rs`](./bus.rs)                         | `SessionExpiredSubscriber`: `EventHandler` for `DomainEvent::SessionExpired`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| [`tools.rs`](./tools.rs)                     | Agent tools `credential_list`, `session_state`, `oauth_connect_url`, `oauth_list` (these two dispatch the hosted `auth_oauth_*` controllers by wire name through the registry and answer `BACKEND_UNAVAILABLE:` without the hosted layer).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `*_tests.rs`                                 | Sibling test suites (`#[path = ...]`).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |

## Public surface

- **`AuthService`** ([`core.rs`](./core.rs)): `from_config`, `new`, `load_profiles`, `store_provider_token`, `set_active_profile`, `remove_profile`, `get_profile`, `get_provider_bearer_token`.
- **Constants**: `APP_SESSION_PROVIDER` (`"app-session"`), `DEFAULT_AUTH_PROFILE_NAME` (`"default"`), `api_key::API_KEY_PROVIDER` (`"api-key"`).
- **Credential kinds**: `session_state::CredentialKind {Session, ApiKey, Local}` (wire values `"session"`, `"api-key"`, `"local"`). `backend_credential::resolve_backend_credential` returns `BackendCredential::ApiKey` when a key is stored (before any session classification) and `BackendCredential::Session` otherwise; `BackendClient::authed_json` sends an API key as `x-api-key` and a session as `Authorization: Bearer`. TinyHumans managed-inference and cloud-embedding host adapters resolve the bearer from the API key first, then the stored app session. Every backend caller goes through it (integrations, voice, channels, webhooks, hosted proxies); bearer-only seams (connector proxy route, memory-host `SessionBearer`, the Socket.IO handshake) use `backend_bearer_secret`, which returns the key first and the stored session otherwise: the backend accepts a key as `Authorization: Bearer` by its `tiny_` prefix. `has_backend_credential` is the boot-time "signed in?" question the scheduler gate asks; `SessionExpired` is ignored for an API-key runtime and for a local session. A local session also has no managed bearer: `resolve_backend_credential` answers `LOCAL_SESSION_BACKEND_UNAVAILABLE` and the managed-inference host resolver answers `LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE`, so neither hosted REST nor managed inference sends a token the backend will only 401 (#6932).
- **Helpers**: `normalize_provider`, `default_profile_id`, `select_profile_id`, `state_dir_from_config`, `profile_id`.
- **Types** ([`profiles.rs`](./profiles.rs)): `AuthProfile`, `AuthProfileKind` (`OAuth`/`Token`), `TokenSet`, `AuthProfilesData`, `AuthProfilesStore`.
- **Ops/RPC** ([`ops`](./ops)): `set_credential`, `clear_credential`, `store_session`, `clear_session`, `SetCredentialRequest`, `seed_api_key_from_env`, `seed_session_from_env`, `auth_get_state`, `auth_get_session_token_json`, `store_provider_credentials`, `remove_provider_credentials`, `list_provider_credentials`, `list_provider_credentials_by_prefix`, `encrypt_secret`, `decrypt_secret`, `start_credential_gated_services`, `stop_credential_gated_services`.
- **Backend re-exports**: `jwt::user_id_from_profile_payload`. `BackendClient` lives in `crates/openhuman-tinyhumans/src/client/`. The OAuth connect/handoff types (`IntegrationSummary`, `IntegrationTokensHandoff`) and `decrypt_handoff_blob` moved to `crates/openhuman-tinyhumans/src/hosted/oauth/` with the OAuth flows themselves.
- **Schema controllers**: `all_credentials_controller_schemas`, `all_credentials_registered_controllers`.

## RPC / controllers

Namespace `auth` (JSON-RPC `openhuman.auth_*` / CLI `openhuman-core auth <function>`). Defined in [`schemas.rs`](./schemas.rs):

| Method                             | Description                                                                                                            |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `auth_set_credential`              | Install the backend credential: `{ token, kind?: "session" \| "api-key" \| "local", userId?, user? }`.                 |
| `auth_clear_credential`            | Remove the credential of one `kind`, or every kind.                                                                    |
| `auth_get_state`                   | Current auth state (`AuthStateResponse`: `isAuthenticated`, `userId`, `user`, `profileId`, `credential`, `expiresAt`). |
| `auth_get_session_token`           | Read stored app session token.                                                                                         |
| `auth_store_provider_credentials`  | Store provider credentials for a profile.                                                                              |
| `auth_remove_provider_credentials` | Remove provider credentials.                                                                                           |
| `auth_list_provider_credentials`   | List stored provider credentials (optional provider filter).                                                           |

The account-bound `auth` methods `auth_create_channel_link_token`, `auth_oauth_connect`, `auth_oauth_list_integrations`, `auth_oauth_fetch_integration_tokens`, `auth_oauth_revoke_integration` and `auth_oauth_fetch_client_key` keep their wire names but are served by `openhuman-tinyhumans` (`hosted::{channel_link, oauth}`, on the TinyHumans SDK), registered only when `openhuman_tinyhumans::install` runs.

`openhuman.auth_store_session` and `openhuman.auth_clear_session` survive as legacy aliases (`core/legacy_aliases.rs`) of the credential pair for bundles that predate it.

Note: `list_provider_credentials_by_prefix` and secret helpers are public ops but not registered as `auth.*` controllers here. They are called directly by other domains. Composio direct-mode credential helpers live with `integrations::composio`.

## Agent tools

[`tools.rs`](./tools.rs): `credential_list`, `session_state` (the stored credential state and user, no token material), `oauth_connect_url`, `oauth_list` (these two dispatch the hosted `auth_oauth_*` controllers by wire name through the registry and answer `BACKEND_UNAVAILABLE:` without the hosted layer).

## Events

[`bus.rs`](./bus.rs): `SessionExpiredSubscriber` (`name() == "credentials::session_expired_handler"`, domain filter `["auth"]`) **subscribes** to `DomainEvent::SessionExpired`. On a non-local session it flips the scheduler gate to signed-out and drops the rejected credential (`clear_session`); for a local offline session or an API-key runtime it re-enables the gate and no-ops. This module does not publish events directly (publishers of `SessionExpired` are 401-detection sites elsewhere). The host learns of the sign-out through the Socket.IO `auth:session_expired` bridge and `auth.get_state`.

## Persistence

New hosted sessions may include `issuingBackend` on `auth_set_credential`, also
returned by `auth_get_state`. Its canonical HTTP(S) origin is stored as the
non-secret `session_issuing_backend` profile metadata, beside the encrypted JWT.
`get_session_token` and `resolve_backend_credential` refuse to release a bound
JWT for a different configured backend (`SESSION_BACKEND_MISMATCH`). Same-token
refresh preserves the association; it cannot rebind the token. Legacy profiles
without this field remain unbound because the original backend cannot be
inferred. API-key precedence and local-session behavior are unchanged.

`AuthProfilesStore` (`profiles.rs`) writes the schema document to `auth-profiles/secrets.enc` in the config state directory (parent of `config.config_path`, user-scoped after activation). The document contains `schema_version` (current = 1), `updated_at`, `active_profiles` (provider → profile-id), and [`profiles`](./profiles) (id → profile). The old root-level `auth-profiles.json` is read only for one-time import. The `app-session` profile's metadata carries `user_id`, `user_json` (the host-supplied payload) and `session_expires_at` (the JWT `exp`, for the local expiry precheck). Secret handling:

- Config-backed profile secrets use TinySecurity `KeyringAccess2` over TSD `KeyringSecrets`, with fresh approval for each read, write, and delete. No-Config bootstrap APIs keep secrets only in the encrypted profile document.
- The whole local document is encrypted by `EncryptedFileSecrets`. Legacy `enc:` fields are unsupported and unrecoverable profiles are dropped; malformed legacy JSON is quarantined during import.
- Reads and mutations coordinate through `tinystoragedrivers-fence::LocalLeases`; doctor reads open the encrypted document read-only.

## Dependencies

- `crate::config`: `Config`, config load (`load_config_with_timeout`), user-dir activation (`default_root_openhuman_dir`, `user_openhuman_dir`, `read/write/clear_active_user`, `pre_login_user_dir`), onboarding state.
- `crate::modules::security`: TinySecurity `KeyringAccess2` client and exact-operation consent callback.
- `tinystoragedrivers_secrets::EncryptedFileSecrets`: profile-local encrypted document and read-only doctor access.
- `crate::platform::llm_gate`: signed-out override flipped on install/removal/expiry.
- `crate::memory::conversations`: purge pre-login threads, bind conversation persistence after activation.
- `crate::memory`: bind memory client to the active workspace after activation.
- `crate::inference::host_runtime`: the local runtime probe is reset on logout.
- [`jwt.rs`](./jwt.rs): JWT `exp` decode and `user_id_from_profile_payload`.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`), `crate::core` (`ControllerSchema`/`FieldSchema`/`TypeSchema`), `crate::core::events::DomainEvent` + `tinybus::EventHandler`, `crate::rpc::RpcOutcome`: controller registry + RPC envelope + event bus.

## Used by

Many domains consume `AuthService` / session helpers, including (paths under `crates/openhuman-core/src/`): `core/{all,auth,jsonrpc}.rs` and `core/runtime/{context,services}.rs` (controller wiring, auth gate, gated services, boot seeding, identity seeding), `desktop/app_state/ops/` (snapshot), `channels/controllers/ops/*` and `channels/runtime/startup/credentials.rs` (managed credentials), `integrations/composio/{client/direct,ops/direct_mode}.rs` (BYO key, owned by the integration), `config/ops/model.rs`, `config/migrations/unify_ai_provider_settings.rs`, `embeddings/{factory,rpc/api_keys}.rs`, `security/encryption/ops.rs`, `inference/{openai_oauth,provider}/*` (provider auth, OpenAI OAuth), `flows/*`, `modules/{connectors,memory_host}.rs`, `web3/wallet/ops.rs`, and the prompt-layer identity readers (`agent/tinyagents/host/context_composer.rs`, `agent/session_host/turn/context.rs`, …). Outside this crate, `crates/openhuman-tinyhumans/src/hosted/{announcements,billing,team}/ops.rs` and `crates/openhuman-rpc/src/http_host/auth.rs` consume it too.

## Notes / gotchas

- [`mod.rs`](./mod.rs) exposes an explicit set of credential APIs. Controller operations live in `ops`; session state and backend transport resolution live in separate modules.
- `set_credential` does heavy orchestration beyond just storing a token (directory activation, thread purge, service startup). Treat it as the install funnel, not a thin setter, except on the same-token/same-user refresh path, which only rewrites the stored payload.
- An embedder host (`CoreContext::current_embedder_config()` is set) keeps the credential under its own `config_path` scope and never touches the operator's global `active_user.toml`.
- Local offline sessions are detected purely by the JWT signature segment being literally `local` (`is_local_session_token`); they are never sent anywhere and are never treated as expired.
- Secrets are never logged; debug lines record only lengths/markers, honoring the CLAUDE.md redaction rule.

## Further reading

- [Security](../security/README.md)
- [OS keyring and secret storage](../../../../../docs/gitbooks/en/features/os-keyring-and-secret-storage.md)
- [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)

`channel_credential_provider` owns the persisted `channel:<id>:<mode>` namespace.
Managed login and channel setup use it through the host facade so existing
credential profiles remain readable across module upgrades.
