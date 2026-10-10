# runtime

The client side of code execution. Agents, skills and flows run JavaScript and
Python through this folder: the `node_exec`, `npm_exec`, `python_exec` and
`shell` tools, Node-dependent skills, TokenJuice's ML compressor and the flows
native-tool backend all reach a toolchain or a worker from here.

The machinery itself is not here. Finding, downloading, verifying, unpacking
and caching an interpreter, and keeping pools of warm workers, all belong to
the `tinyruntime` module (`vendor/tinyruntime`), loaded over TinyBus and
reached through `crate::modules::runtime`. This folder asks that module for
answers and adapts them onto the types the rest of the core already uses
(`ResolvedNode`, `ResolvedPython`, `PoolExecOutcome`, `PoolRunError`).

## How it works

```text
 tools/impl/system (node_exec, npm_exec, python_exec, shell)
 skills runtime, flows oh: backend, tokenjuice ml
        |                 |                 |                |
        v                 v                 v                v
  runtime::node     runtime::python    runtime::pool   runtime::python_server
  (NodeBootstrap)   (PythonBootstrap)  (run_inline)    (ensure_started)
        \                 |                 /                |
         +----------------+----------------+                 |
                          |                                  v
                          v                         tinyruntime_pyserver
                 runtime::client                    (worker lifecycle,
                 (execute, resolve,                  JSONL protocol)
                  RuntimeCallError)
                    |            \
       feature "modules" on    feature "modules" off
                    |              \
                    v               v
         crate::modules::runtime   client/disabled.rs
         (TinyBus -> tinyruntime)  (every call: Unavailable)
```

### Resolving a toolchain

`NodeBootstrap` and `PythonBootstrap` have the same shape. Each is built over
an `Arc<Config>` and memoises the last answer:

1. `try_cached()` returns the memoised toolchain without awaiting anything.
   The shell uses it on every command to decide whether to prepend a managed
   bin dir to `PATH`, where a blocking bus round trip would change the
   meaning of an unrelated command.
2. `probe_installed()` asks the module to resolve without installing. A warm
   start finds an existing install, and a cold one reports `None` instead of
   starting a download nobody asked for.
3. `resolve()` asks the module to resolve and install if needed, then adapts
   the reply (`ResolvedNode { node_bin, npm_bin, bin_dir, version, source }`,
   `ResolvedPython { python_bin, bin_dir, version, source }`).

Both check their config switch first (`config.node.enabled`,
`config.runtime_python` for Python) and fail or return `None` when the
runtime is turned off.

### Pooled inline execution

`pool::node::run_inline` and `pool::python::run_inline` send one inline job to
the module's warm worker pool through `runtime::client::execute`. Whether a
language pools at all is decided here (`pool::node::enabled`,
`pool::python::enabled`): Node defaults on because each job gets its own
`worker_thread`, Python defaults off because jobs would share one interpreter.
`[runtime_pool] enabled = false` reverts every caller to its per-call spawn.

A module failure is classified into `PoolRunError`, and the exec tools act on
the variant. `PreDispatch` means the job never reached a worker, so a fallback
spawn is safe. `PostDispatch` means it may have run, so the caller must not
retry. `Saturated` means the pool shed the job, and the caller must not spawn
around it either. See [`pool/`](pool/README.md) for the full table.

### The persistent Python worker

`python_server::ensure_started(config)` keeps one long-lived Python process
warm for model-backed backends. Today the only backend is Kompress, the
ModernBERT plain-text compressor used by TokenJuice, enabled by
`config.tokenjuice.ml_compression_enabled`. This folder maps `Config` and the
managed interpreter onto a `ServerLaunch` and holds the process-wide slot; the
process lifecycle and the JSONL protocol are `tinyruntime_pyserver`.

## Layout

| Path | What it does |
| --- | --- |
| [`client/`](client/README.md) | The single import point for `modules::runtime` (`execute`, `resolve`, `RuntimeCallError`). With the `modules` feature off it re-exports `disabled.rs`, which answers every call with `RuntimeCallError::Unavailable`. |
| [`node/`](node/README.md) | `NodeBootstrap`, the Node toolchain client (behind `runtime-node`), plus the ungated native-tool bridge (`ops.rs`, `types.rs`) and the `javascript.*` controllers (`schemas.rs`, `rpc.rs`). `stub.rs` carries the `NodeBootstrap` type surface when `runtime-node` is off. |
| [`javascript/`](javascript/README.md) | A re-export facade over [`node`](./node) so callers import a language slot (`crate::runtime::javascript`) instead of a backend. No logic. |
| [`python/`](python/README.md) | `PythonBootstrap`, the Python interpreter client. |
| [`python_server/`](python_server/README.md) | Host side of the persistent Python worker: `ensure_started`, `status`, the backend registry and Kompress provisioning (`ensure_kompress`, `request_kompress`). |
| [`pool/`](pool/README.md) | Pooled inline execution (`run_inline` per language), the per-language enable decision, and `PoolRunError` classification. |

