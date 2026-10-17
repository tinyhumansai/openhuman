# Transport

The port the core uses to reach the hosted TinyHumans backend. It defines the
one HTTP primitive every backend-bound call rides, but carries no
implementation of its own. The core owns routes and error classification; an
implementation of this trait owns TLS, timeouts, redirects, and the SDK's
route policy.

This split is what keeps `tinyhumans-sdk` out of the core. A core built
without a transport still runs agents, memory, tools, and RPC. Every call
that would touch the backend degrades to `BackendTransportError::Unavailable`
(`BACKEND_UNAVAILABLE:` at the RPC boundary) instead of failing to compile or
panicking.

## Layout

| File | Purpose |
| --- | --- |
| [`mod.rs`](./mod.rs) | `BackendTransport` trait, `BackendRequest`, `TransportProfile`, `credential_headers`, `parse_body_text`, `unwrap_envelope`, `compose_url` |
| [`error.rs`](./error.rs) | `BackendTransportError`, the core-owned mirror of the variants classifiers match on |
| [`install.rs`](./install.rs) | Process-global install/resolve for the transport, plus the `CoreContext`-scoped lookup |
| [`plain.rs`](./plain.rs) | `cfg(test)`-only reqwest transport, so this crate's tests do not need a host crate |
| [`transport_tests.rs`](./transport_tests.rs) | Tests for `mod.rs` (included via `#[path]`) |

## Key types

`BackendTransport` is the trait a transport implements:

```rust
async fn send_json(&self, req: BackendRequest<'_>) -> Result<Value, BackendTransportError>;
async fn send_multipart(&self, req: BackendRequest<'_>, form: Form) -> Result<Value, BackendTransportError>;
fn http_client(&self, profile: TransportProfile) -> reqwest::Client;
fn name(&self) -> &'static str;
```

`BackendRequest` fully describes one round trip: the `TransportProfile`
(`Api` for control-plane REST, `Integrations` for `/agent-integrations/*`),
base URL, method, path, query, JSON body, an optional `BackendCredential`
(session JWT rides as `Authorization: Bearer`, an API key as `x-api-key`),
and whether the `{success, data}` envelope should be unwrapped.
`BackendTransportError` covers `Unavailable`, `Url`, `Http` (wraps the
`reqwest::Error` so classifiers can still walk its source chain), `Status`,
`Envelope`, `Header`, `Decode`, `RouteNotExposed`, and `Other`.

Implementations are process-wide singletons (`Arc<dyn BackendTransport>`)
and must be safe to call concurrently.

## Resolution

`resolve_backend_transport` tries, in order: the transport bound to the
ambient `CoreContext` (set via `CoreBuilder::backend_transport`, inherited by
`derive_with`), then the process global set by `install_backend_transport`
(what the desktop shell, TUI, and CLI use, since they boot the core through
`openhuman_rpc::host` / `run_core_from_args` rather than the
builder), then, under `cfg(test)` only, the `plain.rs` fallback. Outside
tests, a miss returns `Err(BackendTransportError::Unavailable)`. There is no
implicit production fallback.

## How it fits

Everything else under `crates/openhuman-core/src/api/` (routes in `rest.rs`,
attribution headers in `headers.rs`, URL resolution in `config.rs`) calls
through this trait rather than building `reqwest` requests directly. The
only production implementation is `SdkBackendTransport` in
[`crates/openhuman-tinyhumans`](../../../../openhuman-tinyhumans/), built on top of the vendored `tinyhumans-sdk`,
and installed once per process by `openhuman_tinyhumans::install` (or
`RuntimeBuilder` for library hosts, or `CoreBuilder::backend_transport`).

## Where next

Read `crates/openhuman-core/src/api/README.md` for the routes and error
classification that sit above this port, and `crates/openhuman-tinyhumans/`
for the real transport and how it maps SDK errors onto
`BackendTransportError`.

## Further reading

- [Parent module README](../README.md)
- [One TinyHumans API key](../../../../../docs/gitbooks/en/developing/tinyhumans-api-key.md)
- [openhuman-tinyhumans crate](../../../../openhuman-tinyhumans/README.md)
- [tinyhumans-sdk](../../../../../vendor/tinyhumans-sdk/README.md)
