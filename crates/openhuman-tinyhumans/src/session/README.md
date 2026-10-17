# session

TinyHumans login and backend-session ownership for OpenHuman hosts. The core
only holds and uses a backend credential (a session JWT, a TinyHumans API key,
or the offline local token). It never obtains, validates, exchanges or
refreshes one. Everything that talks to the backend's `/auth/*` endpoints
lives here instead, shared by the hosts that drive a user login: the Tauri
shell ([`crates/openhuman-app/src/session/`](../../../openhuman-app/src/session/), talking to its core over HTTP
JSON-RPC) and the TUI ([`crates/openhuman-tui/src/session.rs`](../../../openhuman-tui/src/session.rs), holding an
in-process `CoreRuntime`). This module absorbed the former standalone
`openhuman-session` crate.

## How it works

New session credentials carry the normalized backend origin used for exchange
and validation as `issuingBackend`. Core stores this non-secret association in
the existing profile metadata beside the encrypted JWT. Both core token release
and host profile refresh refuse a bound session when the configured backend
differs, before sending its credential. Restore the issuing backend or sign in
again; a mismatch does not erase the stored session. API keys and local sessions
retain their existing behavior. Legacy profiles without an association remain
compatible and unbound because their issuing origin cannot be inferred.

Browser hosts pass their captured origin to `login_with_token_for_backend` or
`store_session_token_for_backend`. These reject a changed backend before any
credential-bearing request and retain the accepted client through exchange,
validation and persistence, so configuration changes cannot redirect a callback.

Four pieces stack up. `SessionManager` is the one hosts drive; it composes the
other three.

```text
            host (Tauri auth_* commands, TUI)
                          |
                          v
                   SessionManager<L>          login, store, logout,
                    |      |      |           current user, state, events
         +----------+      |      +-------------+
         v                 v                    v
   SessionClient     CurrentUserCache        CoreLink (trait L)
   POST /auth/        last /auth/me,         invoke(method, params)
   login-token/       TTL + stale-while-       |
   consume,           revalidate + backoff     v
   GET /auth/me                              the host's core:
   (tinyhumans-sdk)                          auth_set_credential
                                             auth_clear_credential
                                             auth_get_state
                                             auth_get_session_token
                                             config_resolve_api_url
```

The core is the store of record. The manager keeps no copy of the secret: it
reads the stored token back through the link when it needs to refresh
`/auth/me`, and pushes credentials in through `auth.set_credential`.

### Login

`login_with_token(login_token)` exchanges a one-time login token for a JWT
(`SessionClient::consume_login_token`, `POST /auth/login-token/consume`) and
then calls `store_session_token(jwt, None)`. Storing runs under a mutation
lock so two login or logout callbacks cannot interleave, and decides:

| Case                                                    | Result                                                                                    |
| ------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| local offline token                                     | stored as-is; a non-empty `user` payload is required                                      |
| JWT whose `exp` has passed                              | `SessionError::Expired`, nothing stored                                                   |
| `/auth/me` confirms the JWT                             | pushed to the core with the backend's user; cache seeded                                  |
| `/auth/me` rejects the JWT                              | `SessionError::Rejected`, nothing stored                                                  |
| backend unreachable, JWT has a live `exp` and a user id | stored provisionally with `pendingBackendValidation: true`; a background loop revalidates |
| backend unreachable, no live `exp`                      | `SessionError::Transient`, nothing stored                                                 |

Store-time validation (`SessionClient::validate_for_store`) bounds
`/auth/me` by a budget (12 s by default, `OPENHUMAN_AUTH_ME_TIMEOUT_MS`, with
`OPENHUMAN_AUTH_ME_STORE_TIMEOUT_MS` as the legacy name) and retries once
after a transient failure. The revalidation loop backs off from 5 s to 60 s,
stops as soon as the core no longer holds that token, and re-checks under the
mutation lock before acting so a newer login or a logout is never touched by
a stale verdict. The new credential is pushed to the core before the old
loop is cancelled, so a failed push leaves the previous pending credential
still revalidating.

`store_api_key(key)` stores a TinyHumans API key with no backend round trip
and no user identity. `logout()` clears the session credential in the core,
stops any revalidation, forgets the cache and clears `identity`;
`clear_api_key()` clears the stored API key.

### Current user

`current_user(force)` serves `/auth/me` for a session credential through
`CurrentUserCache`, and the stored payload for a local session or an API key.
The cache is keyed on `(base_url, secret)` so one identity's freshness or
outage is never reported as another's. Within `REFRESH_TTL` (5 s) it serves
the cached answer; after that it serves stale while revalidating. When the
backend is unreachable it records a negative entry with a bounded backoff
(capped at `BACKOFF_MAX`, 60 s) so a dead backend is not re-paid the fetch
timeout on every poll. The fetch timeout defaults to 5 s, is clamped to 2 to
12 s, and is set with `OPENHUMAN_AUTH_FETCH_TIMEOUT_SECS`. A backend rejection
clears the credential and returns `SessionError::Rejected`; an availability
failure returns the stored user marked stale.

