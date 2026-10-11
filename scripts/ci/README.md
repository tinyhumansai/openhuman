# `scripts/ci/`

Merge-gate scripts run by `.github/workflows/ci-lite.yml` (the PR lane), plus
the data files some of them read. Each script's header comment is the
authoritative spec; this is an index so a CI failure log points somewhere.

| File | Checks |
| --- | --- |
| [`check-openhuman-rust-layout.mjs`](check-openhuman-rust-layout.mjs) (`pnpm rust:layout`) | Every file under `crates/openhuman-core/src` stays under a 750-line ceiling, warning (without failing) past 725; oversized files are pinned at their exact size in `LEGACY_LIMIT_ENTRIES`, once each, and a pinned file that shrinks must lower or drop its pin; forbids inline `#[cfg(test)] mod` blocks and `tests.rs`/`test.rs` filenames; and asserts every root `tests/*.rs` / `examples/*.rs` has a matching `[[test]]`/`[[example]]` entry in `crates/openhuman-cli/Cargo.toml`, and that the core manifest declares no target tables. |
| [`check-feature-forwarding.mjs`](check-feature-forwarding.mjs) | The Tauri shell ([`crates/openhuman-app/Cargo.toml`](../../crates/openhuman-app/Cargo.toml), which sets `default-features = false`) forwards exactly the gates in `product-features.txt`, no more, no less, and the library chain that re-declares the core's gates (`openhuman-embed` → `openhuman-tinyhumans` → `openhuman-rpc` → `openhuman-cli` / `openhuman-tui`) forwards every one of them, by target and not just by name. The shell's list is read from its `openhuman-rpc` dependency. Logic lives in [`scripts/lib/feature-forwarding.mjs`](../lib/feature-forwarding.mjs) (openhuman#4919, openhuman#6364). |
| [`check-crate-chain.mjs`](check-crate-chain.mjs) | The crate chain holds: each OpenHuman crate's normal dependencies name only the layer below it (core: none; embed: core; tinyhumans: embed; rpc: tinyhumans; app/cli/tui: rpc), and host `src/` never names `__host`, `core_host` or `openhuman_core::`. Dev-dependencies are exempt. Runs as part of `pnpm rust:layout`. |
| [`product-features.txt`](product-features.txt) / [`product-features.sh`](product-features.sh) | Source of truth for the shipped desktop product's Cargo feature gates, distinct from `[features] default` (the contributor build). The `.sh` prints them comma-separated for `cargo --features "$(scripts/ci/product-features.sh)"`. |
| [`check-module-pins.mjs`](check-module-pins.mjs) + [`module-pin-exemptions.json`](module-pin-exemptions.json) | A loadable module's registry pin ([`crates/openhuman-core/src/modules/registry.rs`](../../crates/openhuman-core/src/modules/registry.rs)) and its `vendor/*` submodule pin must describe the same release (openhuman#5727). The exemptions file records known, expected drift, not a way to silence the gate. Logic lives in [`scripts/lib/module-pins.mjs`](../lib/module-pins.mjs). |
| [`check-submodule-monotonic.mjs`](check-submodule-monotonic.mjs) | A `vendor/*` submodule pin may never move backwards onto a commit that is an ancestor of what the base branch already has. |
| [`check-toolchain-image.mjs`](check-toolchain-image.mjs) | The CI image's toolchain, the version-pinned `dtolnay/rust-toolchain@<version>` workflow steps, and [`.github/Dockerfile`](../../.github/Dockerfile) all match the channel pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml). |
| [`rust-coverage.sh`](rust-coverage.sh) | Runs the complete product-feature Rust suite under `cargo-llvm-cov`, including every core integration target and the root Rust support crates. |
| [`assert-coverage-presence.sh`](assert-coverage-presence.sh) + [`coverage-presence-allowlist.txt`](coverage-presence-allowlist.txt) | Fails when an eligible Rust source file produced no lcov records at all, meaning the full product suite never compiled it (openhuman#5593). Allowlist entries must document, inline, which gate excludes the file and why that's intended. |
| [`list-feature-gated-rust-tests.mjs`](list-feature-gated-rust-tests.mjs) | Prints every file under `crates/openhuman-core/src` that carries tests behind a product feature gate (`voice`, `media`, `web3`, `meet`, `mcp`, `skills`, `flows`, `channels`, `contacts`). `ci-lite.yml`'s `rust-feature-gate-smoke` job diffs the output against a checked-in EXPECTED list so a new gated test file forces the gate-contract test filters to be reviewed (#5022). |
| [`check-saas-ambient.mjs`](check-saas-ambient.mjs) + [`saas-ambient-baseline.json`](saas-ambient-baseline.json) (`pnpm saas:ambient`) | Ratchets code in `crates/openhuman-core/src` that reaches past a SaaS user agent's `CoreContext`: bare `tokio::spawn` / `spawn_blocking` / `std::thread::spawn` (use `core::runtime::spawn_scoped`), direct `Config::load_or_init`, `env::set_var` / `remove_var`, and `home_dir()`. Each existing site is baselined exactly; a new one fails, and a fixed one must be removed with `--write-baseline`. |
| [`orch-ip-gate.sh`](orch-ip-gate.sh) | Blocks the server-side orchestration "brain" (reasoning/wake graph, its prompts, per-agent model-selection metadata) from re-entering this open client repo. |

The complete Rust and frontend coverage suites feed the PR CI Gate's
changed-line diff-cover check (>= 80%) in `ci-lite.yml`.

## Running locally

The module boundary audit (`pnpm rust:module-boundaries`) resolves the root and
excluded desktop host with all features, traversing normal and build edges on
all platforms. It also resolves each bus contract independently with default
and all features, outside the owning module workspace, to avoid feature
unification with implementation crates. It runs in the Rust scripts lane.
Temporary exceptions in [`module-boundaries.json`](module-boundaries.json) must
have reasons; unused exceptions fail the audit. A passing transitional audit is
not a completed migration. `pnpm rust:module-boundaries:complete` additionally
rejects all exceptions and pending contracts. Cargo failure is an audit failure.
Generated probes and lockfiles stay in the checkout's `target/module-boundaries/`.

`check-openhuman-rust-layout.mjs` and `check-crate-chain.mjs` run through
`pnpm rust:layout`. The module-boundary audit has the two pnpm commands above,
and the ambient-access ratchet runs through `pnpm saas:ambient`. Other gates
are invoked directly, e.g.
`node scripts/ci/check-feature-forwarding.mjs` or
`bash scripts/ci/orch-ip-gate.sh`, and can be run the same way locally. Every
gate here runs from `.github/workflows/ci-lite.yml`; `ci-full.yml` does not
call any of them, and only reaches `product-features.sh` indirectly through
`test-reusable.yml`. Grep `ci-lite.yml` for a script's name to see its exact
invocation and any required environment.

Long-running CI commands (the coverage lanes) go through
[`scripts/ci-cancel-aware.sh`](../ci-cancel-aware.sh), which lives directly under `scripts/`, not here.

## Further reading

- [`scripts/README.md`](../README.md), the map of the rest of `scripts/`.
- [Testing strategy](../../gitbooks/developing/testing-strategy.md) for how the coverage gate fits the test layers.
- [Loadable modules](../../gitbooks/developing/loadable-modules.md), which the module-pin gates protect.
- [`AGENTS.md`](../../AGENTS.md) for the layout and feature-forwarding rules these scripts enforce.
