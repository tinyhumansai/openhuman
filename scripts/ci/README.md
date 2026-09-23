# `scripts/ci/`

Merge-gate scripts run by `.github/workflows/ci-lite.yml` (the PR lane), plus
the data files some of them read. Each script's header comment is the
authoritative spec; this is an index so a CI failure log points somewhere.

| File | Checks |
| --- | --- |
| `check-openhuman-rust-layout.mjs` (`pnpm rust:layout`) | Every file under `crates/openhuman-core/src` (except `api`, `bin`, `core`, `lib.rs`, `main.rs`, `rpc`) stays under a 750-line ceiling, with pinned legacy exceptions; forbids inline `#[cfg(test)] mod` blocks and `tests.rs`/`test.rs` filenames; and asserts every root `tests/*.rs` / `examples/*.rs` has a matching `[[test]]`/`[[example]]` entry in `crates/openhuman-core/Cargo.toml`. |
| `check-feature-forwarding.mjs` | The Tauri shell (`crates/openhuman-app/Cargo.toml`, which sets `default-features = false`) forwards exactly the gates in `product-features.txt` — no more, no less. Logic lives in `scripts/lib/feature-forwarding.mjs` (openhuman#4919). |
| `product-features.txt` / `product-features.sh` | Source of truth for the shipped desktop product's Cargo feature gates, distinct from `[features] default` (the contributor build). The `.sh` prints them comma-separated for `cargo --features "$(scripts/ci/product-features.sh)"`. |
| `check-module-pins.mjs` + `module-pin-exemptions.json` | A loadable module's registry pin (`crates/openhuman-core/src/modules/registry.rs`) and its `vendor/*` submodule pin must describe the same release (openhuman#5727). The exemptions file records known, expected drift — not a way to silence the gate. Logic lives in `scripts/lib/module-pins.mjs`. |
| `check-submodule-monotonic.mjs` | A `vendor/*` submodule pin may never move backwards onto a commit that is an ancestor of what the base branch already has. |
| `check-toolchain-image.mjs` | The CI image's toolchain, the version-pinned `dtolnay/rust-toolchain@<version>` workflow steps, and `.github/Dockerfile` all match the channel pinned in `rust-toolchain.toml`. |
| `rust-coverage.sh` | Runs the complete product-feature Rust suite under `cargo-llvm-cov`, including every core integration target and the root Rust support crates. The support crates share one invocation, the integration targets are prebuilt as one graph, and the TinyJuice host-module regression runs on the instrumented lib when `TINYJUICE_TEST_MODULE` is set. Doctests are not instrumentable and run in `rust-quality` instead. |
| `assert-coverage-presence.sh` + `coverage-presence-allowlist.txt` | Fails when an eligible Rust source file produced no lcov records at all, meaning the full product suite never compiled it (openhuman#5593). Allowlist entries must document, inline, which gate excludes the file and why that's intended. |
| `list-feature-gated-rust-tests.mjs` | Prints every file under `crates/openhuman-core/src` that carries tests behind a product feature gate (`voice`, `media`, `web3`, `meet`, `mcp`, `skills`, `flows`, `channels`, `contacts`). `ci-lite.yml`'s `rust-feature-gate-smoke` job diffs the output against a checked-in EXPECTED list so a new gated test file forces the gate-contract test filters to be reviewed (#5022). |
| `orch-ip-gate.sh` | Blocks the server-side orchestration "brain" (reasoning/wake graph, its prompts, per-agent model-selection metadata) from re-entering this open client repo. |

The complete Rust and frontend coverage suites feed the PR CI Gate's
changed-line diff-cover check (>= 80%) in `ci-lite.yml`.

## Running locally

Only `check-openhuman-rust-layout.mjs` is wired to a pnpm script
(`pnpm rust:layout`); the rest are invoked directly, e.g.
`node scripts/ci/check-feature-forwarding.mjs` or
`bash scripts/ci/orch-ip-gate.sh`, and can be run the same way locally. Every
gate here runs from `.github/workflows/ci-lite.yml`; `ci-full.yml` does not
call any of them, and only reaches `product-features.sh` indirectly through
`test-reusable.yml`. Grep `ci-lite.yml` for a script's name to see its exact
invocation and any required environment.

Long-running CI commands (the coverage lanes) go through
`scripts/ci-cancel-aware.sh`, which lives directly under `scripts/`, not here.
