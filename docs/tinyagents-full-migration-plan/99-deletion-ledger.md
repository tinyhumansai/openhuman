# TinyAgents migration deletion ledger

This ledger records host-side removals required by
[`tinyagents-migration-plan-2026-07-22.md`](../tinyagents-migration-plan-2026-07-22.md).
A row moves to `DELETED` only after its crate-backed replacement and the named
parity evidence are in place. Generic code moved into `vendor/tinyagents` must
also name its upstream PR before the host copy is removed.

| Work package | Host artifact | Preconditions / replacement | Status | Evidence |
| --- | --- | --- | --- | --- |
| WP-1 | `inference/provider/router.rs` + tests | Crate `ModelRouter` owns live routing | DELETED | `fcd3f3331`; #4783 adopted the router; root `cargo check` green |
| WP-1 | `inference/provider/reliable.rs` + tests | Crate retry/fallback owns every model call | DELETED | `fcd3f3331`; crate retry/fallback plus root `cargo check` green |
| WP-1 | Legacy compatible raw-coverage trio | Wire parity retained against crate `OpenAiModel` | DELETED | `4750defb0`; all 14 `inference_provider_e2e` tests green |
| WP-1 | `inference/provider/legacy_provider.rs` and `compatible` alias | Every OpenAI-compatible slug uses crate `OpenAiModel` | DELETED | #4780/#4782/#4784 plus native wire/SSE parity tests; root check and raw-coverage target compile green |
| WP-1 | `inference/provider/traits.rs` + tests | No `impl Provider`; remaining host bridge data types live in `provider/types.rs` | DELETED | `rg 'impl Provider' src` empty; factory unit suite 173 passed |
| WP-1 | `tinyagents/model.rs::ProviderModel` / `MaxTokensModel` | Tier and bespoke models are direct `ChatModel`s | DELETED | `rg ProviderModel src` empty; tinyagents unit suite 127 passed |
| WP-1 | `tinyagents/convert.rs` message conversion | Runtime uses crate `Message`; durable JSONL/thread DTO conversion belongs beside agent persistence; retain tool-schema conversion until WP-4 | REHOMED | Message adapter and tests moved to `agent/message_convert.rs`; `tinyagents/convert.rs` is tool-schema-only; root check green |
| WP-1 | `inference/provider/crate_provider.rs` | No legacy `Provider` consumer needs the reverse adapter | DELETED | `rg 'impl Provider' src` empty; root `cargo check --lib` green |
| WP-1 | `inference/provider/{auth_error_registry,resolved_route,temperature,thread_context}.rs` | Host state/policy lives outside the legacy provider abstraction | REHOMED | `4ce6ca726`, `59871c8ab`, `6cb17f91e`, `07f675ba3`; root `cargo check` and focused auth-registry tests green |
| WP-2 | `routing/` parallel implementation | Generic decisions use crate `ModelRouter`; no live consumer remained | DELETED | Repository-wide reference audit found the module self-contained; #4783 owns live routing; root check green |
| WP-2 | `tool_timeout` implementation | No deletion: crate `ToolTimeout` is per-tool metadata, while the host owns global config/env state and enforces the adapter deadline | HOST-OWNED | Execution-path audit; timeout precedence/deadline tests |
| WP-2 | `model_council/` and `council_registry/` | Crate `parallel::map_reduce` already owned generic ordered fan-out; the product surface had no live consumer | DELETED | Crate-backed execution audit followed by upstream dead-code cleanup |
| WP-2 | `tool_status/` | OpenHuman security markers, serialized UI/persistence taxonomy, retry categories, and remediation copy | HOST-OWNED | Consumer audit; classifier/type unit tests |
| WP-3 | legacy `run_turn_engine` and graph escape hatches | All regression assertions exercise the crate turn path | ALREADY DELETED | Audit found no engine definition or runtime env read; session and subagent paths call TinyAgents unconditionally. Removed the stale runner comment. |
| WP-4 | host tool trait/adapter artifacts selected by design | Approved tool-model decision preserves security and ungated result types | DESIGN GATE | Successor design document |
| WP-5 | `SchemaGuardMiddleware` | TinyAgents `InvalidArgsPolicy::ReturnToolError` owns recoverable schema-invalid admission | DELETED | Policy regression + 18 `agent_harness_e2e` tests green; 232 net host lines removed in the cutover commit |
| WP-5 | generic seam middlewares | Equivalent crate middleware released and adopted | PARTIAL | `SchemaGuard` deleted; TinyAgents #72 repeat tracker adopted and host duplicate accounting deleted (51 focused middleware tests green); `ArgRecovery` still awaits TinyAgents #71. Per-middleware drift rows remain authoritative. |
| WP-5 | detached subagent registry mechanics | Crate `DetachedTaskRegistry` + `TaskStore`/`SteeringRegistry` own generic process-local lifecycle | CLOSED | TinyAgents #75 merged as `d548657` and canonical pointer `4358efe` contains it; OpenHuman commits `3fc769828` + `29908675f`; 17 focused `running_subagents` tests green. Host retains durable projection, product metadata, RPC, and `RunQueue` fallback. |
| WP-5 | `agent/progress_tracing.rs` and `progress_tracing/langfuse.rs` | C4 S2-S6 gates pass; journal projection is self-sufficient | BLOCKED | One-release shadow parity and C4 §5 gate |

Deletion totals are reconciled in WP-6 after all rows are terminal. The
original projection is approximately 30k host LOC deleted and 12–15k generic
LOC upstreamed; measured totals, not the estimate, are authoritative.
