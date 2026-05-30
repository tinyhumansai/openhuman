# tls

Platform-conditional TLS backend selection for `reqwest` HTTP clients. A single tiny utility that centralises the `#[cfg(target_os = "windows")]` guard so every HTTP-client construction site picks the right TLS backend in one line, and future policy changes live in exactly one place.

## Responsibilities

- Provide one canonical `reqwest::ClientBuilder` factory pre-configured with the platform-appropriate TLS backend.
- Encode the cross-platform TLS policy:
  - **Windows** → `native-tls` (schannel). Honors the Windows certificate store, including corporate / AV / TLS-inspecting-proxy CAs. `rustls` + webpki-roots only knows Mozilla CAs and fails such environments with `UnknownIssuer`.
  - **macOS / Linux** → `rustls` + webpki-roots. Avoids the OpenSSL runtime dependency on Linux; historically more reliable on macOS staging TLS handshakes than `native-tls`.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/tls/mod.rs` | Entire module. Module docstring (policy) + the single `tls_client_builder()` function. No `mod`/`pub mod` decls; no submodules. |

## Public surface

- `tls_client_builder() -> reqwest::ClientBuilder` — returns a `reqwest::Client::builder()` with `.use_native_tls()` on Windows and `.use_rustls_tls()` elsewhere, selected at compile time via `cfg`. Intended as the starting point for any client reaching external HTTPS endpoints; callers chain `.timeout(...)`, `.http1_only()`, proxy config, etc. and then `.build()`.

## RPC / controllers

None. Not RPC-facing — no `schemas.rs`, no controllers.

## Agent tools

None.

## Events

None.

## Persistence

None — stateless, pure factory.

## Dependencies

- External crate `reqwest` (and its `native-tls` / `rustls` feature backends) only. No `use crate::openhuman::*` or `use crate::core::*` imports — this is a leaf utility module with zero internal dependencies.

## Used by

Every HTTP-client construction site that talks to external HTTPS endpoints, including:

- `src/openhuman/config/schema/proxy.rs` — proxy-aware client builders (primary + fallback).
- `src/openhuman/integrations/client.rs` and integration tools (`tools/searxng.rs`, `tools/brave.rs`, `tools/querit.rs`, `tools/seltz.rs`).
- `src/openhuman/inference/provider/compatible.rs` — OpenAI-compatible inference provider clients.
- `src/openhuman/composio/client.rs`, `src/openhuman/app_state/ops.rs`.
- `src/api/rest.rs` (REST API client).

Registered in the domain tree via `pub mod tls;` in `src/openhuman/mod.rs`.

## Notes / gotchas

- Backend choice is **compile-time** (`cfg(target_os = ...)`), not runtime — you cannot switch backends at runtime without recompiling for the target.
- Always start from `tls_client_builder()` rather than `reqwest::Client::builder()` directly, otherwise Windows corporate-CA environments will fail with `UnknownIssuer`.
- The doctest in the docstring is `rust,ignore`.
