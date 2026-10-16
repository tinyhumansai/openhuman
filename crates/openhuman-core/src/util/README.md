# util

Small, self-contained helpers that several domains share: UTF-8-safe text
truncation, retry with backoff for flaky filesystem operations, log
redaction, URL normalization, JSON-RPC param decoding and the platform TLS choice for `reqwest`. `pub mod util;` in
`lib.rs` is unconditional, so everything here is compiled in every build.

## How it works

There is no flow here, only a rule about direction. Domains depend on `util`;
`util` depends on no domain. No file in this folder has a `use crate::` line.
A helper that needs a `Config`, a `Tool`, a workspace or security policy
belongs in the domain that owns those things. Domains import LLM-facing text
sanitization directly from `tinytools_std::sanitize`.

```text
  agent  memory  tools  inference  config  integrations  mcp  ...
     \      |      |        |        |          |         /
      +-----+------+--------+--------+----------+--------+
                              |
                              v
                        crate::util            (no `use crate::`)
                              |
                              v
        external crates: serde_json, sha2, url, reqwest, tokio,
                         tracing, anyhow
```

`util` is kernel surface, so it also sits under the dependency-floor ratchet
in `scripts/kernel-floor.limits` (checked by `scripts/check-kernel-floor.sh`).
Adding an external crate here raises the floor of every build. That is why
`redact` is a few lines of SHA-256 rather than a link to the memory engine's
copy.

## Layout

| Path | What it does |
| --- | --- |
| [`text.rs`](./text.rs) | Char-count truncation (`truncate_with_ellipsis`, `truncate_with_suffix`), byte-capped truncation with an ellipsis (`truncate_at_byte_boundary`), and byte-index rounding to a char boundary (`floor_char_boundary`, `ceil_char_boundary`, `utf8_safe_prefix_at_byte_boundary`). |
| [`retry.rs`](./retry.rs) | `retry_with_backoff` (sync, `std::thread::sleep`) and `retry_with_backoff_async` (`tokio::time::sleep`), plus `is_transient_fs_error`. |
| [`redact.rs`](./redact.rs) | `redact` hashes a string to 8 hex chars for log lines. `redact_url_for_log` replaces URL userinfo with `redacted`. |
| [`url.rs`](./url.rs) | `normalize_api_base_url`, `normalize_backend_api_base_url`, `join_url`, `host_is_local`. |
| [`params.rs`](./params.rs) | `read_required` and `read_optional`, the shared param decoders for controller handlers. |
| [`types.rs`](./types.rs) | `MaybeSet<T>` (`Set`, `Unset`, `Null`). |
| [`tls/`](tls/README.md) | `tls_client_builder()`, the platform-conditional TLS backend for `reqwest` clients. |

## Key types and entry points

### Text

`truncate_with_ellipsis(s, max_chars)` keeps at most `max_chars` characters
and appends `...` when it cuts. It counts characters, not bytes, so emoji and
CJK text never split. `truncate_at_byte_boundary` is the byte-capped
variant used when a size limit is in bytes (log fields, telemetry payloads).
`floor_char_boundary` and `ceil_char_boundary` round a byte index to the
nearest valid char boundary below or above it.

### Retry

`retry_with_backoff(op_name, attempts, base_ms, f)` retries `f` only while
the error is transient by `is_transient_fs_error`. Any other error returns at
once. The sleep is `base_ms * 2^i`, capped at 30 seconds. Each retry logs a
`warn!` (`[util] transient fs retry`), a success after retries logs `info!`,
and the final error carries `<op_name> failed after <n> attempts` context.
Use the async version on the tokio executor.

`is_transient_fs_error` walks the `anyhow` chain for an `io::Error` and, on
Windows, matches the mandatory-locking codes: 5 (`ERROR_ACCESS_DENIED`), 32
(`ERROR_SHARING_VIOLATION`), 33 (`ERROR_LOCK_VIOLATION`), 303
(`ERROR_DELETE_PENDING`), 665 (`ERROR_FILE_SYSTEM_LIMITATION`) and 1224
(`ERROR_USER_MAPPED_FILE`). On other platforms it returns `false`, so the
helpers run `f` once.

