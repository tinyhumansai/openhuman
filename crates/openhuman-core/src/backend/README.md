# backend

This folder is the core's side of talking to a hosted backend it otherwise
knows nothing about. It defines the transport port every backend call rides,
the authenticated JSON client (`BackendClient`) that domains use to call
routes, and the typed errors those calls return. Domains such as channels,
voice, memory billing, media generation and the integrations budget gate call
into it; hosts install the actual transport.

The core has no dependency on `tinyhumans-sdk` and holds no hosted URL,
default, environment variable, header policy or product identity of its own.
All of that belongs to the installed transport, and the helpers in [`mod.rs`](./mod.rs)
ask it. The only production transport is `SdkBackendTransport` in
[`crates/openhuman-tinyhumans`](../../../openhuman-tinyhumans/), built on the vendored SDK.

## How it works

A backend call goes through three layers. The domain names a route and a
credential, `BackendClient` checks the endpoint, sends the request through the
resolved transport and classifies the result, and the transport does the HTTP
round trip with the SDK's route policy.

```text
domain code (channels, voice, memory billing, ...)
    |  BackendClient::from_config(&cfg)?.authed_json(cred, method, path, body)
    v
BackendClient                                  backend/client.rs
    |  endpoint checks (HTTPS or loopback; API key only to managed host)
    |  resolve_backend_transport()
    v
dyn BackendTransport                           backend/transport/
    |  send_json(BackendRequest { profile, base_url, path, credential, .. })
    v
SdkBackendTransport (crates/openhuman-tinyhumans)
    |  TLS, timeouts, attribution headers, route policy, wire classification
    v
hosted backend
    |
    v  Result<Value, BackendTransportError>
BackendClient::finish_authed_json  ->  Value  or  BackendApiError / anyhow
    |
    v  (on the JSON-RPC String error channel)
flatten_authed_error  ->  SESSION_EXPIRED: / API_KEY_REJECTED: /
                          BACKEND_UNAVAILABLE: / full error chain
```

### Finding a transport

`transport::install::resolve_backend_transport` picks the transport for the
current call. The first hit wins:

1. The transport bound to the ambient `CoreContext`
   (`CoreBuilder::backend_transport`, inherited by `CoreContext::derive_with`).
2. The process global set by `install_backend_transport`. The desktop shell,
   TUI and CLI use this path because they boot the core through
   `openhuman_rpc::host` or `run_core_from_args` rather than the
   builder. `openhuman_tinyhumans::install` (or `RuntimeBuilder` for library
   hosts) does the install.
3. Under `cfg(test)` only, `PlainHttpTransport` from [`transport/plain.rs`](./transport/plain.rs).
4. Otherwise `BackendTransportError::Unavailable`.

Production has no implicit fallback. A core nobody gave a transport runs
agents, memory, tools and RPC as normal, and every backend-touching call
answers with a typed "backend unavailable" error. `is_installed()` lets paths
that reach the backend host directly (the Langfuse proxy push, the socket,
channel reply delivery, browser-task module config) skip quietly when there is
no transport.

### Base URLs and attribution

`mod.rs` has thin helpers that ask the transport for host-specific values:

| Helper | Returns |
| --- | --- |
| `base_url(&config.api_url)` | The control-plane origin (account, integrations, channels, voice, sockets, telemetry) for `BaseUrlPurpose::ControlPlane`. `Unavailable` without a transport. |
| `require_base_url` | The same, on the `String` error channel: a missing transport becomes the `BACKEND_UNAVAILABLE:` sentinel. |
| `inference_base_url` | The managed OpenAI-compatible inference origin (`BaseUrlPurpose::Inference`), which honours an `api_url` that points at an inference endpoint. Without a transport it still returns an explicitly configured override (normalized by `util::url::normalize_api_base_url`), because managed inference talks to that URL directly. |
| `product_identity()` | The `x-sdk-name` value the host attributes traffic to, or `None`. |
| `attribution_headers()` | The host's full attribution header set (`x-sdk-name`, client versions), or an empty map. |

