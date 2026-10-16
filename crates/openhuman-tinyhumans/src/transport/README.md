# transport

This module owns the hosted-backend HTTP port (`port.rs`: `BackendTransport`,
`BackendRequest`, `BackendTransportError`, `TransportProfile`, `BaseUrlPurpose`)
and `SdkBackendTransport`, its implementation. Every hosted-backend request
made by `BackendClient`, `IntegrationClient` and the other hosted callers in
this crate passes through it and rides the vendored `tinyhumans-sdk`'s raw HTTP
primitive. The SDK owns route policy (its unexposed-route registry) and the
credential header shapes the backend expects; this module wires the SDK to the
port's request type and translates the SDK's errors into the port's. The core
has no part in it.

## How it works

`SdkBackendTransport::new()` builds two `reqwest::Client`s, one per
`TransportProfile` (`Api` and `Integrations`), with
`crate::backend::headers::build_backend_client`. That gives every request the
platform TLS backend, the profile's timeout (120 s or 60 s), disabled
redirects and the attribution headers (`x-core-version`, `x-tauri-version`,
`x-sdk-name`) as defaults.

For each request the caller hands over a `BackendRequest` (method, path, base
URL, optional query and body, the resolved credential, the profile and whether
to unwrap the `{success, data}` envelope). `send_json` then:

1. builds a `TinyHumansClient` for `req.base_url`, giving it the profile's
   `reqwest::Client` and the product-identity headers as SDK default headers
   (so `x-sdk-name` holds even for a client this crate did not build);
2. attaches the credential: `BackendCredential::Session` as a bearer token
   (`with_token`), `BackendCredential::ApiKey` as `x-api-key`
   (`with_api_key`), or nothing;
3. calls `sdk.raw().send(method, path, query, body, unwrap_envelope)`;
4. maps an error through `map_sdk_error(error, &method, path)`.

`send_multipart` does the same through `raw().post_multipart` for uploads
such as speech-to-text.

The transport also answers configuration questions for this crate's callers and
its config enricher, so the core keeps no hosted URL or header policy of its own:

| Trait method | Answer |
| --- | --- |
| `base_url(configured, ControlPlane)` | `backend::url::effective_backend_api_url` |
| `base_url(configured, Inference)` | `backend::url::effective_api_url` |
| `product_identity()` | `backend::product::product_identity()` |
| `attribution_headers()` | `backend::headers::attribution_headers()`, built fresh each call; on a malformed header value it degrades to the identity header alone |
| `http_client(profile)` | a clone of the profile's `reqwest::Client`, for callers that drive `reqwest` themselves |
| `name()` | `"tinyhumans-sdk"` |

### Error mapping

`map_sdk_error` in [`error.rs`](error.rs) maps each `tinyhumans_sdk::Error` variant onto a
`BackendTransportError` variant one for one (`Url`, `Http`, `Status`,
`Header`, `Decode`, `RouteNotExposed`, `Envelope`, with `Other` as a
catch-all), so the classifiers in `client/classify.rs` and the
integrations client see the same shapes they would see calling the SDK
directly.

One case gets a typed variant here because only this backend's wire behavior
says what it means: a 404 on a channel-message path
(`.../channels/<provider>/messages/<id>`).

```text
 404 on .../channels/<p>/messages/<id>
   |
   +-- method is PATCH and body contains "Cannot PATCH "
   |     (Express unmatched-route page: the backend has no edit route)
   |       -> ChannelMessageRouteMissing { provider, message_id }
   |
   +-- otherwise (handler's JSON 404: the message is gone)
           -> ChannelMessageNotFound { provider, message_id }
```

The client recovers from the two differently (`HostedApiError` variants in
`client/error.rs`) and never reads the response body itself.

## Layout

| Path | What it does |
| --- | --- |
| [`port.rs`](port.rs) | The port: `BackendTransport`, `BackendRequest`, `BackendTransportError`, `TransportProfile`, `BaseUrlPurpose`, `QueryParam`. |
| [`mod.rs`](mod.rs) | `SdkBackendTransport` and its `BackendTransport` impl. |
| [`error.rs`](error.rs) | `map_sdk_error`, plus the channel-message path parser and the unmatched-route check. |

## Key types and entry points

- `SdkBackendTransport::new()` builds the transport; it fails only when a
  `reqwest::Client` cannot be built.
- `SdkBackendTransport::shared()` returns it as `Arc<dyn BackendTransport>`.
- `crate::client::shared_transport()` is the process-wide instance every hosted
  caller uses.
- `map_sdk_error` is public (re-exported at the crate root) for code that
  calls the SDK and needs the same translation.

Most hosts never construct the transport by hand: `crate::install()` and
`RuntimeBuilder::build()` build the shared one. The core holds no transport. A
core with this layer not connected still runs agents, memory, tools and RPC,
and answers backend-touching calls with `HostedApiError::BackendUnavailable` /
`BACKEND_UNAVAILABLE:`.

## Boundaries

- The port lives here in [`port.rs`](port.rs); the core has no copy.
  Credential resolution is the core's generic one
  (`security::credentials::session_support`).
- Routes live in the vendored SDK ([`vendor/tinyhumans-sdk`](../../../../vendor/tinyhumans-sdk/),
  `tinyhumansai/tinyhumans-sdk`). A missing backend route goes into the SDK's
  route registry and is named from this crate; this transport never grows a
  route implementation of its own. A route the SDK does not expose fails with
  `RouteNotExposed`.
- The hosted RPC proxies in [`../hosted/`](../hosted/README.md) do not call
  this trait. They build their own typed `TinyHumansClient` through
  `hosted::client::HostedClient`, reusing this transport's `http_client`.
- The SDK's Socket.IO client is compiled out (`default-features = false`);
  realtime stays on the core's `platform::socket`.

## Tests

[`transport_tests.rs`](transport_tests.rs) pins the wire shape against a wiremock backend: bearer
versus `x-api-key`, attribution headers, `Status` and `Envelope` mapping, and
SDK route refusal. [`channel_404_tests.rs`](channel_404_tests.rs) pins the two channel-message 404
cases.

```bash
cargo test -p openhuman-tinyhumans transport::
```

## Further reading

- [`docs/gitbooks/en/developing/tinyhumans-api-key.md`](../../../../docs/gitbooks/en/developing/tinyhumans-api-key.md): running on a TinyHumans API key.
- [`docs/gitbooks/en/developing/architecture.md`](../../../../docs/gitbooks/en/developing/architecture.md): architecture overview.
- [`crates/openhuman-tinyhumans/README.md`](../../README.md): the openhuman-tinyhumans crate README.
- [`vendor/tinyhumans-sdk/README.md`](../../../../vendor/tinyhumans-sdk/README.md): tinyhumans-sdk.
