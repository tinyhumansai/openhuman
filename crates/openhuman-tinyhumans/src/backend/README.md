# backend

Everything host-specific about reaching the TinyHumans backend: where it is,
how requests are attributed, and which product they are attributed to. The
core holds none of this and has no backend module. This crate's config
enricher ([`../client/enricher.rs`](../client/enricher.rs), registered with
`config::register_enricher`) publishes the answers on `Config.resolved_endpoints`
for the core to read, and `SdkBackendTransport`
(in [`../transport/`](../transport/README.md)) answers from the three files here.

## How it works

```text
 asked for                         answered from
 -------------------------------   ------------------------------------
 control-plane base URL          -> url::effective_backend_api_url
 inference base URL              -> url::effective_api_url
 attribution headers             -> headers::attribution_headers
 product identity                -> product::product_identity
 (client construction)           -> headers::build_backend_client
```

### Base URLs (`url.rs`)

`config.api_url` does two jobs: it is the hosted backend base, and it can also
be a bring-your-own inference base (Ollama, vLLM, LM Studio, OpenRouter, and
so on). Those servers only speak chat completions and answer 400 or 404 on
`/auth/*`, `/teams/*`, `/agent-integrations/*` and every other control-plane
path. So this file resolves two URL families separately.

`effective_api_url` is the inference base. It resolves in this order:

1. a non-empty `config.api_url`, normalized;
2. `BACKEND_URL`, then `VITE_BACKEND_URL`, from the runtime environment (each
   checked on its own, so an empty `BACKEND_URL` does not hide a valid
   `VITE_BACKEND_URL`);
3. the same two keys baked in at compile time with `option_env!`, so a
   shipped installer resolves the right environment with no shell variables;
4. `DEFAULT_STAGING_API_BASE_URL` (`https://staging-api.tinyhumans.ai`) when
   the app environment is staging, else `DEFAULT_API_BASE_URL`
   (`https://api.tinyhumans.ai`).

`effective_inference_url` uses an explicit `inference_url` as-is when one is
set, and otherwise joins `OPENHUMAN_INFERENCE_PATH`
(`/openai/v1/chat/completions`) onto `effective_api_url`.

`effective_backend_api_url` is the control-plane base used for auth, billing,
team, referral, webhooks, channels, voice, sockets and integrations. It starts
from the same `config.api_url`, but skips it and falls through to steps 2 to 4
when the URL looks like an inference endpoint, unless it also looks like an
OpenHuman backend. Three detectors feed that decision:

- `looks_like_local_ai_endpoint`: a path ending in `/v1/chat/completions`
  or `/v1/completions` on any host, or a loopback or private host with one of
  the well-known runner ports (11434, 8000, 8080, 1234, 8888) or a `/v1` path.
  A bare loopback URL with no path does not match, so test mock backends on
  ephemeral ports keep working;
- `looks_like_inference_provider_endpoint`: a host in the curated
  `INFERENCE_PROVIDER_DOMAINS` list (or a subdomain of one), or a path that is
  exactly `/v1` or `/api/v1`;
- the core's built-in cloud provider host list
  (`config::schema::cloud_providers::host_is_builtin_cloud_provider`).

The first fallback logs one `warn!` (guarded by `std::sync::Once`) so the
redirect is visible without repeating on every request. URLs in logs go
through `redact_url_for_log`.

### Attribution headers and clients (`headers.rs`)