`state()` combines the core's view (`auth_get_state`) and the current user
into a `SessionState`. Every change is broadcast as a `SessionEvent`
(`Changed` or `Expired`) to `subscribe()` receivers, and the user id is
mirrored into `identity` for synchronous readers.

### Which backend

`SessionManager::client()` asks the core which backend it is configured
against (`config_resolve_api_url` through the link) and rebuilds its
`SessionClient` when that base URL changes, for example on an environment
switch. `SessionClient` normalizes the base to its origin and builds its
HTTP client through the core's `util::tls::tls_client_builder`, with the
attribution headers from `ClientHeaders` (`x-sdk-name`, plus
`x-core-version` and `x-tauri-version` when the host sets them).

## Layout

| Path                                 | What it does                                                                                                                                                                                                            |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`mod.rs`](mod.rs)                   | Module declarations and re-exports.                                                                                                                                                                                     |
| [`manager.rs`](manager.rs)           | `SessionManager`, `SessionState`, `SessionEvent`, `SessionError`; the login, store, logout, revalidation and current-user flows.                                                                                        |
| [`client.rs`](client.rs)             | `SessionClient` (login-token exchange, `GET /auth/me`, store-time validation), `ClientHeaders`, `SessionClientError`, `FetchMeError`, transient status and phrase classification.                                       |
| [`cache.rs`](cache.rs)               | `CurrentUserCache`, `CachedUser`, TTL, backoff and fetch-timeout policy.                                                                                                                                                |
| [`link.rs`](link.rs)                 | `CoreLink`, `CoreAuthState`, the RPC method-name constants, and helpers (`push_credential`, `clear_credential`, `core_auth_state`, `core_session_token`, `resolve_backend_url`, `unwrap_envelope`).                     |
| [`credential.rs`](credential.rs)     | `Credential` and `CredentialKind` (`Session`, `ApiKey`, `Local`), `Credential::classify`, and the JWT and profile helpers (`decode_jwt_exp`, `jwt_is_live`, `user_id_from_jwt_claims`, `user_id_from_profile_payload`). |
| [`identity.rs`](identity.rs)         | Process-global user id (`set_user_id`, `peek_user_id`, `clear`) for Sentry `before_send` hooks that cannot await. Only the id is kept, never a token or profile.                                                        |
| [`test_support.rs`](test_support.rs) | Axum stub backend and stub `CoreLink` for this module's tests (test builds only).                                                                                                                                       |

## Key types and entry points

- `SessionManager::new(link, headers)` returns an `Arc<SessionManager<L>>`.
  Hosts call `login_with_token`, `store_session_token`, `store_api_key`,
  `logout`, `clear_api_key`, `current_user`, `state` and `subscribe`.
- `CoreLink` is a one-method async trait, `invoke(method, params)`. The
  desktop shell implements it over loopback HTTP JSON-RPC; the TUI over its
  in-process runtime.
- `ClientHeaders::new(sdk_name)` with `with_core_version` and
  `with_tauri_version` sets the attribution headers for auth calls.
- `Credential::classify(token)` tells a local offline token from a JWT.

## Boundaries

- This module may use core utilities (`util::tls`) and this crate's own
  `backend::product`, but it never calls `openhuman_embed::__host::security::*` and
  never dispatches into a core except through `CoreLink`. The host owns the
  login; the core only takes the resulting credential.
- `link::unwrap_envelope` copies the walk `openhuman_rpc::unwrap_rpc` does
  rather than importing it, so this crate does not depend on
  `openhuman-rpc`.
- The core's side of the handoff (`auth.set_credential` and friends) lives in
  [`crates/openhuman-core/src/security/credentials/`](../../../openhuman-core/src/security/credentials/).
- Library and headless hosts that authenticate with an API key do not need
  this module; see
  [`docs/gitbooks/en/developing/tinyhumans-api-key.md`](../../../../docs/gitbooks/en/developing/tinyhumans-api-key.md).
  Embedders use `openhuman_embed::Auth`.

## Gotchas

- Never log a token or the `/auth/me` payload. Logs carry base URLs, user ids
  and error classes only.
- The mutation lock covers login and logout side effects; the revalidation
  loop takes it again before applying a verdict. Keep that order when adding
  a new mutating flow.

## Tests

[`manager_tests.rs`](manager_tests.rs), [`client_tests.rs`](client_tests.rs), [`cache_tests.rs`](cache_tests.rs), [`link_tests.rs`](link_tests.rs) and
[`credential_tests.rs`](credential_tests.rs) sit beside their modules and drive the flows against
the axum stub backend and stub `CoreLink` in [`test_support.rs`](test_support.rs).

```bash
cargo test -p openhuman-tinyhumans session::
```

## Further reading

- [`docs/gitbooks/en/developing/tinyhumans-api-key.md`](../../../../docs/gitbooks/en/developing/tinyhumans-api-key.md): running on a TinyHumans API key.
- [`docs/gitbooks/en/developing/architecture/security.md`](../../../../docs/gitbooks/en/developing/architecture/security.md): security.
- [`crates/openhuman-tinyhumans/README.md`](../../README.md): the openhuman-tinyhumans crate README.
