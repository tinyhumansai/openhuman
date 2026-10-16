# client

`IntegrationClient`: the shared HTTP client every integration domain uses to
talk to the OpenHuman backend's `/agent-integrations/*` routes. The managed
tools, file storage, media and web3 all build one of these rather than making
their own backend calls, so auth, egress policy, error classification, and
budget gating live in one place instead of being copied per domain.

## Key files

| File | Role |
| --- | --- |
| [`construct.rs`](./construct.rs) | The `IntegrationClient` type and its constructors. Holds the backend URL, the app-session auth token, a separate `reqwest::Client` for downloads, and a lazily-fetched pricing cache. Also sanitizes a misconfigured `backend_url` that carries an inference-style path (issue #2075). |
| [`requests.rs`](./requests.rs) | Outbound request plumbing: route guards (refusing intentionally unexposed routes such as `webhooks`/`admin`), egress disclosure and enforcement, budget gating, and the JSON verb methods (`post`/`get`/`patch`/`delete`/`upload_multipart`). |
| [`download.rs`](./download.rs) | Raw-bytes download support. The file-storage download route 302-redirects to a presigned S3 URL; `get_bytes` reads `Content-Type` and `Content-Disposition` off the response since the JSON envelope helpers do not carry them. |
| [`budget_gate.rs`](./budget_gate.rs) | Refuses `/agent-integrations/*` calls once the account's AI credits are exhausted, so a tool call never burns a backend round trip the backend would reject anyway. Reads `GET /teams/me/usage` through `crate::client::BackendClient` with a one-minute failure backoff to avoid flooding Sentry on a persistent outage. |
| [`pricing.rs`](./pricing.rs) | The pricing cache on `IntegrationClient` plus `pricing_for_config`, which short-circuits to empty pricing in Composio direct mode (no backend session exists to serve `/agent-integrations/pricing` there). |
| [`errors.rs`](./errors.rs) | Backend error classification: extracting a readable detail from an error body, and handling the session-JWT 401 to session-expiry recovery path. |

## How auth works

Every request carries the app-session JWT as its `Authorization: Bearer`,
resolved through `resolve_backend_credential`, the same credential
billing, team, webhooks, and memory all use. A `401` from an
`/agent-integrations/*` route is therefore unambiguous: it means that JWT
has expired, been revoked, or been rotated server-side, never a
third-party integration's own auth failure. `errors.rs` maps that case to
session-expiry recovery.

## How it fits

JSON traffic rides `crate::client::IntegrationsHttp` over the shared SDK
transport; `IntegrationClient` does not open its own connections for anything
but downloads (which share `IntegrationsHttp`'s two clients). Domain code
(`file_storage`, `tools`, `media`, `web3`, `composio`) builds one client per call
site through `crate::integrations::build_client` and calls its verb methods
directly rather than reaching for `BackendClient` or a raw `reqwest` call.

## Where to look next

See [`../README.md`](../README.md) for the integrations domain overview.
`errors.rs`'s `map_transport_error` is where a raw `BackendTransportError`
becomes the `anyhow::Error` every verb method returns.

## Further reading

- [Third-party integrations](../../../../../docs/gitbooks/en/features/integrations/README.md)
- [Connections](../../../../../docs/gitbooks/en/features/connections.md)