The two purposes differ only in how an operator override is treated: a
control-plane call must never land on an inference endpoint the user pointed
`api_url` at, while managed inference uses it as given. The transport owns
the defaults, the `BACKEND_URL` / `VITE_BACKEND_URL` overrides and that guard
([`crates/openhuman-tinyhumans/src/backend/url.rs`](../../../openhuman-tinyhumans/src/backend/url.rs)).

### Sending an authenticated request

`BackendClient::new` parses the base URL, requires an absolute `http(s)` URL
with a host, and strips any path, query and fragment so `Url::join` resolves
root-relative routes correctly even when a caller passes a full completions
URL. `BackendClient::from_config` resolves the base through `base_url` and
fails with `BackendApiError::BackendUnavailable` when no transport is
installed.

`authed_json(credential, method, path, body)` accepts a `BackendCredential`
(or a bare session token). Before sending it refuses two unsafe cases: an API
key may only go to the managed host (`api.tinyhumans.ai`,
`staging-api.tinyhumans.ai`) or a loopback endpoint, and any credential
requires HTTPS or loopback HTTP. These checks live in
`inference::provider::openhuman_backend_model`. It then sends a
`BackendRequest` with `TransportProfile::Api` and `unwrap_envelope: true`.
The transport puts a session JWT on `Authorization: Bearer` and an API key on
`x-api-key` (`transport::credential_headers`).

### Classifying the result

The private `finish_authed_json` is the classification chokepoint for every
`authed_json` call:

| Transport result | Outcome |
| --- | --- |
| `Ok(value)` | Returned as is. |
| `Unavailable` | `BackendApiError::BackendUnavailable`. |
| `Http(reqwest::Error)` | Walks the whole `reqwest`/`hyper`/`rustls` source chain. A transient transport phrase (`core::observability::contains_transient_transport_phrase`) is logged as a warning; anything else goes to `report_error`. |
| `ChannelMessageRouteMissing` | `BackendApiError::ChannelEditUnsupported`. |
| `ChannelMessageNotFound` | `BackendApiError::MessageNotFound`. |
| `Status { 401, .. }` | `ApiKeyRejected` for an API-key credential, `Unauthorized` for a session. Not reported to Sentry. |
| `Status { 400, budget body }` | Logged at info as a budget exhaustion, not reported. |
| `Status { transient code }` | Logged as a warning (`is_transient_http_status_code`), not reported. |
| other `Status` | Reported with method, path, host, status and a PII-safe `body_shape`, then returned as an error. |
| any other error | Returned with `backend request <METHOD> <path>` context. |

`body_shape` (`backend_api_body_shape`) never echoes response values. For a
JSON object it records the key count and only the keys that look like short
ASCII identifiers, so an email or UUID used as a key is counted as redacted.