## Key types and entry points

- `NodeBootstrap`, `ResolvedNode`, `NodeSource` ([`node/bootstrap.rs`](./node/bootstrap.rs)) are the
  Node toolchain client and its answer. `ShellTool` holds an
  `Option<Arc<NodeBootstrap>>`.
- `PythonBootstrap`, `ResolvedPython`, `PythonSource` ([`python/bootstrap.rs`](./python/bootstrap.rs))
  are the Python equivalents.
- `pool::node::run_inline`, `pool::python::run_inline`, `PoolExecOutcome`,
  `PoolRunError`, `PoolSettings` ([`pool/`](./pool/)) are what `node_exec` and
  `python_exec` call and match on.
- `python_server::ensure_started`, `status`, `request_kompress`
  ([`python_server/`](./python_server/)) start the worker and send it a compress request.
- `runtime::node::execute_tool` and `list_tools` ([`node/ops.rs`](./node/ops.rs)) build the
  full agent tool registry (`tools::ops::all_tools_with_runtime`) and run one
  tool by name, publishing `ToolExecutionStarted` and
  `ToolExecutionCompleted` on the bus with session id `"javascript"`.

## RPC / CLI surface

With `runtime-node` on, [`node/schemas.rs`](./node/schemas.rs) registers the [`javascript`](./javascript)
namespace through `core/all.rs`:

| Method | Description |
| --- | --- |
| `javascript.list_tools` | List the agent tools an embedded JavaScript host can call. |
| `javascript.execute_tool` | Run one agent tool by name with JSON arguments. |

## Boundaries

- Interpreter discovery, managed installs, digest checks, archive
  extraction, install caches and the warm worker pools belong to the
  `tinyruntime` module (`vendor/tinyruntime`), reached only through
  `crate::modules::runtime`.
- The Python worker's process lifecycle, wire types and script
  (`SERVER_SCRIPT`) are `tinyruntime_pyserver`, a library crate in
  `vendor/tinyruntime`.
- How `tinyruntime` and its language providers are loaded and admitted is
  covered in [`modules/registry`](../modules/registry/README.md).
- The exec tools that use these clients live in
  `crates/openhuman-core/src/tools/impl/system/`.
- Runtime settings ([`node`](./node), `runtime_python`, `runtime_pool`) are defined
  under `config/schema/`.

## Gotchas

- `runtime/` is always compiled even though `crate::modules` is behind the
  `modules` feature, because `ShellTool` is kernel code that names
  `NodeBootstrap`. That is why every module call goes through
  `runtime::client`: with `modules` off the calls answer `Unavailable`
  instead of failing to build.
- `runtime-node` (default on, and in `scripts/ci/product-features.txt`) gates
  the managed Node client, the `javascript.*` controllers, `node_exec`,
  `npm_exec` and the `node_runtime` harness step. [`node/ops.rs`](./node/ops.rs) and
  [`node/types.rs`](./node/types.rs) stay ungated because the flows `oh:` native-tool backend
  uses them in every build.
- `PoolRunError` classification reads the module's error text
  ("pool is at capacity", "failed after dispatch"). Anything unrecognised is
  treated as pre-dispatch.

## Tests

Tests sit beside their modules as `*_tests.rs`. Run them with
`cargo test -p openhuman runtime::` or `pnpm debug rust runtime::`. Build with
`--no-default-features` and a feature list without `runtime-node` (or without
`modules`) to check the stub paths compile.

See `gitbooks/developing/performance.md` for how pooling and in-process
execution feed into the project's density and cold-start numbers.

## Further reading

- [tinyruntime submodule](../../../../vendor/tinyruntime/README.md)
- [System and utilities tools](../../../../gitbooks/features/native-tools/system-and-utilities.md)
- [Loadable modules](../../../../gitbooks/developing/loadable-modules.md)
