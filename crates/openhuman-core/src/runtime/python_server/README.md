# runtime/python_server

The long-lived Python worker that keeps model-backed backends warm. This
module owns the process lifecycle and wire protocol; interpreter resolution
(finding or provisioning a `python` binary) belongs to the sibling
[`runtime/python`](../python/README.md) client, which this module calls into
to pick the interpreter it launches.

One backend currently runs inside the worker process:

- **Kompress** ([`kompress.rs`](./kompress.rs)): TokenJuice's ModernBERT/torch plain-text
  compressor, gated by `config.tokenjuice.ml_compression_enabled`.

It is gated behind `config.runtime_python.enabled`; `registry::enabled_backends`
computes the active set from that flag and `config.tokenjuice.ml_compression_enabled`.

## Key files

| File | Role |
| --- | --- |
| [`mod.rs`](./mod.rs) | Submodule decls and `pub use` re-exports. |
| [`server.rs`](./server.rs) | Host side: maps `Config` and the managed interpreter onto a `ServerLaunch`, holds the process-wide `ServerSlot` (`ensure_started`, `status`). The process lifecycle itself (spawn, handshake, request/response, restart-on-failure, idle expiry, start back-off) is `tinyruntime_pyserver::{PythonServer, ServerSlot}`. |
| [`registry.rs`](./registry.rs) | `RuntimePythonBackend` enum (`Kompress`) and `enabled_backends(config)`. |
| [`kompress.rs`](./kompress.rs) | Kompress venv provisioning (`ensure_kompress`, `install_into`, `kompress_provisioned`) and the compress request (`request_kompress`). |
| (`tinyruntime-pyserver`) | Owns the JSONL wire types (`PROTOCOL_VERSION`, request/response/ready-line), the status types (`BackendStatus`, `ServerStatus`, re-exported here as `RuntimePythonServerStatus`) and the worker script itself (`SERVER_SCRIPT`), written to the cache root as `runtime_python_server.py` each time a server is prepared. Library crate in `vendor/tinyruntime`. |

## Lifecycle

`ensure_started(config)` is the single entry point. It caches one
`Arc<RuntimePythonServer>` behind a process-wide `OnceLock<Mutex<ServerCache>>`
(`Empty` / `Ready` / `Failed { message, retry_after }`):

- If the enabled backend set has changed since the cached server launched
  (e.g. Kompress toggled on or off), the cache is discarded and a new server
  is started: the running process was never provisioned for the new set.
- If the Kompress backend has been idle longer than
  `config.tokenjuice.ml_sidecar_idle_timeout_secs`, the server is torn down and
  restarted on the next request, freeing the torch process's memory.
- A startup failure caches `Failed` with a five-minute (`START_FAILURE_BACKOFF`)
  retry window so a broken venv does not retry on every call.
- `RuntimePythonServer::request` sends one request, and on failure resets the
  child and retries once before giving up.

`prepare_launch` picks the interpreter for the worker: when Kompress is enabled it gets its own dedicated venv via `kompress::ensure_kompress`; otherwise the worker runs under the base interpreter from `runtime::python::PythonBootstrap`.

## Wire protocol

JSONL over the child's stdin/stdout (`tinyruntime_pyserver::protocol`), one line per message:

- Startup handshake: the worker writes a `ReadyLine` (`ready`, `protocol`,
  `backends`, optional `error`) before any request is sent; a `protocol`
  mismatch against `PROTOCOL_VERSION`, `ready: false`, or no line within
  `HANDSHAKE_TIMEOUT` (30s) fails the launch.
- Requests: `PythonServerRequest { id, method, params }`, methods namespaced by
  backend (`kompress.compress`).
- Responses: `PythonServerResponse { id, ok, result, error }`, matched back to
  the request by `id`; the read loop skips lines with a stale/unparseable `id`
  and enforces a 60s per-request timeout (`REQUEST_TIMEOUT`, sized for the
  slower Kompress backend).

stderr is drained continuously by a background task and only logged at
`debug`/`trace`, never surfaced to callers.

## Backends

**Kompress** (`kompress.rs`): TokenJuice's ModernBERT/torch plain-text
  compressor, gated by `config.tokenjuice.ml_compression_enabled`.

It is gated behind `config.runtime_python.enabled`; `registry::enabled_backends`
computes the active set from that flag and `config.tokenjuice.ml_compression_enabled`.

## Key files

| File | Role |
| --- | --- |
| [`mod.rs`](./mod.rs) | Submodule decls and `pub use` re-exports. |
| [`server.rs`](./server.rs) | Host side: maps `Config` and the managed interpreter onto a `ServerLaunch`, holds the process-wide `ServerSlot` (`ensure_started`, `status`). The process lifecycle itself (spawn, handshake, request/response, restart-on-failure, idle expiry, start back-off) is `tinyruntime_pyserver::{PythonServer, ServerSlot}`. |
| [`registry.rs`](./registry.rs) | `RuntimePythonBackend` enum (`Kompress`) and `enabled_backends(config)`. |
| `kompress.rs` | Kompress venv provisioning (`ensure_kompress`, `install_into`, `kompress_provisioned`) and the compress request (`request_kompress`). |
| (`tinyruntime-pyserver`) | Owns the JSONL wire types (`PROTOCOL_VERSION`, request/response/ready-line), the status types (`BackendStatus`, `ServerStatus`, re-exported here as `RuntimePythonServerStatus`) and the worker script itself (`SERVER_SCRIPT`), written to the cache root as `runtime_python_server.py` each time a server is prepared. Library crate in `vendor/tinyruntime`. |