The core never reads a response body to decide what a 404 means. The
transport classifies the two channel-message 404s (`tinyhumans_sdk::classify`,
applied by `openhuman-tinyhumans`'s `map_sdk_error`), and the core decides
the recovery.

### Flattening for JSON-RPC

`flatten_authed_error` turns an `authed_json` error into the string that
crosses the JSON-RPC boundary, keyed off the typed downcast rather than the
display text:

- `Unauthorized` becomes `SESSION_EXPIRED: ...`, which the dispatcher treats
  as session expiry: no Sentry report, and `DomainEvent::SessionExpired` is
  published so the host can drive re-sign-in.
- `ApiKeyRejected` becomes `API_KEY_REJECTED: ...`
  (`core::observability::API_KEY_REJECTED_PREFIX`). It is kept apart from
  session expiry because a library runtime authenticating with an API key has
  no session to clear.
- `BackendUnavailable` becomes `BACKEND_UNAVAILABLE: ...`
  (`core::observability::BACKEND_UNAVAILABLE_PREFIX`), which observability
  demotes.
- Everything else keeps its full `{err:#}` chain so real failures still reach
  Sentry.

## Layout

| Path | What it does |
| --- | --- |
| `mod.rs` | Re-exports, plus `base_url`, `require_base_url`, `inference_base_url`, `product_identity` and `attribution_headers`. No local URL, header or identity state. |
| [`client.rs`](./client.rs) | `BackendClient`, `BackendApiError`, `flatten_authed_error`, endpoint checks and `finish_authed_json` classification. |
| [`client/channel.rs`](./client/channel.rs) | Channel routes on `BackendClient`: `send_channel_message`, `send_channel_typing`, `send_channel_edit`, `send_channel_delete`, `send_channel_reaction`, `create_channel_thread`, `update_channel_thread`, `list_channel_threads`. |
| [`classify.rs`](./classify.rs) | `is_budget_exhausted_message`, a one-line wrapper over `tinyinference_providers::is_budget_exhausted_message`. |
| [`transport/`](transport/README.md) | The port: `BackendTransport`, `BackendRequest`, `TransportProfile`, `BaseUrlPurpose`, `BackendTransportError`, the install slot, and shared helpers (`credential_headers`, `parse_body_text`, `unwrap_envelope`, `compose_url`). |
| `transport/plain.rs` | `PlainHttpTransport`, compiled only under `cfg(test)` so the core's wiremock tests need no host crate. It applies no route policy. |

## Key types and entry points

- `BackendTransport` ([`transport/mod.rs`](./transport/mod.rs)) is the trait a transport
  implements: `send_json`, `send_multipart`, `http_client(profile)`,
  `base_url(configured, purpose)`, `product_identity`,
  `attribution_headers` and `name`. Implementations are process-wide
  `Arc<dyn BackendTransport>` singletons and must be safe to call
  concurrently.
- `BackendRequest` (`transport/mod.rs`) fully describes one round trip:
  profile (`Api` for control-plane REST, `Integrations` for
  `/agent-integrations/*`), base URL, method, path, query pairs, JSON body,
  optional credential, and whether to unwrap the `{success, data}` envelope.
- `BackendTransportError` ([`transport/error.rs`](./transport/error.rs)) mirrors the SDK error arms
  the classifiers match on: `Unavailable`, `Url`, `Http`, `Status`,
  `ChannelMessageNotFound`, `ChannelMessageRouteMissing`, `Envelope`,
  `Header`, `Decode`, `RouteNotExposed`, `Other`.
- `install_backend_transport`, `resolve_backend_transport`, `is_installed`
  ([`transport/install.rs`](./transport/install.rs)) manage the process slot.
- `BackendClient` (`client.rs`) is the client domains hold. Besides
  `authed_json` it has `url_for(path)` and `raw_client()`, which returns the
  transport's `Api`-profile `reqwest::Client` (attribution headers, no
  credential) for non-JSON shapes such as multipart STT uploads.
- `BackendApiError` (`client.rs`) is what callers match on for expected
  states: `Unauthorized`, `ApiKeyRejected`, `MessageNotFound`,
  `ChannelEditUnsupported`, `BackendUnavailable`.

## Boundaries

- Route policy, TLS, timeouts, redirects, attribution headers, URL defaults
  and wire classification belong to the transport implementation in
  `crates/openhuman-tinyhumans` (`src/transport/`, `src/backend/url.rs`,
  `src/backend/headers.rs`, `src/backend/product.rs`).
- Backend routes are defined in [`vendor/tinyhumans-sdk`](../../../../vendor/tinyhumans-sdk/). Add a missing route
  there (its unexposed-route registry is the policy the transport enforces)
  and then name it from the core. Do not recreate route implementations in
  this folder.
