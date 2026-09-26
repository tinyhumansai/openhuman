# API

The core's side of the TinyHumans / AlphaHuman hosted backend: URL
resolution, session-token retrieval, product attribution, the authenticated
REST client, error classification, and the Socket.IO handshake URL.

The core has **no dependency on `tinyhumans-sdk`**. It reaches the backend
only through the port in `transport/` (`BackendTransport`); the SDK-backed
implementation lives in `crates/openhuman-tinyhumans`, which hosts install
once per process. A core with no transport installed runs agents, memory,
tools and RPC as normal and answers every backend-touching call with the
typed `BackendApiError::BackendUnavailable` / `BACKEND_UNAVAILABLE:`
sentinel, which `core::observability` demotes.

Routes are named here (and in the domains that call `authed_json`); the SDK
still owns route *policy* (its unexposed-route registry) inside the transport.
Add a missing backend route to `vendor/tinyhumans-sdk` so the policy tables
know it, then name it from the core as usual.

## Layout

| File | Purpose |
| --- | --- |
| `transport/` | `BackendTransport` port, `BackendRequest`, `BackendTransportError`, process-global install/resolve; `plain.rs` is the `cfg(test)`-only reqwest fallback; production has no fallback — a host installs the transport from `openhuman-tinyhumans` |
| `headers.rs` | Attribution headers (`x-core-version`, `x-tauri-version`, `x-sdk-name`) and the per-`TransportProfile` `reqwest::ClientBuilder` every transport implementation builds from |
| `classify.rs` | `is_budget_exhausted_message` — backend/provider budget-exhaustion body classification shared by inference, agent loop guards, scheduler, web chat and telemetry |
| `config.rs` | Backend/inference URL resolution and local-vs-hosted classification |
| `jwt.rs` | Session-token load, JWT payload/`exp` reading and `Authorization` header formatting |
| `product.rs` | `x-sdk-name` product-attribution header |
| `rest.rs` | `BackendOAuthClient`, typed `BackendApiError`, and the error-classification chokepoint |
| `rest_tests.rs` | Tests for `rest.rs` (included via `#[path]`) |
| `socket.rs` | Socket.IO (Engine.IO v4) WebSocket URL construction |
| `models/` | Serde DTOs shared across auth and realtime call sites |

## `transport/`

`BackendTransport` is the one HTTP primitive every hosted-backend call rides:

```rust
async fn send_json(&self, req: BackendRequest<'_>) -> Result<Value, BackendTransportError>;
async fn send_multipart(&self, req: BackendRequest<'_>, form: Form) -> Result<Value, BackendTransportError>;
fn http_client(&self, profile: TransportProfile) -> reqwest::Client;
```

`BackendRequest` carries the profile (`Api` for control-plane REST,
`Integrations` for `/agent-integrations/*`), base URL, method, path, query,
JSON body, the `BackendCredential` (session JWT → `Authorization: Bearer`,
API key → `x-api-key`) and whether the `{success,data}` envelope is unwrapped.
`BackendTransportError` mirrors the variants the classifiers match on
(`Http`, `Status`, `Envelope`, `RouteNotExposed`, …) plus `Unavailable`.

Resolution (`resolve_backend_transport`), first hit wins: the transport bound
to the ambient `CoreContext` (`CoreBuilder::backend_transport`, inherited by
`derive_with`) → the process global (`install_backend_transport`, what the
desktop shell, TUI and CLI use because they boot the core through
`run_server_embedded_with_ready` / `run_core_from_args`) → under `cfg(test)`
only, `PlainHttpTransport` → `Err(Unavailable)`. Production has no implicit
fallback.

## `config.rs`

Single source of truth for every URL the app uses to reach the hosted backend
or an LLM inference endpoint. Two families are resolved separately because a
`config.api_url` pointed at a local model runner (Ollama, vLLM, LM Studio)
only speaks `/v1/chat/completions` and 404s on every other path:

- `effective_api_url` — chat/inference base: non-empty `config.api_url` →
  `BACKEND_URL`/`VITE_BACKEND_URL` runtime env → the same keys baked in via
  `option_env!` → environment default. `effective_inference_url` returns an
  explicit `inference_url` override verbatim, otherwise joins
  `OPENHUMAN_INFERENCE_PATH` (`/openai/v1/chat/completions`) onto
  `effective_api_url`.