Every request that reaches the hosted backend carries three attribution
headers: `x-core-version` (this crate's `CARGO_PKG_VERSION`),
`x-tauri-version` (only when the desktop shell exported
`OPENHUMAN_TAURI_VERSION`, the `TAURI_VERSION_ENV_VAR` constant) and
`x-sdk-name` (the product identity). Version strings are filtered to
header-safe characters and clamped to 64 bytes. `attribution_headers()`
builds that map.

`backend_client_builder(profile)` returns a `reqwest::ClientBuilder` with the
platform TLS backend (`openhuman_embed::__host::util::tls::tls_client_builder`, which
picks schannel on Windows and rustls elsewhere), HTTP/1 only, redirects
disabled, a 15 s connect timeout and the attribution headers as defaults. The
request timeout depends on the `TransportProfile`: 120 s for `Api` and 60 s
for `Integrations`. `build_backend_client` finishes the builder.

Redirects are off because the SDK adds `x-api-key` per request and `reqwest`
does not strip a custom header when following a cross-origin redirect.

### Product identity (`product.rs`)

OpenHuman and other products (OpenCompany, for example) share one login and
reach the backend through this crate. The `x-sdk-name` header tells the
backend which product a call came from. The identity is process-wide because
this crate builds backend requests at many call sites that no embedding product
owns, so a host sets it once at startup with `set_product_identity`, before
any backend traffic. A process that never sets it sends
`DEFAULT_PRODUCT_IDENTITY` (`"openhuman"`).

`ProductIdentity::new` keeps ASCII alphanumerics plus `.`, `_` and `-`,
truncates to 64 characters and lower-cases the result (the backend matches
against lower-case product names). It returns `None` when nothing remains. Storage is a `OnceLock<RwLock<ProductIdentity>>`
rather than a bare `OnceLock`, so tests can override it more than once in a
process.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | Declares the three modules and re-exports the common names (`ProductIdentity`, `set_product_identity`, `effective_backend_api_url`, `DEFAULT_API_BASE_URL`, and so on). |
| [`url.rs`](url.rs) | Base-URL resolution, environment overrides, staging default, and the inference-endpoint guard for control-plane calls. Re-exports a few URL helpers from the core (`join_url`, `normalize_api_base_url`, `host_is_local`). |
| [`headers.rs`](headers.rs) | `attribution_headers`, `backend_client_builder`, `build_backend_client`, `TAURI_VERSION_ENV_VAR`. |
| [`product.rs`](product.rs) | `ProductIdentity`, `set_product_identity`, `product_identity`, `product_identity_header(s)`, `PRODUCT_IDENTITY_HEADER`, `DEFAULT_PRODUCT_IDENTITY`. |

## Key types and entry points

- `effective_backend_api_url(&Option<String>)` ([`url.rs`](url.rs)): the base for any
  non-inference backend call. `hosted::client::HostedClient` and the
  transport's `base_url(ControlPlane)` both use it.
- `effective_api_url(&Option<String>)` (`url.rs`): the inference base, used by
  the transport's `base_url(Inference)`.
- `build_backend_client(TransportProfile)` ([`headers.rs`](headers.rs)): the `reqwest`
  client every hosted request rides.
- `set_product_identity(ProductIdentity)` ([`product.rs`](product.rs)): re-exported at the
  crate root; also reachable through `InstallOptions::product_identity`.

## Boundaries

- The core owns only the generic URL utilities this file
  re-exports (`openhuman_embed::__host::util::url`). It owns the app-environment
  reading too (`config::app_env`).
- Do not put the `x-sdk-name` header on third-party endpoints, MCP servers,
  BYOK inference endpoints or presigned storage redirects. These helpers are
  for TinyHumans backend traffic only.

## Gotchas

- Set the product identity before the transport is built. The `reqwest`
  clients capture `attribution_headers()` as default headers at construction.
  `InstallOptions::product_identity` orders this correctly.
- Tests that mutate `BACKEND_URL`, `VITE_BACKEND_URL` or the app-environment
  variables must hold `url::backend_env_test_lock()`. The environment is
  process-global, and a module-local lock does not stop races with other
  modules' tests. Compile-time env values are stubbed to `None` under
  `cfg(test)` so CI bakes do not leak in.

## Tests

[`headers_tests.rs`](headers_tests.rs), [`product_tests.rs`](product_tests.rs) and [`url_tests.rs`](url_tests.rs) sit beside their
modules.

```bash
cargo test -p openhuman-tinyhumans backend::
```

## Further reading

- [`docs/gitbooks/en/developing/tinyhumans-api-key.md`](../../../../docs/gitbooks/en/developing/tinyhumans-api-key.md): running on a TinyHumans API key.
- [`crates/openhuman-tinyhumans/README.md`](../../README.md): the openhuman-tinyhumans crate README.
- [`vendor/tinyhumans-sdk/README.md`](../../../../vendor/tinyhumans-sdk/README.md): tinyhumans-sdk.
