# Feature design workflow (new capabilities)

Follow this order so behavior is **specified**, **proven in Rust**, **proven over RPC**, then **surfaced in the UI** with matching tests.

1. **Specify against the current codebase** — Ground the design in **existing** domains, controller/registry patterns, and JSON-RPC naming (`openhuman.<namespace>_<function>`). Reuse or extend documented flows in [`ARCHITECTURE.md`](ARCHITECTURE.md) and sibling guides; avoid parallel architectures.
2. **Implement in Rust** — Add domain logic under `src/openhuman/<domain>/`, wire **schemas + registered handlers** into the shared registry, and land **unit tests** in the crate (`cargo test -p openhuman`, focused modules) until the feature is correct in isolation.
3. **JSON-RPC E2E** — Add or extend **integration-style tests** that call the real HTTP JSON-RPC surface (e.g. [`tests/json_rpc_e2e.rs`](../tests/json_rpc_e2e.rs), mock backend / [`scripts/test-rust-with-mock.sh`](../scripts/test-rust-with-mock.sh) as appropriate) so methods, params, and outcomes match what the UI will call.
4. **UI in the Tauri app** — Build **React** screens, state, and **`core_rpc_relay` / `coreRpcClient`** usage in `app/`; keep **business rules** in the core, not duplicated in the shell.
5. **App unit tests** — Cover components, hooks, and clients with **Vitest** (`yarn test` / `yarn test:unit` in `app/`).
6. **App E2E** — Add **desktop E2E** specs where the feature is user-visible (`yarn test:e2e*`, isolated workspace — see [`TESTING.md`](TESTING.md)) so the full stack (UI → Tauri → sidecar) behaves as intended.

**Capability catalog** — When a change adds, removes, renames, relocates, or materially changes a user-facing feature, update **`src/openhuman/about_app/`** in the same work so the runtime capability catalog remains the source of truth for what the app can do.

**Debug logging (throughout)** — Add **lots of development-oriented logging** as you build, not as an afterthought. In **Rust**, use `log` / `tracing` at **`debug`** or **`trace`** on RPC entry and exit, error paths, state transitions, and any branch that is hard to infer from tests alone. In **`app/`**, follow existing patterns (e.g. the **`debug`** npm package with a **namespace** per area) plus **dev-only** detail where useful. Prefer **grep-friendly prefixes** (`[feature]`, domain name, or JSON-RPC method) so terminal output from **sidecar**, **Tauri**, and **WebView** can be correlated during `yarn dev` / `tauri dev`. **Never** log secrets, raw JWTs, API keys, or full PII—redact or omit.

**Planning rule:** When scoping a feature, define the **E2E scenarios (core RPC + app)** up front. Those scenarios should **cover the full intended scope**—happy paths, failure modes, auth or policy gates, and regressions you care about. If a scenario is not testable end-to-end, the spec is incomplete or the cut is too large; split or add harness support first.

## Debug logging rule

- **Default to verbose diagnostics on new/changed flows**: Add substantial, development-oriented logs while implementing features or fixes so issues are easy to trace end-to-end.
- **Log critical checkpoints**: Include logs at entry/exit points, branch decisions, external calls, retries/timeouts, state transitions, and error handling paths.
- **Use structured, grep-friendly context**: Prefer stable prefixes (for example `[domain]`, `[rpc]`, `[ui-flow]`) and include correlation fields such as request IDs, method names, and entity IDs when available.
- **Platform conventions**: In Rust, use `log` / `tracing` at `debug` or `trace`; in `app/`, use namespaced `debug` logs and dev-only detail as needed.
- **Keep logs safe**: Never log secrets or sensitive payloads (API keys, JWTs, credentials, full PII). Redact or omit sensitive fields.
- **Treat debuggability as a deliverable**: Changes lacking sufficient logging for diagnosis are incomplete and should be updated before handoff.

## Controller migration checklist

- `src/openhuman/<domain>/mod.rs`: keep export-focused, add `mod schemas;` and re-export:
  - `all_controller_schemas as all_<domain>_controller_schemas`
  - `all_registered_controllers as all_<domain>_registered_controllers`
- `src/openhuman/<domain>/schemas.rs` must define:
  - `schemas(function: &str) -> ControllerSchema`
  - `all_controller_schemas() -> Vec<ControllerSchema>`
  - `all_registered_controllers() -> Vec<RegisteredController>`
  - domain handler fns `fn handle_*(_: Map<String, Value>) -> ControllerFuture`
- Handlers should delegate to existing domain `rpc.rs` functions during migration.
- Wire domain exports into `src/core/all.rs` for both declared schemas and registered handlers.
- Keep adapters generic: do not add domain-specific logic to `src/core/cli.rs` or `src/core/jsonrpc.rs`.
- Remove migrated method branches from `src/rpc/dispatch.rs` once registry coverage is in place.

## Platform notes

- **Vendored CEF-aware `tauri-cli` (required)**: The default runtime is **CEF**, and only the **vendored** `tauri-cli` at `app/src-tauri/vendor/tauri-cef/crates/tauri-cli` knows how to bundle the Chromium Embedded Framework into the app's `Contents/Frameworks/`. The stock `@tauri-apps/cli` / upstream `cargo-tauri` produces a bundle **without** `Frameworks/` and the app panics at startup inside `cef::library_loader::LibraryLoader::new` with `No such file or directory`. `yarn dev:app` (and every other `cargo tauri` script in `app/package.json`) now calls **`yarn tauri:ensure`** which runs [`../scripts/ensure-tauri-cli.sh`](../scripts/ensure-tauri-cli.sh) to install the vendored CLI into `~/.cargo/bin/cargo-tauri` on first use. If you ever install a different `tauri-cli` over it (e.g. `npm i -g @tauri-apps/cli`) you'll need to re-run the ensure script / `cargo install --locked --path app/src-tauri/vendor/tauri-cef/crates/tauri-cli`.
- **macOS deep links**: Often require a built **`.app`** bundle; not only `tauri dev`. See [`telegram-login-desktop.md`](telegram-login-desktop.md) if applicable.
- **`window.__TAURI__`**: Not assumed at module load; guard Tauri usage accordingly.
- **Core sidecar**: Must be staged/built so `core_rpc` can reach the `openhuman` binary (see `scripts/stage-core-sidecar.mjs`).
