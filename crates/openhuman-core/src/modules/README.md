# modules

This folder is the host for loadable native modules. A module is a first-party
`cdylib` that speaks the tinybus module ABI and ships separately from the core
binary: `tinycomputer`, `tinysearch`, `tinydocs`, `tinywallet`, `tinyjuice`,
`tinyvoice`, `tinyruntime` (with its `tinyruntime-nodejs` and
`tinyruntime-python` providers), `tinymcp`, `tinyconnectors`, `tinybox`,
`tinychannels` and `tinyhosts`. The core downloads a module from a pinned
GitHub release, checks it against a digest compiled into this crate, lets
tinybus admit it, attaches it to a private in-process broker, and then calls
it like any other bus service. Domains reach a module through the per-module
host files here ([`documents.rs`](./documents.rs), [`wallet.rs`](./wallet.rs), [`voice.rs`](./voice.rs), ...) rather than
talking to tinybus themselves.

The `//!` comments in [`mod.rs`](mod.rs), [`host.rs`](host.rs) and
[`ops.rs`](ops.rs) carry the full argument for this design. This README is
the map over them.

## How it works

### What a module buys and what it costs

Moving a capability into a module removes its dependency tree from the core
build instead of only gating it behind a feature. A document writer or an
integration SDK no longer compiles into a binary that mostly does something
else.

The price is isolation. A loaded module shares this process's address space,
privileges and crash domain. tinybus deadlines, bounded queues and caught
panics contain ordinary misbehavior, not a segfault, and `dlopen` runs code
before any symbol can be inspected. The ABI, manifest and digest gates decide
what is admitted, never what is safe. Modules are first-party code that
happens to ship separately; anything untrusted belongs in its own process.

tinybus also never unloads a library. A module that was refused, faulted, or
failed to initialize stays failed until the process restarts, so this host
caches failures instead of retrying them.

### Layering

```text
  domain code (tools, voice, web3, search, inference/tokenjuice, ...)
        |
        v
  per-module host file      documents.rs, wallet.rs, voice.rs, runtime.rs,
  (typed calls, errors)     desktop.rs, browser*.rs, connectors.rs, search/
        |
        v
  ops::ensure_loaded*  ---> registry::find(id)   (compiled-in ModuleRecord)
        |                         |
        |                         v
        |                  tinybus::module::resolution (one slot per id)
        v
  host::runtime()  -> ModuleRuntime { ModuleHost, Connection, tokio handle }
        |                 |
        |                 +-- private Broker on a MemoryBus
        v
  runtime.proxy(bus_name, object_path)  -> tinybus Proxy -> loaded module
```

### The module bus

`host::runtime()` builds the module bus once per process. It starts a
dedicated thread (`openhuman-module-bus`) with its own multi-threaded tokio
runtime, creates a `MemoryBus` transport and a `Broker`, spawns the broker,
wraps it in a tinybus `ModuleHost`, and connects the host's own `Connection`.
Before returning it installs the `tokenjuice_host` callback on that
connection. The result is stored in a `OnceLock` as a `ModuleRuntime` and
lives until the process exits; there is no shutdown path because there is
nothing a shutdown could reclaim.

Modules get their own broker because `core::bus::BUS` (a `OnceBus`) builds
its broker internally and never hands it out, while `ModuleHost::new` needs
one. The consequence is that a module can serve requests but cannot publish a
`DomainEvent`.