- Account-bound hosted routes (link tokens, OAuth connect and handoff,
  billing, team, webhook tunnels, announcements) are called from
  [`crates/openhuman-tinyhumans/src/hosted/`](../../../openhuman-tinyhumans/src/hosted/) on the SDK's typed clients, not
  through `BackendClient`. The integration token handoff decrypt lives in
  [`crates/openhuman-tinyhumans/src/hosted/oauth/handoff.rs`](../../../openhuman-tinyhumans/src/hosted/oauth/handoff.rs).
- Session-token lookup, JWT parsing and `Authorization` formatting live in
  `security::credentials::jwt`. The core never obtains, exchanges or validates
  a session.
- The Socket.IO handshake URL builder is `platform::socket::url::websocket_url`.
- `IntegrationClient::map_transport_error`
  ([`integrations/client/errors.rs`](../integrations/client/errors.rs)) does the same classification job for
  `/agent-integrations/*` traffic.
- Hosted memory does not go through this port. The TinyMemory engine uses its
  own HTTP client and only takes `base_url`, `attribution_headers` and a
  credential source from the core.

## Gotchas

- Every TinyHumans backend request must carry a sanitized `x-sdk-name`, which
  the transport stamps. That covers `BackendClient`, `IntegrationClient`
  (except redirected file downloads), the Langfuse and OTLP ingestion push,
  the managed model catalog listing, and the host session owner's
  `POST /auth/login-token/consume` and `GET /auth/me`. Never add it to
  third-party endpoints, MCP servers, BYOK inference endpoints or presigned
  storage redirects.
- Route new backend calls through `authed_json` (or
  `IntegrationClient::map_transport_error`) instead of matching
  `BackendTransportError` by hand, or 401s and transient failures will page
  Sentry.
- `ChannelEditUnsupported` and `MessageNotFound` mean different things. A
  missing edit route (every `PATCH` edit today, #5230) says the message still
  exists, so callers must keep its id and only disable editing. Forgetting
  the id there leaves streaming drafts undeleted in the chat.
- Under `cfg(test)` the "global" transport slot is per thread, so a test that
  installs a fake transport must run on a single thread to see it.
- When auditing hand-built backend requests, grep for
  `bearer_authorization_value` and `header(AUTHORIZATION`.

## Tests

[`client_tests.rs`](./client_tests.rs) covers base stripping in `BackendClient::new`, 401
classification, `flatten_authed_error` and `backend_api_body_shape`.
[`client_channel_tests.rs`](./client_channel_tests.rs) covers channel route shapes and, with a stub
transport, the mapping of the typed channel-message 404s. [`mod_tests.rs`](./mod_tests.rs) and
[`transport/transport_tests.rs`](./transport/transport_tests.rs) cover the URL helpers and the port. The 404
wire classification is tested where it lives
([`vendor/tinyhumans-sdk/tests/classify.rs`](../../../../vendor/tinyhumans-sdk/tests/classify.rs),
[`crates/openhuman-tinyhumans/src/transport/channel_404_tests.rs`](../../../openhuman-tinyhumans/src/transport/channel_404_tests.rs)), and the
attribution header tests are in
[`crates/openhuman-tinyhumans/src/backend/headers_tests.rs`](../../../openhuman-tinyhumans/src/backend/headers_tests.rs) and
[`crates/openhuman-tinyhumans/src/transport/transport_tests.rs`](../../../openhuman-tinyhumans/src/transport/transport_tests.rs).

Run with `cargo test -p openhuman backend::` or `pnpm debug rust backend::`.

## Further reading

- [Parent module README](../../README.md)
- [One TinyHumans API key](../../../../docs/gitbooks/en/developing/tinyhumans-api-key.md)
- [Deep architecture reference](../../../../docs/gitbooks/en/developing/architecture.md)
- [openhuman-tinyhumans crate](../../../openhuman-tinyhumans/README.md)
- [tinyhumans-sdk](../../../../vendor/tinyhumans-sdk/README.md)