- `effective_backend_api_url` — base for all control-plane calls (auth,
  billing, team, integrations, voice, sockets, …). Skips the user's
  `api_url` override when it `looks_like_local_ai_endpoint`,
  `looks_like_inference_provider_endpoint`, or resolves to a builtin cloud
  provider host (`config::schema::cloud_providers`) and is not the OpenHuman
  backend itself, so pointing `api_url` at Ollama or `openrouter.ai` doesn't
  also misroute `/teams/me/usage` and billing calls there. Falls through the
  same env/default chain, passed through `normalize_backend_api_base_url`
  (`pub(crate)`) which strips an inference-style path from a misconfigured
  `BACKEND_URL`.
- `normalize_api_base_url` — trims whitespace and trailing slashes only; it
  is a cheap string operation with no URL parsing. `api_url(base, path)` is
  the matching join helper.
- `DEFAULT_API_BASE_URL` (`https://api.tinyhumans.ai`) /
  `DEFAULT_STAGING_API_BASE_URL` (`https://staging-api.tinyhumans.ai`),
  chosen by `default_api_base_url_for_env` from `app_env_from_env`, which
  reads `OPENHUMAN_APP_ENV` / `VITE_OPENHUMAN_APP_ENV` (`APP_ENV_VAR` /
  `VITE_APP_ENV_VAR`) at runtime, then compile time. `api_base_from_env` is
  the separate `BACKEND_URL` / `VITE_BACKEND_URL` lookup; both check each key
  independently so an empty primary never shadows the secondary.
- `looks_like_local_ai_endpoint` / `looks_like_inference_provider_endpoint` —
  heuristics documented in-file; both are intentionally tight to avoid
  misclassifying real custom backends or ephemeral test mock servers.

## `jwt.rs`

`get_session_token` is a re-export of
`crate::security::credentials::session_support::get_session_token` (with
`APP_SESSION_PROVIDER` and `DEFAULT_AUTH_PROFILE_NAME`), so callers keep one
import path for "where the token lives". Token *parsing* and header
*formatting* — `bearer_authorization_value`, `decode_jwt_payload`,
`decode_jwt_exp_unix` — are implemented here (they are pure and the
credentials store needs them on a core with no backend at all; the SDK keeps
its own identical copy for hosts). `decode_jwt_exp` wraps the Unix-seconds
`exp` decoder in the `chrono` type the credentials store uses, so an expired
token can be rejected locally instead of round-tripping to a guaranteed 401.
None of them verify the signature; the backend stays the authority.

## `product.rs`

`ProductIdentity` / `set_product_identity` attach a sanitized `x-sdk-name`
header (`PRODUCT_IDENTITY_HEADER`) to every backend-bound request, so the
backend can attribute calls to OpenHuman or OpenCompany even though
both share one login and reach the backend through this crate.
`ProductIdentity::new` keeps ASCII alphanumerics plus `.`, `_`, `-`,
lower-cases, truncates to 64 bytes, and returns `None` when nothing survives,
so the wrapped value can never break `HeaderValue` construction. The
identity is process-wide (a `OnceLock<RwLock<ProductIdentity>>`), not a
constructor parameter, because `BackendOAuthClient` is built at dozens of
call sites across domains. **Call `set_product_identity` once at startup,
before building any backend client** — `BackendOAuthClient` and
`IntegrationClient` bake the identity into their default headers at
construction and do not pick up a later change. A build that never calls
the setter sends `DEFAULT_PRODUCT_IDENTITY` (`"openhuman"`).

Tests across `api::product`, `api::rest`, and `integrations` all
touch this process-global state; `product_identity_test_lock` (test-only)
serializes them to avoid cross-module races.

## `rest.rs`

`BackendOAuthClient` holds the backend origin (base URL stripped to its
origin) and sends every request through the process `BackendTransport`
(`TransportProfile::Api`: `x-core-version`, optional `x-tauri-version`, and
`x-sdk-name` default headers; platform TLS via `util::tls`; 120 s timeout —
all specified by `headers.rs`). Key surface:

- `authed_json` — send an authenticated request and route the result through
  `finish_authed_json`.
- Typed route helpers (`fetch_client_key`, `send_channel_*`,
  `*_channel_thread`) all go through `authed_json` and are bearer-only: the
  core never obtains, exchanges or validates a session. The account-bound
  routes (link tokens, `/auth/me` link checks, OAuth connect / integrations,
  billing, team, webhook tunnels, announcements) are called by
  `openhuman-tinyhumans` (`hosted/`) on the TinyHumans SDK's typed clients.
