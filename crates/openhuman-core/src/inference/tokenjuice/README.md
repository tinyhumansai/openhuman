# OpenHuman TokenJuice Adapter

TinyJuice ships as a separately released TinyBus module. OpenHuman depends on
its `tinyjuice-bus` contracts and `tinyjuice-host` tool adapters. Compression,
cached retrieval, queries and HTML extraction execute inside the module. This
directory owns configuration, model callbacks, dispatch context, artifact
authorization and savings attribution.

Token compression is one of the ways OpenHuman keeps a turn cheap and fast
(see [docs/gitbooks/en/developing/performance.md](../../../../../docs/gitbooks/en/developing/performance.md)):
compressing tool output before it reaches the model shrinks the prompt
without the model ever seeing less information than it needs. See
[docs/gitbooks/en/features/token-compression.md](../../../../../docs/gitbooks/en/features/token-compression.md)
for the user-facing feature page.

OpenHuman-owned files:

| Path | Role |
| --- | --- |
| [`mod.rs`](./mod.rs) | TinyBus calls (through `tinyjuice_bus::names::methods` constants), config installation, pass-through fallback, and savings wiring. |
| [`schemas.rs`](./schemas.rs) | JSON-RPC controller schemas and handlers. |
| [`config_patch.rs`](./config_patch.rs) | Partial update shape for the `[tokenjuice]` config block. |
| `tinyjuice-host::generate` | Shared turn-bound, single-use summary callback tickets; OpenHuman still registers the callback bound to its active model turn. |
| `mod.rs::repl_tools_for*` | OpenHuman's query-source adapter, which binds TinyJuice's shared REPL tools to the active `DispatchCtx`, module configuration and artifact directory. Tool declarations, artifact path checks and execution are owned by `tinyjuice-host`. |
| `tinyjuice-host::retrieve_tool` | Shared retrieval tool implementation and declaration; OpenHuman supplies the callback that reads from the module CCR store. |
| Plain text | Compressed with TinyJuice's deterministic TextCrusher. |
| [`savings.rs`](./savings.rs) | Thin adapter: attribution model and `agent::cost` price per input token; the module (`RecordSavings`, `SavingsStats`, `ResetSavings`) does the aggregate math and persists the `juice.savings` document in its storage (a legacy `state/tokenjuice_savings.json` is imported once). |

TinyJuice-owned engine pieces:

| TinyJuice repository path | Role |
| --- | --- |
| `src/compress.rs` | Content router entry point. |
| `src/compressors/` | JSON, code, log, search, diff, HTML, plain-text, and generic compressors. |
| `src/cache/` | CCR store, retrieval markers, disk tier, ranged retrieval helpers. |
| `src/rules/` | Rule loader/compiler and embedded rule table. |
| `src/vendor/rules/*.json` | Vendored upstream rule JSON files. |
| `src/detect/`, `text/`, `tokens.rs`, `types.rs` | Detection, text helpers, token estimates, public types. |

## Module boundary

`tinyjuice-bus` owns the serialized request/result types, schemas and tool
declarations. The host's `juice_find`, `juice_extract` and `juice_summarize`
adapters send `Query` requests with opaque CCR handles. They never fetch the
original to run a local query. Artifact queries retain OpenHuman's traversal,
canonical containment and size checks, then send authorized content without
a filesystem path.

`web_fetch` uses TinyTools' asynchronous HTML adapter: `Detect` and
`ExtractHtml` execute in the module independently of the compression setting.
An unavailable module fails that affected operation with a sanitized terminal
report. Optional output compaction preserves its existing disclosed pass-through
behavior. There is no linked implementation fallback.

## Wiring

- `mod.rs::client` uses the shared `ModuleClient` and process-wide lazy loader.
  Explicit configuration and bundled artifact discovery remain host-owned.
  Builds without `modules` return an unavailable error.
- Controllers are registered from `crate::inference::tokenjuice::all_tokenjuice_registered_controllers()`,
  called by [`core/all.rs`](../../core/all.rs).
- `tools/ops.rs` constructs TinyJuice's
  `tinyjuice_host::retrieve_tool::RetrieveToolOutputTool` with the host's
  module-CCR lookup callback in the agent tool catalog (it is not re-exported
  through [`tools/mod.rs`](../../tools/mod.rs)) and
  treats every `RECOVERY_TOOL_NAMES` entry as a recovery tool. The registered
  tool name is `tinyjuice_bus::tools::RETRIEVE` (`"juice_retrieve"`);
  `"tokenjuice_retrieve"` and `"retrieve_tool_output"` are recognized aliases
  only. Only
  `RECOVERY_TOOL_VISIBLE` (the live tool) is force-added to a curated
  `ToolScope::Named` belt (`session_host/builder/mod.rs::ensure_recovery_tool_visible`);
  the aliases stay registered for transcript replay but off the wire. It is
  added when compaction is on or the agent's results can be summarized
  (`summarizes_tool_output`), since a summary's footer names it too.
- Handle mode: `install_request` sets `CompressOptions.repl_handle` from
  `repl_handle_active(config)`,
  which needs `context.compaction_enabled`, `router_enabled`, `ccr_enabled` and
  `repl_handle_enabled`. `tools/ops.rs` registers the REPL tools under the same
  test, `session_host/builder/mod.rs::ensure_repl_tools_visible` adds them to a
  curated belt, and `middleware/tool_output.rs::is_compaction_exempt` keeps
  their answers from being stored behind a second handle (their own
  `max_result_size_chars` still caps them). The additive `Query` member carries the typed operation; existing `Repl`
  arity is unchanged.
- Durable CCR uses the released `InstallRequest.ccr_durable` option; legacy
  files are imported from the workspace before module installation.
- Contract crate: `tinyjuice-bus` ([`vendor/tinyjuice/crates/tinyjuice-bus`](../../../../../vendor/tinyjuice/crates/tinyjuice-bus/),
  path dependency in [`crates/openhuman-core/Cargo.toml`](../../../Cargo.toml)).

`ccr_disk_enabled` projects to the bus's `ccr_durable` flag. The module's
durable CCR data uses the host-granted storage schema; the previous
`.tokenjuice/ccr` directory is read only by the host's guarded legacy import
before installation and is never sent over the bus. `ccr_ttl_secs` remains a
separate module cache retention setting.

## Further reading

- [Parent module README](../README.md)
- [tinyjuice](../../../../../vendor/tinyjuice/README.md)

Contract types are imported directly from `tinyjuice_bus::types` and `tinyjuice_bus::wire`; the adapter keeps serialization compatibility tests in `mod_contract_tests.rs`.

## `[components] juice`

`[components] juice = false` (or `OPENHUMAN_COMPONENT_JUICE=0`) removes the
whole module from a core: `modules::ensure_loaded("tinyjuice")` answers
`COMPONENT_DISABLED:juice` without downloading or loading anything, the `MlHost`
callback is not served, `juice_*` / `retrieve_tool_output` tools are not
registered, the tool-output middleware runs with compaction and the summary
stage off (per-tool cap and byte-budget truncation only), `install_from_config`
is a no-op, and every `tokenjuice.*` RPC answers `COMPONENT_DISABLED`.
`[tokenjuice] router_enabled` stays a sub-setting of the on state. A legacy
config with `[tokenjuice] router_enabled = false` and no `[components] juice`
key is migrated on load to `[components] juice = false` (an explicit
`[components]` key always wins).
