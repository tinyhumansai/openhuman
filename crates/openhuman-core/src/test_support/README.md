# test_support

Test-support domain: wipe-and-reset plus read-only introspection RPCs that let E2E specs drive the in-process core between tests without restarting the process. `openhuman.test_reset` returns the running core to a fresh-install baseline; the `test_support.*` introspection RPCs let specs verify that a UI action actually flowed through to disk and to live Rust state (workspace tree, files, the `IN_FLIGHT` chat map, wallet prepared quotes). Everything here is gated behind the `/rpc` bearer token (only written in debug builds) and, for the destructive reset, an explicit `OPENHUMAN_E2E_MODE` env flag: so it is effectively unreachable in release.

## Responsibilities

- Reset persistent core state in-place to the "fresh install" baseline: no authenticated user (`active_user.toml` removed, `api_key` cleared), onboarding not completed (`onboarding_completed=false`, `chat_onboarding_completed=false`), no cron jobs. Memory v2 lives in a remote engine and is not wiped.
- Short-circuit and surface errors on any individual wipe step (partial resets are treated as worse than a clear failure).
- Gate `reset` behind `OPENHUMAN_E2E_MODE` (`1`/`true`/`TRUE`/`yes`/`YES`).
- Provide narrow, read-only introspection RPCs: resolve the active workspace root, list workspace files (depth- and count-capped), read a workspace file (lossy UTF-8, 1 MiB cap), snapshot the in-flight chat map, and snapshot the wallet prepared-quote store.
- Enforce workspace-jail path safety: reject `..` escapes, canonicalize the root (handles macOS `/var`→`/private/var` symlink), and skip symlinks during directory walks so listings can't escape the workspace.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/test_support/mod.rs` | Export-only module: declares `introspect`, `rpc`, `schemas`; re-exports `all_test_support_controller_schemas` / `all_test_support_registered_controllers`. |
| `crates/openhuman-core/src/test_support/rpc.rs` | `openhuman.test_reset` implementation: `reset()` wipes cron, config fields, and active user; returns a `ResetSummary`. Includes the `OPENHUMAN_E2E_MODE` guard. |
| `crates/openhuman-core/src/test_support/rpc_tests.rs` | Tests for the `OPENHUMAN_E2E_MODE` guard (`reset` rejects when unset; `1`/`true`/`yes` accepted). |
| `crates/openhuman-core/src/test_support/introspect.rs` | Read-only introspection RPCs: `workspace_root`, `list_workspace_files`, `read_workspace_file`, `in_flight_chats`, `wallet_prepared_quotes`, plus the `resolve_workspace_relative` path guard and the iterative `walk_dir`. |
| `crates/openhuman-core/src/test_support/introspect_tests.rs` | Tests for `resolve_workspace_relative` only: `..` traversal (missing and existing targets), leading `/` and `./` handling, and (unix) a symlink pointing out of the workspace. The RPC functions themselves have no unit tests here; E2E specs exercise them. |
| `crates/openhuman-core/src/test_support/schemas.rs` | `ControllerSchema` definitions, the registered-controller list, and `handle_*` dispatchers that delegate to `rpc`/`introspect` and serialize via `Outcome::into_cli_compatible_json`. |

## Public surface

From `mod.rs`:
- `all_test_support_controller_schemas()`: `Vec<ControllerSchema>` for the six controllers.
- `all_test_support_registered_controllers()`: `Vec<RegisteredController>` (schema + handler pairs).

From `rpc` / `introspect` (used by handlers, also `pub`):
- `rpc::reset() -> Outcome<ResetSummary>`.
- `introspect::workspace_root()`, `list_workspace_files(rel_root, max_depth)`, `read_workspace_file(rel_path, max_bytes)`, `in_flight_chats()`, `wallet_prepared_quotes()`.
- Result types: `ResetSummary`, `WorkspaceRoot`, `ListEntry`/`ListResult`, `ReadFileResult`, `InFlightEntryView`/`InFlightResult`, `PreparedQuotesResult`.

## RPC / controllers

Six controllers, registered into the global registry via `crates/openhuman-core/src/core/all.rs`:

