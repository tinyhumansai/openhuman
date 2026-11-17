# Epic 7964: upstream/main merged into epic/7964-channel-v2 (2026-10-24/25)

Receipts for `epic-7964-upstream-merge` (merge commits of `upstream/main` up to `51e12c53` /
current tip at merge time). Raw logs live in the session scratchpad; the numbers below are
copied from them. Nothing was pushed to `tinyhumansai/*`.

## Fork epic tips (all merged with their upstream default branch, merge commits, no rebase)

| Repo | Fork epic tip | Notes |
| --- | --- | --- |
| senamakel/tinychannels | `ec6bdf9d` | upstream main 337d7137 merged; later PRs: context_compacting event (wire contract 33), typed inbound preparation + turn-lease cap, final-message handle end, inbound voice ingress, PR #130 (outbound voice replies + adapter typing, merged) |
| senamakel/tinyagents | `3020f2e2` | upstream main 7346fabd merged; restored `run_progress_ledger` export and `TurnRunnerConfig::cancel_grace` |
| senamakel/tinyflows | `5953af94` | upstream main merged (clean) |
| senamakel/tinyvoice | `fca28c70` | upstream main merged (clean) |

Every other gitlink is upstream's pin; `vendor/tinymemes` is gone with upstream; `.gitmodules` is upstream's.

## Gates (product features, `--test-threads=1` for unit suites)

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy -p openhuman --all-targets --features <product> -- -D warnings` | exit 0 |
| `cargo clippy -p openhuman -p openhuman-cli -p openhuman-tinyhumans -- -D warnings` | exit 0 |
| `pnpm --filter openhuman-app rust:clippy` | exit 0 |
| `pnpm rust:layout` | exit 0 |
| `channels::` | 338 passed, 0 failed |
| `web_chat::` | 310 passed, 0 failed |
| `cron::` | 330 passed, 0 failed (needs `RUST_MIN_STACK=16777216`) |
| `flows::` | 518 passed, 1 failed: `flows_build_copilot_toolset_unhides_the_live_run_tools`, fails identically on upstream/main (pre-existing) |
| JSON-RPC e2e (`json_rpc_*_e2e`, 10 suites) | 9 exit 0; `json_rpc_auth_security_e2e` (telegram bot token) exit 101, also fails on upstream/main (module staging / pin skew) |
| `pnpm docs:generate` / `docs:check` / `docs:test` | exit 0 / 0 / 0 |
| `pnpm test` (vitest) | 9306 passed; `src/lib/attachments.test.ts` gzip case fails on Node 22, also on upstream/main |
| `pnpm typecheck`, `lint`, `format:check`, `i18n:check`, `i18n:english:check`, `chat-wire:check` | exit 0 |
| Coverage lane (`OH_COV_RUNNER=nextest scripts/ci/rust-coverage.sh`) + `diff-cover` vs upstream/main | 86% of 11048 changed lines (threshold 80) |

## Known red / not pre-existing

- `modules::registry::tests::release_manifest_tests::tinychannels_registry_matches_scoped_runtime_release`:
  the registry pins the published TinyChannels release (contract 23); the fork bus is contract 33.
  Needs a release + re-pin; pins were deliberately not touched.
- RESOLVED (senamakel/tinychannels#130, merged `ec6bdf9d`): outbound mirrored voice replies and module-channel
  typing. `native_channel_session_restart_e2e` (3 tests, including the voice-note mirror TTS test) passes against the
  staged fork module; the host drives `StartTyping` / `StartTypingRefresh` again and
  `channels::tests::runtime_dispatch` now asserts typing precedes the reply.
- Staged-module run (`node scripts/modules/build.mjs tinychannels --out <dir>` + `OPENHUMAN_TEST_MODULE_SOURCE_DIR`): `native_channel_session_restart_e2e` 3/3, `saas_mode_e2e` 19/19, `saas_profile_isolation_e2e` 4/4, `components_e2e` 1/1, `embedded_server_shutdown_e2e` 1/1. `json_rpc_auth_security_e2e::telegram_bot_token_connect...` still needs the registry-pinned release (it rejects a source override), so it stays red until the post-merge re-pin.
- Debounce: a command arriving inside the debounce window is pinned by `inbound_debounce::tests::non_mergeable_item_seals_pending_batch_and_runs_behind_it` (and `inbound_batch_tests`: commands are never mergeable). A dispatch-loop level test was attempted and dropped: the loop does not drain with a slash command in the stub runtime.
- (earlier note) Several saas/embed e2e suites need the fork TinyChannels module staged
  (`node scripts/modules/build.mjs tinychannels --out <dir> --print-env`, plus
  `OPENHUMAN_TEST_MODULE_SOURCE_DIR`); with the published module they fail until re-pin. Two timing
  flakes (`lease fence ... expired before renewal`, `inline_permissions`) were also seen on upstream/main.
- Pre-existing on upstream/main: `cargo fmt --check` drift in two test files (fixed here), two clippy lints
  (fixed here), 6 files with no coverage records (undeclared upstream modules), 24 nextest failures.

## Decisions taken in the merge

- Upstream per-conversation inbound debounce was ported onto the epic dispatch (debouncer only delays/merges
  intake; the TinyAgents `TurnRunner` still owns queueing).
- Voice-note audio now lives in the storage port's BlobStore (the storage-bypass ratchet forbids new fs writers).
- Storage-schema lint exempts TinyChannels' declared legacy migration source collection names.
- `app/.prettierignore` lists the generated `threadWire.generated.ts` like `chatWire.generated.ts`.
- The wiki patch (`docs/handoffs/7964-wiki-handoff.patch`) stays deferred to the final upstream stage.