### Redaction

`redact(s)` returns the first 4 bytes of the SHA-256 digest as 8 hex chars.
It is stable across runs, so a developer holding the raw value can grep for
it. Use it for source ids, entity ids and content paths, which can embed
email addresses. `tinymemory_core::util::redact` is an identical, independent
copy; the two never need to agree because neither reads the other's output.

`redact_url_for_log(raw)` parses the URL (assuming `http://` when the scheme
is missing), replaces any username and password with `redacted`, and strips a
trailing slash. An unparseable input comes back trimmed and unchanged.

### URLs

`normalize_api_base_url` trims whitespace and trailing slashes without
parsing. `normalize_backend_api_base_url` also strips any path, query and
fragment, so an `api_url` set to a full inference endpoint
(`.../openai/v1/chat/completions`) becomes a bare origin before backend
paths are appended; it retries with `https://` for scheme-less values.
`join_url(base, path)` joins with RFC 3986 rules, which means an absolute
`path` replaces the base's path. Paths should start with `/`. `host_is_local`
classifies loopback, unspecified, RFC 1918 IPv4 and `localhost` /
`*.localhost` hosts using typed `url::Host` matching.

### Params

`read_required::<T>(params, key)` fails with `missing required param '<key>'`
or `invalid '<key>': <serde error>`. `read_optional` treats an absent key and
`null` the same (`Ok(None)`). Domain `schemas.rs` files use these so the error
wording stays one contract.

### Sanitization

The stripping rule for untrusted text going into a model's context lives in
`tinymcp_bus`, which applies it to every remote MCP tool description. This
module re-exports it at the old path so the orchestrator prompt builder's
skill descriptions get the same rule. Change it in `vendor/tinymcp`, not here.

### `MaybeSet`

`MaybeSet<T>` tells "field absent" (`Unset`) apart from "field explicitly
null" (`Null`) in partial-update payloads. The caller today is
`tools/impl/system/proxy_config.rs`.

### Re-exports

[`mod.rs`](./mod.rs) re-exports `read_optional`, `read_required`, `redact_url_for_log`,
the `retry` functions, the `text` functions and `MaybeSet` at the module root,
so `crate::util::truncate_with_ellipsis` works without naming the submodule.
`redact::redact`, `url::*` and `tls::tls_client_builder` are
reached through their submodule path. From outside the crate the library name
is `openhuman_core`, which is the path the `truncate_with_ellipsis` doctest
uses.

## Boundaries

- The sanitization rule is owned by `tinymcp_bus` (`vendor/tinymcp`).
- Hosted-backend URL resolution (defaults, `BACKEND_URL` overrides) belongs
  to `crates/openhuman-tinyhumans`, which publishes the result on
  `Config.resolved_endpoints` (`config::register_enricher`). [`url.rs`](./url.rs) only does string and URL shape work.
- Secret scrubbing for logs and Sentry lives in `core::log_redaction` and
  `core::observability`. `redact` is only a stable hash for identifiers.

## Gotchas

- Do not add `use crate::` here. If a helper needs domain types, it belongs in
  that domain.
- Do not add external dependencies casually. The kernel-floor ratchet only
  goes down, and raising it needs a written justification.
- `retry_with_backoff` blocks the thread while it sleeps. Call
  `retry_with_backoff_async` from async code.
- `join_url` with a relative `path` (no leading slash) drops the base's last
  segment, which is almost never what an API client wants.

## Tests

Tests sit beside each file ([`text_tests.rs`](./text_tests.rs), [`retry_tests.rs`](./retry_tests.rs),
[`redact_tests.rs`](./redact_tests.rs), [`url_tests.rs`](./url_tests.rs), [`params_tests.rs`](./params_tests.rs)). Run them with
`cargo test -p openhuman util::` or `pnpm debug rust util::`. The
`truncate_with_ellipsis` doctest runs under `cargo test -p openhuman --doc`.

## Further reading

- [Architecture overview](../../../../docs/gitbooks/en/developing/architecture.md)
- [Testing strategy](../../../../docs/gitbooks/en/developing/testing-strategy.md)