| Namespace.function | Inputs | Purpose |
| --- | --- | --- |
| `test.reset` | none | Wipe auth, onboarding, and cron to a fresh-install baseline. Requires `OPENHUMAN_E2E_MODE`. |
| `test_support.workspace_root` | none | Return active `workspace_dir` path + existence. |
| `test_support.list_workspace_files` | `rel_root?`, `max_depth?` | Recursive listing, capped at depth 6 / 2000 entries. |
| `test_support.read_workspace_file` | `rel_path`, `max_bytes?` | Lossy-UTF-8 file read, capped at 1 MiB; rejects `..` escapes. |
| `test_support.in_flight_chats` | none | Snapshot the `IN_FLIGHT` chat map (`(client_id, thread_id)` → `request_id`). |
| `test_support.wallet_prepared_quotes` | none | Snapshot the in-memory wallet prepared-quote store. |

Note the `reset` controller uses namespace `test` (method `openhuman.test_reset`) while the introspection controllers use namespace `test_support`.

## Persistence

This module owns no state of its own. It mutates and reads state owned by other domains:
- Wipes: cron jobs (`cron::clear_all_jobs`), config fields (`onboarding_completed`, `chat_onboarding_completed`, `api_key`), and `active_user.toml` (`config::clear_active_user` under `default_root_openhuman_dir`).
- Reads: workspace files under `Config::workspace_dir`, the in-process `IN_FLIGHT` chat map, and the in-memory wallet prepared-quote store.

## Dependencies

- `crate::config`: `Config::load_or_init`/`save`, `workspace_dir`, `clear_active_user`, `default_root_openhuman_dir`; config is the source of truth for the workspace root and the fields reset wipes.
- `crate::cron`: `clear_all_jobs` to wipe scheduled jobs during reset.
- `crate::web_chat`: `in_flight_entries_for_test` to snapshot the live `IN_FLIGHT` chat map.
- `crate::web3::wallet`: `prepared_quotes_for_test` and `PreparedTransaction` to snapshot prepared quotes.
- `crate::core::all`: `ControllerFuture`, `RegisteredController` for handler wiring.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}`: controller schema types.
- `crate::core::Outcome`: the controller result type.

## Used by

- `crates/openhuman-core/src/core/all.rs`: pushes `all_test_support_registered_controllers()` into the global registry under `DomainGroup::Platform`, behind the same `#[cfg(feature = "e2e-test-support")]`. Method names come from the registry's `openhuman.{namespace}_{function}` rule. `all_test_support_controller_schemas()` has no consumer outside this module.
- `crates/openhuman-core/src/lib.rs`: declares `pub mod test_support;` behind `#[cfg(feature = "e2e-test-support")]`. The feature is off in the contributor default set (`default = [...]` in `crates/openhuman-core/Cargo.toml`) and absent from `scripts/ci/product-features.txt`, so neither a bare `cargo check` nor the shipped app compiles this module. It is turned on by the E2E build (`app/scripts/e2e-build.sh`, forwarded through `crates/openhuman-app/Cargo.toml`'s `e2e-test-support` feature) and by a dedicated CI Lite lane (`.github/workflows/ci-lite.yml`) that `cargo check`s it and runs the `test_support::introspect::` tests with `--no-default-features --features e2e-test-support`.
- Consumed at runtime by E2E specs (WDIO) calling `openhuman.test_reset` and `openhuman.test_support_*` over JSON-RPC.

## Notes / gotchas

- Double gating: `reset` is fail-closed behind both the `/rpc` bearer token and `OPENHUMAN_E2E_MODE`; introspection RPCs rely on the bearer token alone. The token file specs read is written by `crates/openhuman-app/src/core_process.rs` under `#[cfg(debug_assertions)]` only.
- In-process reset: the core process is not restarted; specs reload the webview afterward so the renderer also starts blank.
- Add new persistent state to `reset`. Per the module docstring, any new domain that survives `test_reset` is a leak that lets specs interfere, so extend `rpc::reset` when adding persistent state.
- Path-jail ordering: `resolve_workspace_relative` rejects `..` (and root/prefix components) lexically before touching the filesystem; the `canonicalize` + `starts_with` check underneath only catches symlink escapes and used to fail open for non-existent targets (openhuman#6085). It canonicalizes the root first (macOS `/var`→`/private/var`); `walk_dir` uses `symlink_metadata` and skips symlinks so listings can't follow links out of the workspace.
- CI runs only the `introspect` tests (the CI Lite lane filters to `test_support::introspect::`). Run `cargo test -p openhuman --features e2e-test-support --lib -- test_support::` locally to cover both files.
- `read_workspace_file` byte accounting: `returned_bytes` is the raw byte count before lossy UTF-8 conversion (which substitutes U+FFFD and can change length) so specs can assert byte-accurate truncation.
- `list_workspace_files` schema omits `entries` for brevity, but the runtime payload includes them. `max_depth` defaults to 2, clamped to 6; entry cap is 2000.
