# Standalone consumers

Embed currently ships as source in the OpenHuman monorepo. Its crates have
`publish = false`; there is no crates.io release or promised semver boundary.
Pin a full 40-character commit SHA and commit the consumer's `Cargo.lock`.
Use a reviewed commit on upstream `main`, or a reviewed release tag resolved
to its full SHA. Never track a floating `main` dependency in a production
server. Updating the pin is an explicit dependency change with host tests.

The supported bootstrap generates a fresh consumer workspace. A checkout
containing the script is enough; its own submodules need not be initialized.
With Python 3.11+, Git and Rust 1.99.1 installed:

```sh
python3 scripts/bootstrap-embed-consumer.py \
  --rev 9d35caa5f142499933e3aacc8fc68bac64aac741 \
  --destination /path/to/new-host
cd /path/to/new-host
cargo check
cargo check --features embed
```

The pin above is an example baseline, not a claim that later additions are
present there. Choose the reviewed commit containing the APIs your host uses.
`--source /path/to/local/openhuman` can use a local Git checkout. The checkout
must contain the requested commit; uncommitted files, local `.env` files and
untracked credentials are not copied. Credential-bearing source URLs are
refused; use Git credential helpers or SSH authentication. An existing
destination is never overwritten. Git failures leave no partial destination.

The bootstrap:

1. Clones committed source into `vendor/openhuman`, verifies the exact commit,
   and initializes recursive submodules at their recorded gitlinks.
2. Reads that pin's workspace `[patch]` tables and generates the matching
   consumer-root Cargo patches with relative paths. Paths outside the source
   checkout and unknown patch forms are refused.
3. Creates an optional `openhuman-embed` dependency, `embed` feature and pinned
   toolchain. Seeds the host lockfile from the source lockfile when present,
   then resolves it and removes only Cargo-reported unused patches. This avoids
   nondeterministic `patch.unused` ordering causing spurious `--locked` failures.
   Strips Git metadata from the generated bundle so the host can commit source
   files normally, without accidentally creating undocumented Git submodules.
   `OPENHUMAN_REV` records the source pin.

Cargo does not inherit a dependency workspace's patches. A bare git dependency
on Embed can therefore resolve different TinyTools/TinyInference copies and
create incompatible types. Cargo normally fetches Git submodules itself; that
does not make dependency-root patches propagate. The bootstrap owns both
steps so consumers do not have to initialize submodules or mirror patch lists
by hand. For an existing host, move the generated dependency, feature and
patch sections to its workspace-root manifest together, retaining the bundle
at the same relative path. Regenerate from the new pin when upgrading.

This is a source bundle, not a binary SDK or a registry package. Initial setup
requires network access unless the needed Git repositories are already local.
Cargo's registry cache is also needed for a subsequent `--offline` build.
Pass `--offline` to the bootstrap to resolve Cargo using cached registry sources;
this flag does not disable Git fetching during source/submodule preparation.

# Feature and HTTP boundaries

The generated host has `default = []`; its default build does not compile
Embed, the core, or an HTTP client through this dependency. The `embed` feature
turns on `openhuman-embed` with `default-features = false`, which avoids the
core's default product feature set and optional hosted modules.

**Enabling Embed still compiles the core and its unconditional HTTP
libraries.** This pin does not offer an HTTP-free Embed implementation. Do not
describe `default-features = false` as an offline dependency graph. It controls
features, not all transitive dependencies. A host that must have no HTTP in
its default binary must keep Embed optional and feature-gate every reference
to it, as the generated entry point does. The core does not contact a service
merely because its HTTP libraries are linked; runtime configuration and the
chosen operation determine requests.

The supported toolchain is Rust 1.99.1, matching `rust-toolchain.toml`.
Dependencies use `cfg_select!`, stabilized in Rust 1.96; older toolchains are
not supported by this source pin. Pinning the exact toolchain also makes
consumer diagnostics reproducible.

# Sharing a server runtime

Use `openhuman_embed::process::tokio_runtime()` or its configurable
`tokio_runtime_builder()` to build Tokio. These helpers set the worker stack
size and blocking-thread limit required by deep agent turns; a host does not
need to import core constants or hand-assemble the builder.

Create one `Runtime` during server startup and share it with `Arc<Runtime>`.
Create or reuse independently configured `Agent` handles on that runtime;
clone an agent handle for concurrent requests and use distinct session IDs
for unrelated requests. Do not call `Runtime::builder().build()` per HTTP
request: process-wide keyring, event bus and subscriber ownership deliberately
refuse a second live runtime with `RuntimeError::AlreadyRunning`.

Tenants requiring distinct configuration can use `ProfileRuntime`; callers
must still follow its shared-process isolation contract. A runtime's shared
bus and process state do not provide arbitrary process isolation. Runtime
ownership, agent lifetime and session scopes should be tested by the host.

See [runtime](src/runtime/README.md), [profiles](src/profiles.rs) and
[routing](ROUTING.md).