## Lifecycle

`ensure_started(config)` is the single entry point. It caches one
`Arc<RuntimePythonServer>` behind a process-wide `OnceLock<Mutex<ServerCache>>`
(`Empty` / `Ready` / `Failed { message, retry_after }`):

- If the enabled backend set has changed since the cached server launched
  (e.g. Kompress toggled on or off), the cache is discarded and a new server
  is started: the running process was never provisioned for the new set.
- If the Kompress backend has been idle longer than
  `config.tokenjuice.ml_sidecar_idle_timeout_secs`, the server is torn down and
  restarted on the next request, freeing the torch process's memory.
- A startup failure caches `Failed` with a five-minute (`START_FAILURE_BACKOFF`)
  retry window so a broken venv does not retry on every call.
- `RuntimePythonServer::request` sends one request, and on failure resets the
  child and retries once before giving up.

`prepare_launch` picks the interpreter for the worker: when Kompress is enabled it gets its own dedicated venv via `kompress::ensure_kompress`; otherwise the worker runs under the base interpreter from `runtime::python::PythonBootstrap`.

## Wire protocol

JSONL over the child's stdin/stdout (`tinyruntime_pyserver::protocol`), one line per message:

- Startup handshake: the worker writes a `ReadyLine` (`ready`, `protocol`,
  `backends`, optional `error`) before any request is sent; a `protocol`
  mismatch against `PROTOCOL_VERSION`, `ready: false`, or no line within
  `HANDSHAKE_TIMEOUT` (30s) fails the launch.
- Requests: `PythonServerRequest { id, method, params }`, methods namespaced by
  backend (`kompress.compress`).
- Responses: `PythonServerResponse { id, ok, result, error }`, matched back to
  the request by `id`; the read loop skips lines with a stale/unparseable `id`
  and enforces a 60s per-request timeout (`REQUEST_TIMEOUT`, sized for the
  slower Kompress backend).

stderr is drained continuously by a background task and only logged at
`debug`/`trace`, never surfaced to callers.

## Backends

**spaCy** (`spacy.rs`): `ensure_spacy` provisions the `spacy-venv`
(`pip install spacy click`, `python -m spacy download en_core_web_sm`),
tracked by a versioned ready marker (`SPACY_READY_MARKER_VERSION`) so a
package-set change forces re-provisioning; `spacy_provisioned` is a cheap,
network-free readiness check. `extract` sends `spacy.extract` and
returns the shared `tinymemory_api::host::SpacyResponse` type. Called from
`modules::memory_host` for the memory tree's query extractor.

**Kompress** (`kompress.rs`): `ensure_kompress` provisions a CPU-only torch +
transformers venv and pre-downloads `config.tokenjuice.ml_model_id`;
The worker loads the model fully offline
(`HF_HUB_OFFLINE=1`, `TRANSFORMERS_OFFLINE=1`) so startup never depends on the
network once provisioned. `kompress_provisioned` is the network-free marker check the `kompress` init
step uses. `request_kompress` sends `kompress.compress` with
`target_ratio` / `max_input_chars` from `config.tokenjuice`. Called from
`inference::tokenjuice::ml`.

Provisioning is serialized behind a `tokio::sync::Mutex` so concurrent callers
don't race the same venv build.

## Status

`RuntimePythonServerStatus { enabled, running, backends: Vec<BackendStatus>, message }`
is returned by `status()` and reflects the cache directly: `Empty` reports
`disabled`, `Failed` reports the last error, `Ready` reports each backend's
`ready` flag from the worker's handshake `backends` list. There is no public
RPC method for this.

## Persistence

No domain store. `server::python_server_cache_root` picks the root:
`<runtime_python.cache_dir>/runtime-python-server` when configured, else
`<OS cache dir>/openhuman/runtime-python-server`, else
`<workspace_dir>/runtime_python_server`. Under it live `kompress-venv/`, the Kompress HF cache `kompress-hf/`, and the written
`runtime_python_server.py`.

## Security

The worker runs under whichever interpreter `runtime::python::PythonBootstrap`
or the venv provisioning resolved: it does not choose or sandbox that
interpreter itself, and it is spawned with the core's own
environment inherited (there is no `env_clear`). On top of that the module
adds `OPENHUMAN_RPS_BACKENDS` (the enabled backend list) and, for Kompress,
`OPENHUMAN_RPS_KOMPRESS_{MODEL,DEVICE,TARGET_RATIO,MAX_INPUT_CHARS}`,
`HF_HOME`, `HF_HUB_OFFLINE=1`, `TRANSFORMERS_OFFLINE=1`, and
`HF_HUB_DISABLE_TELEMETRY=1`. `process_util::apply_no_window` is applied to
the worker spawn and to every provisioning command so Windows shows no
console flash. Sandbox/approval policy for what reaches this worker is decided by the caller before the request is
sent, not here.

## Used by

- `crates/openhuman-core/src/inference/tokenjuice/ml/mod.rs`: `request_kompress` for
  plain-text compression.

## Notes / gotchas

- Restart-on-failure in `RuntimePythonServer::request` means a transient
  worker crash is invisible to callers except for added latency on the retried
  call.

## Further reading

- [Parent module (`runtime`)](../README.md)
- [tinyruntime submodule](../../../../../vendor/tinyruntime/README.md)
- [System and utilities tools](../../../../../gitbooks/features/native-tools/system-and-utilities.md)
- [Loadable modules](../../../../../gitbooks/developing/loadable-modules.md)
