# OpenHuman TokenJuice Adapter

TinyJuice ships as a separately released TinyBus module. OpenHuman links only
its `tinyjuice-bus` vocabulary and declarations. Compression, cached retrieval,
queries and HTML extraction execute inside that module. This directory owns
configuration, model callbacks, artifact authorization and savings attribution.

Token compression is one of the ways OpenHuman keeps a turn cheap and fast
(see [gitbooks/developing/performance.md](../../../../../gitbooks/developing/performance.md)):
compressing tool output before it reaches the model shrinks the prompt
without the model ever seeing less information than it needs. See
[gitbooks/features/token-compression.md](../../../../../gitbooks/features/token-compression.md)
for the user-facing feature page.

OpenHuman-owned files:

| Path | Role |
| --- | --- |
| [`mod.rs`](./mod.rs) | TinyBus calls (through `tinyjuice_bus::names::methods` constants), config installation, pass-through fallback, and savings wiring. |
| [`types.rs`](./types.rs) | Re-export of the `tinyjuice-bus` contract (`tinyjuice_bus::types::{AgentTokenjuiceCompression, CompressOptions, CompressedOutput, CompressorKind, ContentHint, ContentKind}` and `tinyjuice_bus::wire::{CacheStats, CompactResponse, InstallRequest, RangeUnit, RetrieveRange}`) under the paths ~40 call sites in this crate already use. |
| [`schemas.rs`](./schemas.rs) | JSON-RPC controller schemas and handlers. |
| [`config_patch.rs`](./config_patch.rs) | Partial update shape for the `[tokenjuice]` config block. |
| [`generate.rs`](./generate.rs) | Turn-bound, single-use summary callbacks. The host registers the model call, sends its opaque ticket through the existing contract, and withdraws the callback when the ticket drops. |
| [`repl_tools.rs`](./repl_tools.rs) | The three REPL tools (`juice_find`, `juice_extract`, `juice_summarize`) over a stored result. Declarations come from `tinyjuice-bus`; operations execute over `Query` inside the module CCR store. Registered by [`tools/ops.rs`](../../tools/ops.rs) only while `repl_handle_active(config)`. |
| [`tools.rs`](./tools.rs) | OpenHuman agent tool implementation for the retrieve tool (`RETRIEVE_TOOL_NAME = "juice_retrieve"`; `"tokenjuice_retrieve"` is a recognized recovery-tool alias, not the tool's registered name, see `RECOVERY_TOOL_NAMES`). |
| Plain text | Compressed with TinyJuice's deterministic TextCrusher. |
| [`savings.rs`](./savings.rs) | OpenHuman model-pricing attribution and persisted dashboard stats. |

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
- `tools/ops.rs` registers `crate::inference::tokenjuice::TokenjuiceRetrieveTool::new()`
  in the agent tool catalog (it is not re-exported through [`tools/mod.rs`](../../tools/mod.rs)) and
  treats every `RECOVERY_TOOL_NAMES` entry as a recovery tool. The registered
  tool name is `RETRIEVE_TOOL_NAME` (`"juice_retrieve"`);
  `"tokenjuice_retrieve"` and `LEGACY_RETRIEVE_TOOL_NAME`
  (`"retrieve_tool_output"`) are recognized aliases only. Only
  `RECOVERY_TOOL_VISIBLE` (the live tool) is force-added to a curated
  `ToolScope::Named` belt (`session_host/builder/mod.rs::ensure_recovery_tool_visible`);
  the aliases stay registered for transcript replay but off the wire. It is
  added when compaction is on or the agent's results can be summarized
  (`summarizes_tool_output`), since a summary's footer names it too.
- Handle mode: `install_request` sets `CompressOptions.repl_handle` (and
  `repl_save_dir` when `repl_save_enabled`) from `repl_handle_active(config)`,
  which needs `context.compaction_enabled`, `router_enabled`, `ccr_enabled` and
  `repl_handle_enabled`. `tools/ops.rs` registers the REPL tools under the same
  test, `session_host/builder/mod.rs::ensure_repl_tools_visible` adds them to a
  curated belt, and `middleware/tool_output.rs::is_compaction_exempt` keeps
  their answers from being stored behind a second handle (their own
  `max_result_size_chars` still caps them). The additive `Query` member carries the typed operation; existing `Repl`
  arity is unchanged.
- Contract crate: `tinyjuice-bus` ([`vendor/tinyjuice/crates/tinyjuice-bus`](../../../../../vendor/tinyjuice/crates/tinyjuice-bus/),
  path dependency in [`crates/openhuman-core/Cargo.toml`](../../../Cargo.toml)).

## Further reading

- [Parent module README](../README.md)
- [tinyjuice](../../../../../vendor/tinyjuice/README.md)
