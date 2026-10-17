# test_support

RPCs that let the desktop E2E specs control and inspect a running core
without restarting it. `openhuman.test_reset` puts the core back to a
fresh-install baseline between specs, and the `test_support.*` methods are
read-only probes that let a spec confirm a UI action reached disk and live
Rust state (the workspace tree, file contents, the in-flight chat map, the
wallet's prepared quotes).

None of this ships. The module only compiles with the `e2e-test-support`
Cargo feature, which is off in the contributor default set and absent from
`scripts/ci/product-features.txt`.

## How it works

The E2E build turns the feature on, the registry picks the controllers up,
and the WDIO runner calls them over JSON-RPC with the debug-only bearer.

```text
app/scripts/e2e-build.sh
   pnpm tauri build --debug --features e2e-test-support
        |
        v
crates/openhuman-app  e2e-test-support = ["openhuman_core/e2e-test-support"]
        |
        v
lib.rs              #[cfg(feature = "e2e-test-support")] pub mod test_support;
core/all.rs         push(DomainGroup::Platform,
                         all_test_support_registered_controllers())
        |
        v
running core (debug build)
   core_process.rs writes the /rpc bearer to
   ${tmpdir}/openhuman-e2e-rpc-token (debug_assertions only, mode 0600)
        ^
        |  JSON-RPC: openhuman.test_reset, openhuman.test_support_*
        |
WDIO specs (app/test/e2e/specs/, helpers/reset-app.ts)
```

### Reset

`rpc::reset()` refuses to run unless `OPENHUMAN_E2E_MODE` is `1`, `true`,
`TRUE`, `yes` or `YES`. It then loads the config and wipes, in order:

1. Cron jobs (`cron::clear_all_jobs`), so the post-onboarding seed recreates
   `morning_briefing`.
2. Config fields: `onboarding_completed` and `chat_onboarding_completed` set
   to `false`, `api_key` cleared, then `Config::save`.
3. The active user: `config::clear_active_user` removes `active_user.toml`
   under `default_root_openhuman_dir()`.

Any failed step returns an error immediately. A partial reset is treated as
worse than a clear failure, because it lets later specs pass on contaminated
state. The result is a `ResetSummary` (`cron_jobs_removed`,
`onboarding_was_completed`, `api_key_was_set`, `active_user_cleared`). Memory
lives in a remote engine and is not wiped.

The core process keeps running. Specs reload the webview after the call so
the renderer also starts blank.

### Introspection

The [`introspect.rs`](./introspect.rs) functions read state and change nothing.
`workspace_root` reports `Config::workspace_dir` and whether it exists.
`list_workspace_files` walks the tree with an explicit stack (no async
recursion), defaulting to depth 2, clamped to 6, and stopping at 2000 entries
with `truncated: true`. `read_workspace_file` reads up to `max_bytes`
(capped at 1 MiB) and converts it with lossy UTF-8; `returned_bytes` is the
raw byte count before conversion, so specs can assert truncation exactly.
`in_flight_chats` snapshots the web chat `IN_FLIGHT` map
(`web_chat::in_flight_entries_for_test`), and `wallet_prepared_quotes`
snapshots the in-memory prepared-quote store
(`web3::wallet::prepared_quotes_for_test`).

Every path argument goes through `resolve_workspace_relative`, which keeps
reads inside the workspace:

1. A lexical component walk rejects `..` and any root or prefix component
   (`C:\`, `\\?\`) before touching the filesystem. `./` is allowed.
2. The workspace root is canonicalized (on macOS `/var` resolves to
   `/private/var`), the relative path is joined onto it, and the candidate is
   canonicalized and checked with `starts_with`. This second step catches a
   symlink pointing out of the workspace, which the lexical walk cannot see.

The lexical step matters because `canonicalize` fails on a missing path, and
without it a `..` path to a nonexistent target used to pass the prefix check
(openhuman#6085). `walk_dir` uses `symlink_metadata` and skips symlinks, so
listings never follow a link out of the workspace.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Declares `introspect`, `rpc` and `schemas`; re-exports `all_test_support_controller_schemas` and `all_test_support_registered_controllers`. |
| [`rpc.rs`](./rpc.rs) | `reset()`, the `OPENHUMAN_E2E_MODE` guard and `ResetSummary`. |
| [`introspect.rs`](./introspect.rs) | The read-only probes, their result types, `resolve_workspace_relative` and `walk_dir`. |
| [`schemas.rs`](./schemas.rs) | Controller schemas and thin `handle_*` functions that call `rpc` or `introspect` and serialize with `Outcome::into_cli_compatible_json`. |

## Key types and entry points

- `rpc::reset() -> Result<Outcome<ResetSummary>, String>` ([`rpc.rs`](./rpc.rs)).
- `introspect::workspace_root`, `list_workspace_files(rel_root, max_depth)`,
  `read_workspace_file(rel_path, max_bytes)`, `in_flight_chats`,
  `wallet_prepared_quotes` (`introspect.rs`).
- Result types: `WorkspaceRoot`, `ListEntry` and `ListResult`,
  `ReadFileResult`, `InFlightEntryView` and `InFlightResult`,
  `PreparedQuotesResult` (`introspect.rs`).
- `all_test_support_registered_controllers()` ([`mod.rs`](./mod.rs)), which
  `core/all.rs` pushes under `DomainGroup::Platform`.
  `all_test_support_controller_schemas()` has no consumer outside this module.

## RPC / CLI surface

| Method | Inputs | Purpose |
| --- | --- | --- |
| `test.reset` (wire `openhuman.test_reset`) | none | Wipe cron, onboarding flags, `api_key` and the active user. Requires `OPENHUMAN_E2E_MODE`. |
| `test_support.workspace_root` | none | Active `workspace_dir` and whether it exists. |
| `test_support.list_workspace_files` | `rel_root?`, `max_depth?` | Recursive listing, depth 2 by default, max 6, at most 2000 entries. |
| `test_support.read_workspace_file` | `rel_path`, `max_bytes?` | Lossy UTF-8 read, capped at 1 MiB; rejects paths that leave the workspace. |
| `test_support.in_flight_chats` | none | Snapshot of `IN_FLIGHT` (`(client_id, thread_id)` to `request_id`). |
| `test_support.wallet_prepared_quotes` | none | Snapshot of the in-memory prepared-quote store. |

The reset uses namespace `test`; the probes use `test_support`. Wire names
follow the registry's `openhuman.{namespace}_{function}` rule.

## Boundaries

This module owns no state. It writes to state owned by `config` (onboarding
flags, `api_key`, `active_user.toml`) and `cron` (jobs), and reads state owned
by `config` (workspace root), `web_chat` (`IN_FLIGHT`) and `web3::wallet`
(prepared quotes). Those domains expose the `*_for_test` accessors it calls.

The E2E runner, its helpers and the token-file reader live under
`app/test/e2e/`. The bearer file is written by
`crates/openhuman-app/src/core_process.rs`.

## Gotchas

- Reset is gated twice: the `/rpc` bearer and `OPENHUMAN_E2E_MODE`. The
  probes rely on the bearer alone, which is only exposed to the test runner
  in debug builds. Both sit behind the Cargo feature.
- When a domain adds persistent state, extend `rpc::reset` to wipe it. State
  that survives `test_reset` lets specs interfere with each other.
- The `list_workspace_files` schema leaves `entries` out of its declared
  outputs for brevity, but the runtime payload includes them.
- `scripts/ci/assert-coverage-presence.sh` lists this folder as uncovered by
  design, because the default coverage build does not enable the feature.

## Tests

[`rpc_tests.rs`](./rpc_tests.rs) covers the `OPENHUMAN_E2E_MODE` guard. [`introspect_tests.rs`](./introspect_tests.rs)
covers `resolve_workspace_relative` (`..` with missing and existing targets,
leading `/` and `./`, and on Unix a symlink out of the workspace); the RPCs
themselves are exercised by the E2E specs.

The feature must be enabled to compile any of it:

```bash
cargo test -p openhuman --no-default-features --features e2e-test-support --lib -- test_support::
```

CI runs only the introspect tests, in the `gate-contract-tests-features` lane
of `scripts/ci/self-hosted/lanes-plan.mjs`
(`--features mcp,e2e-test-support ... test_support::introspect::`).

## Further reading

- [Testing strategy](../../../../docs/gitbooks/en/developing/testing-strategy.md)
- [Integration test layout](../../../../tests/README.md)
