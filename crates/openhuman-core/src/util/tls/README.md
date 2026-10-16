# tls

Platform-conditional TLS backend selection for `reqwest` HTTP clients. A single tiny utility that centralizes the `#[cfg(target_os = "windows")]` guard so every HTTP-client construction site picks the right TLS backend in one line, and future policy changes live in exactly one place.

## Responsibilities

- Provide one canonical `reqwest::ClientBuilder` factory pre-configured with the platform-appropriate TLS backend.
- Parse a user-provided PEM CA bundle and attach its roots to a provider-scoped HTTP client.
- Encode the cross-platform TLS policy:
  - Windows uses `native-tls` (schannel). It honors the Windows certificate store, including corporate, AV, or TLS-inspecting-proxy CAs. `rustls` plus webpki-roots only knows Mozilla CAs and fails such environments with `UnknownIssuer`.
  - macOS and Linux use `rustls` with Mozilla and native roots. Corporate CAs installed in the OS trust store or supplied through `SSL_CERT_FILE` are accepted without an OpenSSL runtime dependency on Linux.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/util/tls/mod.rs` | TLS policy, CA bundle parsing, and client builders. |
| `crates/openhuman-core/src/util/tls/mod_tests.rs` | TLS builder and CA validation tests. |

## Public surface

- `tls_client_builder() -> reqwest::ClientBuilder`: returns a `reqwest::Client::builder()` with `.use_native_tls()` on Windows and `.use_rustls_tls()` elsewhere, selected at compile time via `cfg`. It is the starting point for any client reaching external HTTPS endpoints; callers chain `.timeout(...)`, `.http1_only()`, proxy config, and so on, then `.build()`.
- `parse_ca_bundle(pem)` rejects oversized, malformed, or private-key-bearing data and returns the CA certificates.
- `client_with_ca_bundle` and `client_with_ca_bundle_with_timeouts` add those certificates alongside the platform roots and apply the configured proxy policy.

No RPC surface, agent tools, bus events, or persistence. It is a stateless factory function.

## Dependencies

`reqwest` supplies the TLS backends and certificate parser. The provider-scoped builders also use the core's runtime proxy policy.

## Used by

Every HTTP-client construction site that talks to external HTTPS endpoints, including:

- `crates/openhuman-core/src/config/schema/proxy.rs`: proxy-aware client builders (primary and fallback).
- `crates/openhuman-core/src/integrations/client/construct.rs` and `crates/openhuman-core/src/integrations/composio/client/connections.rs`.
- `crates/openhuman-core/src/search/tools/*.rs` (`tavily`, `exa`, `brave`, `searxng`, `querit`, `seltz`): search-tool HTTP clients.
- `crates/openhuman-core/src/desktop/app_state/ops/current_user_fetch.rs`.
- `crates/openhuman-tinyhumans/src/backend/headers.rs` (REST API client profiles for the TinyHumans backend).

Declared via `pub mod tls;` in `crates/openhuman-core/src/util/mod.rs`; not re-exported at the `util` root, so callers spell `crate::util::tls::tls_client_builder`.

## Notes and gotchas

- Backend choice is compile-time (`cfg(target_os = ...)`), not runtime. You cannot switch backends at runtime without recompiling for the target.
- Always start from `tls_client_builder()` rather than `reqwest::Client::builder()` directly, otherwise Windows corporate-CA environments will fail with `UnknownIssuer`.
- The doctest in the docstring is `rust,ignore`.

## Further reading

- [Parent module (`util`)](../README.md)
- [Architecture overview](../../../../../docs/gitbooks/en/developing/architecture.md)
- [Testing strategy](../../../../../docs/gitbooks/en/developing/testing-strategy.md)