- `url_for`, `raw_client` — URL helpers for callers that need to drive a
  non-JSON request (e.g. multipart uploads) without re-implementing TLS/proxy
  setup. `raw_client` returns the transport's `Api`-profile client and fails
  with `BackendUnavailable` when no transport is installed.
- `user_id_from_profile_payload` — pull the user id out of the `/auth/me`
  envelope variants.
- `decrypt_handoff_blob` — AES-256-GCM decrypt for integration token handoff,
  compatible with the backend's `encryptMessageFromString`.

`BackendApiError` is the typed-error surface `authed_json` callers should
match on for expected backend states rather than treating as failures:
`Unauthorized` (401 — session lapsed, not a bug), `MessageNotFound` (404 on a
channel message the provider or backend already deleted),
`ChannelEditUnsupported` (404 because the backend never implemented the
`PATCH` edit route), `BackendUnavailable` (no transport installed).
`flatten_authed_error` maps `Unauthorized` onto the `SESSION_EXPIRED`
JSON-RPC sentinel so the dispatcher classifies it as session expiry instead of
reporting it to Sentry, and `BackendUnavailable` onto `BACKEND_UNAVAILABLE:`
(`core::observability::BACKEND_UNAVAILABLE_PREFIX`) for the same reason.

The private `BackendOAuthClient::finish_authed_json` is the error
classification chokepoint for every `authed_json`/`fetch_billing_summary`
call: it walks the `reqwest`/`hyper`/`rustls` error source chain (not just the
top-level message) to distinguish a transient transport failure from one
worth reporting, and turns specific status/path combinations into the typed
`BackendApiError` variants above. `IntegrationClient::map_transport_error`
(`integrations/client/errors.rs`) plays the same role for integrations.
Route new backend calls through those helpers instead of matching
`BackendTransportError` by hand.

## `socket.rs`

`websocket_url` converts an `http(s)` API base into the Engine.IO v4
WebSocket URL (`wss://…/socket.io/?EIO=4&transport=websocket`) the realtime
client connects to.

## `models/`

Serde DTOs (`auth.rs`, `socket.rs`) shared by auth and realtime call sites —
see [`models/mod.rs`](models/mod.rs) for the full list.

## Backend request rules (from `AGENTS.md`)

- Add missing backend routes to `vendor/tinyhumans-sdk`, not to
  `crates/openhuman-core/src/api/`.
- Every TinyHumans backend request must carry a sanitized `x-sdk-name`:
  `BackendOAuthClient`, `IntegrationClient` (except redirected file
  downloads), the agent's Langfuse ingestion request, and — outside this crate — the host
  session owner's `POST /auth/login-token/consume` / `GET /auth/me`
  (`openhuman_tinyhumans::session`, via `ClientHeaders`).
- Never add `x-sdk-name` to third-party endpoints, MCP servers, BYOK
  inference endpoints, or presigned storage redirects.
- When auditing hand-built backend requests, grep for
  `bearer_authorization_value` and `header(AUTHORIZATION`.

## Called by

74 files outside `api/` reference `crate::api::`
(`grep -rl 'crate::api::' crates/openhuman-core/src`). Heaviest consumers:
`platform/socket/` (realtime client), `hosted/*` (billing, referral,
announcements, team), `integrations/` and
`integrations/composio/`, `channels/controllers/ops/` and
`channels/bus/{delivery,progressive_ui}.rs` (which match on `BackendApiError` variants),
`security/credentials/` (defines the `get_session_token` that `jwt.rs`
re-exports; consumes `BackendOAuthClient`, `flatten_authed_error`,
`decode_jwt_exp`, and re-exports the OAuth types), `voice/`,
`agent/progress_tracing/` (Langfuse ingestion URL and `x-sdk-name`), and
`inference/provider/`, `embeddings/`, `inference/voice/` (backend
inference proxy and cloud transcription/embeddings).

## Tests

`rest_tests.rs` covers `BackendOAuthClient::new` base stripping, the
`x-core-version` / `x-tauri-version` / `x-sdk-name` default headers on both
the SDK and `raw_client` paths, `authed_json` 401/404 classification into
`BackendApiError` (including the route-absence vs message-gone split for
channel edits), `flatten_authed_error`, `user_id_from_*_payload`,
`sanitize_client_version`, `backend_api_body_shape`, and
`decrypt_handoff_blob`. Transient-transport classification is not unit
tested here; it relies on
`core::observability::contains_transient_transport_phrase`. `config.rs`,
`jwt.rs`, `product.rs`, and `socket.rs` carry their own `#[cfg(test)]`
modules.
