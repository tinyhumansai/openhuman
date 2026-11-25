# Epic 7964 final acceptance receipts (2026-10-24)

Integrated revision: `epic/7964-channel-v2` at 254e0ca0df (senamakel/openhuman#662 plus the docs commit). Vendor pins: TinyChannels 5599faba,
TinyAgents 0a0e35491, TinyVoice 321014b, TinyFlows 328aecfb, TinyStorageDrivers 280f39f6, other modules at the merge-base pins.

| Gate | Command | Result |
| --- | --- | --- |
| Channels unit suite | `RUST_MIN_STACK=16777216 cargo test -p openhuman --lib -- channels:: --test-threads=1` | exit 0, 278 passed, 0 failed |
| Turn-state regression | `channels::routes::tests::telegram::handle_runtime_command_telegram_new_status_and_sessions_round_trip` | failed `Turn: idle` before, passes after |
| Clippy | `cargo clippy -p openhuman --all-targets -- -D warnings` | exit 0 |
| Layout | `pnpm rust:layout` | exit 0 |
| Docs | `pnpm docs:generate`, `pnpm docs:check`, `pnpm docs:test` | exit 0, 0, 0 (20/20 node tests) |
| web_chat | `cargo test -p openhuman --lib --features <core product features> -- web_chat --test-threads=1` | exit 0, 305 passed |
| JSON-RPC E2E | `bash scripts/test-rust-e2e.sh --suite json_rpc_e2e` | exit 1. Keep-going run: 101 passed, 3 failed |
| Coverage | `OUT=target/135-final/coverage/lcov-core.info OH_COV_TUI=1 bash scripts/ci/rust-coverage.sh` | exit 1 (12 suites failed, lcov produced) |
| Changed-line coverage | `bash scripts/ci/self-hosted/diff-cover.sh target/135-final/coverage 6e99ddca4d88832787913a3e919be68099cc02cd ...` | exit 1: 35% of 13968 changed lines (9032 missing) |

## Why the last three gates are red

Both root causes sit outside the TinyChannels leaves and need the upstream-main merge into this epic (a later stage):

1. TinyFlows host and module disagree. The epic's `cron/bus_host.rs` serves the old `Deliver(job, run_id, attempt)` callback and lacks `Send`, while the
   TinyFlows module (every release at or above 0.8.24, and the vendored 328aecfb) calls `Deliver(projection)`/`Send`. The registry still pins the v0.8.23 release,
   which lacks `RunJobNow` that the epic's scheduler calls. Failing: `json_rpc_cron_add_run_list_round_trip_over_the_tinyflows_module`,
   `json_rpc_cron_origin_delivery_lands_in_the_asking_thread`, and about 30 `cron::`/`flows::` unit tests. Upstream main carries the matching host change in a4789ec18c.
2. Release-pinned modules differ from the vendored sources: `json_rpc_modules_browser_forget_sites_removes_learned_sites` (TinyComputer v0.14.0),
   and the registry-manifest tests for TinyBox, TinyMCP and TinyChannels.

Unit suites run in parallel share process globals and fail without `--test-threads=1` (the CI scripts run them serially).
The coverage run also executed the 12 failing suites listed in the run log, so their lines are unmeasured.