The dedicated runtime keeps module transports alive across short-lived caller
runtimes (`#[tokio::test]`, an embedder's task runtime). `ModuleRuntime::spawn`
runs process-lifetime work on it and `ModuleRuntime::blocking` runs admission
work on its blocking pool.

Admission runs in tinybus's permissive mode on purpose. Strict mode also
refuses a module whose rustc version string differs from the host's, and
published artifacts are built by CI on whatever toolchain that runner had.
Permissive mode still enforces the ABI revision, descriptor layout, target
triple, pointer width, endianness, feature bits, and the refusal of a
`panic=abort` module.

### Loading a module

Every caller goes through `ops::ensure_loaded(config, id)` or the bounded
`ops::ensure_loaded_within(config, id, within)`. The steps, in order:

1. If `config.modules.enabled` is false, fail with a message saying so.
2. `registry::find(id)` looks up the compiled-in `ModuleRecord`; an unknown id
   fails.
3. `tinybus::module::resolution::global().claim(id)` returns `Done` (already
   ready or already failed), `Wait` (someone else is resolving), or `Run`
   (this caller goes first). Each module has its own slot, so unrelated
   modules never queue behind each other.
4. A `Run` caller spawns the resolution on the module runtime, so a caller
   runtime that shuts down mid-load cannot cancel a resolution others are
   waiting on.
5. Every caller then waits on the slot. `within` bounds only the wait: a
   caller that gives up gets `LoadError::StillLoading`, and the download keeps
   running for the next caller.

The resolution itself (`ops::resolve`) tries the cheapest source first:

```text
  already serving on the broker?          (search path at boot, earlier load)
        | no
        v
  local override?                         [[modules.overrides]] id = "..."
        | no                              or TINYSEARCH_TEST_MODULE /
        v                                 TINYCONNECTORS_TEST_MODULE
  module search path?                     OPENHUMAN_MODULE_PATH, then
        | no                              platform data dirs (tinybus)
        v
  release cache (load_cached)
     host_candidates()  -> artifact keys this host can run, best first
     record.asset_for(key) -> ReleaseAsset { host_key, archive, sha256 }
     ReleasePlan { install_root, bundled_root, allow_download, ... }
     load_first_admitted() tries each asset until one is admitted
        |
        v
  prune_stale_versions() removes older cached versions of this module
```

Inside the release cache, tinybus looks for the archive in the bundled
directory first, then in `install_dir/<id>/<version>/<host_key>`, and
downloads only when `modules.allow_download` is on. On download it fetches the
release's own `checksum.toml`, checks it against the digest pinned in the
registry, hashes the archive, extracts it and `dlopen`s the library. All of
that is synchronous, so it runs on the module runtime's blocking pool and a
cold download never stalls other tasks. Later launches re-verify the cached
artifact from disk instead of downloading again.

A panic inside the loader is reported as an error carrying
`crate::tools::status::MODULE_FAULT_MARKER` and "restart the app to try
again", rather than unwinding into whoever was waiting.

### Bundled archives

Shipped builds carry the registry-pinned archives so normal use needs no
runtime download. `scripts/release/stage-modules.mjs` stages them at build
time (the oldest published key per OS and arch: `macos-15`, `ubuntu-22.04`,
`windows-2022` or `windows-11-arm64`) as the `bundled-modules/` Tauri resource
for desktop installers, under `/opt/openhuman/bundled-modules` in the Docker
image, and beside the binary in the CLI tarball and bench bundle.

`ops::bundled_releases_dir` resolves the directory from, in order, the path
the host registered with `ops::set_bundled_releases_dir` (the Tauri shell does
this in `crates/openhuman-app/src/lib.rs`), the `OPENHUMAN_BUNDLED_MODULES`
environment variable, and `bundled-modules/` beside the executable. Only an
existing directory counts. Bundled files go through the same digest and
admission checks as downloaded ones. Where the archive itself cannot ship (a
notarized macOS app), the bundle carries the archive's digest marker in its
place. When a bundled artifact exists but is refused, the load fails with a
"repair the installation" message instead of falling through to a download.
A plain `cargo build` host has no bundle and uses the release cache.

### Install directory

`ops::install_dir` is `modules.install_dir` when set, else
`<user cache dir>/openhuman/modules`, else `<workspace>/modules` for headless
containers with no cache directory.

### Module configuration

Some modules take a private configuration blob at load and on
reinitialization. `ops::module_config` builds it per id: `search::module_config`
for `tinysearch`, `desktop::module_config` (from [`computer_config.rs`](./computer_config.rs)) for
`tinycomputer`, and `connectors::module_config` for `tinyconnectors`.
Everything else gets `{}`. When the connector configuration cannot be built
(direct mode with no key, an unknown mode) the module still loads with `{}`,
because its capability members need no route.

The host files for those three modules fingerprint the configuration and call
`Connection::reinitialize_module` when it changes, so a rotated credential or
a sign-out reaches a module that is already serving. Secrets travel only in
that configuration, never in an ordinary method call or an RPC reply.

### Boot

`boot::load_declared_modules` runs as a background bootstrap job
(`core/runtime/services.rs`, gated on `ServiceSet::integrations`). It does two
things. It loads every artifact on the module search path through
`ModuleHost::load_search_paths`, so an operator-placed artifact wins over a
download. Then it calls `ensure_loaded` for each `LoadPolicy::Eager` record.
Every record in the registry is currently `LoadPolicy::Lazy`, so the second
step does nothing today. Boot never fails because of a module, and it never
downloads lazy modules: a user who never touches a feature never pays for its
download, its `dlopen`, or its resident memory.

## Layout

### Loading machinery

| Path | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | Design rustdoc for the whole loading model; re-exports `ensure_loaded`, `ensure_loaded_within`, `state_of`, `LoadError`, the controller lists and the public types. |
| [`registry.rs`](registry.rs) | The compiled-in `ALL` table and `find(id)`. Records live in [`registry/`](registry/README.md), one `records_*.rs` file per module family. |
| [`types.rs`](types.rs) | `ModuleRecord`, `PlatformAsset`, `LoadPolicy`, `ModuleSource`, `ModuleState`, `ModuleStatus`. |
| [`host.rs`](host.rs) | The module bus: the dedicated runtime, `ModuleRuntime`, `runtime()`, `is_started()`. |
| [`ops.rs`](ops.rs) | Resolution (`ensure_loaded*`, `resolve`, `load_cached`, `load_local`), per-module configuration, overrides, the bundled and install directories, and status reporting (`list`, `state_of`). |
| [`boot.rs`](boot.rs) | Startup loading: search-path artifacts, then eager records. |
| [`schemas.rs`](schemas.rs) | The `modules` RPC namespace and its handlers. |

### Per-module host files

| Path | Module | What it does |
| --- | --- | --- |
| [`documents.rs`](documents.rs) | `tinydocs` | `generate_docx`, `generate_pptx`, `extract_text`, `extract_document`, `render_pdf`. Inbound bytes ride a tinybus stream; produced documents are held by the module and pulled with `ReadOutput`, then released. Feature `documents`. |
| [`wallet.rs`](wallet.rs) | `tinywallet` | `derive_account`, `sign_transaction_in_module`, `sign_message`, `export_key`. Key material goes only to an attested module whose attested digest is one the registry pinned (`attested_proxy`). Feature `web3`. |
| [`voice.rs`](voice.rs) | `tinyvoice` | Intent routing, command extraction, wake-word and hallucination checks, capture preparation, WAV encoding, frame energies, and a `VadSession` driven from the always-on capture loop. Every call returns a `VoiceCallError` the caller falls back from. Feature `voice`. |
| [`runtime.rs`](runtime.rs) | `tinyruntime` + providers | `ensure_language` loads the router and the language provider, then `resolve`, `execute`, `languages`, `pool_stats`. Settings (`settings_for`, `pool_settings_for`) are read from config on every call. |
| [`desktop.rs`](desktop.rs) | `tinycomputer` | The shared proxy for TinyComputer: loads it with an 8 second bound, reinitializes it when its configuration fingerprint changes, and exposes `call`, `permissions`, `state`, `jev_ready`. |
| [`computer_config.rs`](computer_config.rs) | `tinycomputer` | The private configuration: `jev` (decision model), `planner` (planner, rescue and output models), `browser` (Chrome path), `trace_path`. Rebuilt from `[computer]` and stored credentials on every call; `OPENHUMAN_COMPUTER_TRACE` turns tracing on. |
| [`computer.rs`](computer.rs) | `tinycomputer` | `status` for the Computer settings page: lifecycle state, configured routes, and optionally what the module reports through `Describe`. |
| [`browser.rs`](browser.rs) | `tinycomputer` | `BrowserClient` for the `Browser*` members, over the desktop proxy, plus the shared website policy. |
| [`browser_task.rs`](browser_task.rs) | `tinycomputer` | Browser tasks over `StartTask` / `AwaitTask` / `ContinueTask`: `start`, `wait`, `resume`, `cancel`, `report`. The host keeps the policy (surfaces, origins, action budget, approvals). |
| [`browser_task_report.rs`](browser_task_report.rs) | `tinycomputer` | What is kept when a task stops: a content-free log summary, and with tracing on the full report under `<workspace>/state/computer/tasks/`. |
| [`browser_sites.rs`](browser_sites.rs), [`browser_sites/store.rs`](browser_sites/store.rs) | `tinycomputer` | Per-site memory of plans and elements learned by finished tasks, in `<workspace>/state/computer/sites/<site>.json`, with 30-day expiry. Fed back on `StartTask`; never reaches the agent's memory or prompts. |
| [`connectors.rs`](connectors.rs) | `tinyconnectors` | `proxy`, `call`, `call_stateless`, `call_bare`, `reconcile_route_if_loaded`. Builds the route configuration (signed-in backend or the user's own Composio key) and reconciles it on every call so sign-out reaches the module. |
| [`search/mod.rs`](search/mod.rs) | `tinysearch` | `module_config` from the host's resolved search policy, and `configured_tool_specs` computed synchronously from that same configuration. |
| [`search/proxy.rs`](search/proxy.rs) | `tinysearch` | Lazy load, private reinitialization on configuration change, and serialized `list_tools` / `execute_tool`; `refresh_loaded` after settings saves and credential changes. |
| [`tokenjuice_host.rs`](tokenjuice_host.rs) | `tinyjuice` | The reverse direction: an `MlHost` object the host serves at the contract's `ML_HOST_NAME` / `ML_HOST_PATH`, so the module can call the host's ML compressor and one tool-less model call for its summary stage. |

`tinymcp`, `tinybox`, `tinychannels` and `tinyhosts` have registry records but
no host file in this folder.

## Key types and entry points

- `ModuleRecord` ([`types.rs`](./types.rs)) is one compiled-in module: `id`, `description`,
  `bus_name`, `object_path`, `version`, `release_url`, its `assets`, and its
  `load` policy. `asset_for(host_key)` picks the artifact for a host key.
- `PlatformAsset` (`types.rs`) is one published archive: the host key (for
  example `ubuntu-24.04-x86_64`), the archive name, and its lowercase hex
  SHA-256 from the release's `checksum.toml`.
- `LoadPolicy` (`types.rs`) is `Lazy` (first use) or `Eager` (at boot).
- `ModuleState` (`types.rs`) is the coarse state reported over RPC:
  `Available`, `Loading`, `Ready`, `Failed`, `Unsupported`. `ModuleStatus`
  wraps it with id, version, bus name and a `detail` that never carries a
  path, a credentialed URL, or a payload.
- `ops::ensure_loaded` / `ops::ensure_loaded_within` (`ops.rs`) are the only
  way to load a module. `LoadError` separates `Failed` (terminal for this
  process) from `StillLoading`.
- `ops::state_of` and `ops::list` (`ops.rs`) report state without loading
  anything.
- `host::runtime()` (`host.rs`) returns the `ModuleRuntime`; its `proxy`
  method is how host files reach a loaded module. `host::is_started` lets
  status code answer without starting a broker.
- `ops::set_bundled_releases_dir` (`ops.rs`) is called by the desktop host
  before the core starts.

## Calls before core startup

Hosts obtain `ModuleClient` through `openhuman_rpc::embed::modules`. Construct
it with explicit configuration, and set bundled-artifact discovery before the
first call. Construction starts nothing; calls use the same process-wide lazy
loader as the core and require neither a core runtime nor a signed-in session.
The client preserves argument tuple arity and offers confidential calls that
require recipient attestation. A failed load or call returns a sanitized
`ModuleCallError` and never executes an implementation library as a fallback.

Stateful adapters must call the module's close, cancellation and shutdown
members to release its resources. Keeping a native library mapped does not
remove that obligation.

## RPC surface

All methods are in the `modules` namespace ([`schemas.rs`](./schemas.rs)), wired into the
registry from `core/all.rs`:

| Method | What it does |
| --- | --- |
| `modules.list` | Status of every module in the registry. |
| `modules.status` | Status of one module by `id`. |
| `modules.load` | Resolve and load a module now instead of on first use; returns its status after the attempt. |
| `modules.browser_check_readiness` | Load TinyComputer and briefly launch Chrome; returns `module_ready`, `chrome_ready` and a sanitized `error`. |
| `modules.browser_forget_sites` | Forget learned site memory for one `site`, or for every site when omitted; returns how many were forgotten. |
| `modules.computer_status` | TinyComputer state, decision and planner routes, and with `load` set, what the module reports through `Describe`. |

No method takes a path or an arbitrary artifact. Only the compiled registry
decides what can load.

## Configuration

`config::schema::modules::ModulesConfig` (`[modules]` in `config.toml`)
controls whether and from where the compiled-in modules load, never what is
loadable:

- `enabled`: modules may load at all. When off, `modules.list` reports every
  module as `Unsupported` even if one is still mapped from an earlier request.
- `allow_download`: a missing module may be fetched from its pinned release.
  Off pins the host to bundled or already-cached artifacts.
- `install_dir`: overrides the release cache location.
- `overrides`: `[[modules.overrides]]` entries (`id`, `path`) that load a
  local build instead of the release. An override bypasses the digest check,
  which is acceptable only because the operator named their own file.

## Boundaries

- The loader, ABI and manifest gates, release cache, per-id resolution table,
  platform host keys (`tinybus::module::platform::host_candidates`) and the
  cache-path helpers (`prune_stale_versions` and friends) belong to tinybus
  (`vendor/tinybus`). Change them there.
- Each module's behavior lives in its own repository under `vendor/`
  (`tinydocs`, `tinywallet`, `tinyvoice`, `tinyruntime`, `tinycomputer`,
  `tinysearch`, `tinyconnectors`, `tinyjuice`, `tinymcp`, `tinybox`,
  `tinychannels`, `tinyhosts`). This folder holds only the host half of each
  call and the host's policy around it.
- Each module has a `*-bus` contract crate with its interface names, method
  constants, request and response types, and contract version:

  | Contract | Feature or role |
  | --- | --- |
  | `tinydocs-bus` | `documents` |
  | `tinyvoice-bus` | `voice` |
  | `tinyjuice-bus` | inference kernel |
  | `tinyruntime-bus` | runtime clients |
  | `tinywallet-bus` | `web3` (the chain primitives are in `tinywallet-crypto`) |
  | `tinymcp-bus` | `mcp` |
  | `tinysearch-bus` | search provider declarations and bus payloads |
  | `tinycomputer-bus` | desktop, browser and task members |
  | `tinychannels-bus` | channel vocabulary |
  | `tinyconnectors-bus` | connector names and the Composio types |

  Never redeclare a contract type here, and call members through the
  contract's constants rather than string literals. Contract crates stay
  synchronous and free of I/O. Serialized types, errors, identifiers, versions,
  schemas and static tool declarations belong in the contract. Component
  algorithms execute inside the compiled module; configuration, execution
  policy and approvals stay in this host.
- Policy stays here even where the work moved into a module. For connectors
  that means egress policy (`crate::security::egress`, applied before
  `Execute`), route selection, and webhook delivery; scope enforcement belongs
  to the module, and the host must not re-filter against a second copy of the
  preference. For browser tasks it means allowed surfaces and origins, action
  budgets, and approval of irreversible steps.
- Callers outside this folder: `runtime::client` re-exports `execute`,
  `resolve` and `RuntimeCallError` from `modules::runtime` so `runtime/`
  compiles without the gate; `web3/seams.rs` and `web3/x402/seams.rs` use
  `wallet.rs`; `tools/impl/document`, `tools/impl/presentation`,
  `agent/multimodal.rs` and `agent/attachments/` use `documents.rs`;
  `voice/always_on` and `voice/streaming.rs` use `voice.rs`;
  `tools/impl/browser/` and `desktop/control/` use the TinyComputer files;
  `integrations/composio/module_client.rs` uses [`connectors.rs`](./connectors.rs);
  `search/tools.rs` and `search/bus.rs` use [`search/`](./search/); and
  `inference/tokenjuice` calls `ensure_loaded(config, "tinyjuice")` and is
  called back through [`tokenjuice_host.rs`](./tokenjuice_host.rs).

## Gotchas

- The registry, configuration types and shared `ModuleClient` are available
  without the `modules` feature. The loader and capability adapters are gated
  on `modules`, which is in both the contributor default set and the shipped
  product set. A client compiled without the loader returns an explicit
  unavailable error on a call. `documents`, `voice` and `web3` each imply
  `modules` and turn on their host file here.
  Never enable `modules` directly on the unconditional `tinybus` dependency;
  it is forwarded from this crate's own feature (`modules =
  ["tinybus/modules", ...]`), so a build without `modules` does not pull in
  the loader.
- Pin release checksums verbatim from the release's own `checksum.toml`.
  Never compute a replacement digest from a local build: that check would
  agree with whatever was served.
- A failed module is terminal for the process. Do not add retries or an
  unload path; the fix is a restart, and the error says so.
- Keep the ABI, manifest, dependency and digest admission checks intact.
- `ensure_loaded` waits without bound. Use `ensure_loaded_within` from any
  path with its own deadline, and treat `StillLoading` as "ask again later".
- The browser website list controls document navigation, including redirects
  and link clicks. It is not a network sandbox: an allowed page can still
  load subresources from other hosts, and DNS rebinding is not covered. Hosts
  that need private-network isolation must restrict the browser process at
  the network layer.
- Initialize recursive submodules before building:
  `git submodule update --init --recursive vendor/`.

## Tests

Tests sit beside each file as `<file>_tests.rs`. Run them with
`cargo test -p openhuman modules::` or `pnpm debug rust modules`. Tests that
need a real module load a local build through `TINYSEARCH_TEST_MODULE`,
`TINYCONNECTORS_TEST_MODULE`, or `[[modules.overrides]]`;
`scripts/test-rust-with-mock.sh` and CI build the pinned submodules.

See also `gitbooks/developing/performance.md` for how on-demand modules keep a
minimal build small.

## Further reading

- [Loadable modules](../../../../gitbooks/developing/loadable-modules.md)
- [tinybus submodule](../../../../vendor/tinybus/README.md)
- [Architecture overview](../../../../gitbooks/developing/architecture.md)

The connector host consumes verified release v0.14.0 (contract 1.13). The
argument/default/filter/classification and trigger-archive adapters call bus
members; no normal or build edge reaches `tinyconnectors` or its sync library.
Provider messages remain product output, while terminal bus reports contain
only the module/version/stage/platform/reason vocabulary. Credential-bearing
configuration and direct reads use attested confidential calls.
