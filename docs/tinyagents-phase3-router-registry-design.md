# Phase 3 — RouterProvider → crate ModelRegistry: design & ground truth

**Status:** design (starting Phase 3). Grounded in a full read of both the crate
(`vendor/tinyagents/src/harness/model`, `harness/agent_loop`, `harness/retry`,
`registry/`) and the host seam (`src/openhuman/tinyagents/{mod,routes,model}.rs`,
`inference/provider/{router,factory}.rs`).
**Relates to:** `docs/tinyagents-inference-migration-plan.md` (Phase 3),
`docs/tinyagents-drift-ledger.md` (P1-9), issue #4249.

---

## 1. The premise correction (important)

The plan framed Phase 3 as "implement RouterProvider → crate ModelRegistry in
`vendor/tinyagents`." **Investigation shows the registry migration is already
done at the seam, and the crate already does everything the host router needs —
there is no hard upstream gap.** Specifically, `assemble_turn_harness`
(`tinyagents/mod.rs:1240-1353`) already:

- Registers the primary and every workload-tier route into the crate
  `ModelRegistry` (`harness.register_model(name, model)`; primary via
  `set_default_model`).
- Wires cross-route fallback as a **crate** `RunPolicy.fallback =
  routes::route_fallback_policy(model)` — traversed natively by
  `agent_loop::invoke_model_resolving`.
- Gates vision via the crate: `RequiredCapabilitiesMiddleware` stamps
  `image_in` onto the `ModelRequest`, and the crate's `resolve_request` filters
  named candidates by `ModelProfile::satisfies`.
- Emits the `FallbackSelected` parity event via `FallbackObserverMiddleware`.

The two things the crate *lacks* (per the crate audit) are **not needed** by the
host:

- **Capability-based *selection*** (scan the registry for a capable model): the
  host never does this — the **caller picks the tier** upstream
  (`subagent_runner/ops/graph.rs` sets `model = vision-v1` for image turns); the
  middleware only *validates/enforces*, it doesn't select.
- **Capability-aware *fallback***: the host's fallback chains are hand-built to
  be capability-safe (`routes::same_family_fallbacks`: `vision-v1 → []`; the
  same-family text alternates are all text-capable), so the crate's
  non-capability-filtered `next_after` is benign here.

**Conclusion:** RouterProvider is *already* projected onto the crate registry.
What remains is host-side, and needs no crate release.

## 2. What actually survives (the real remaining work)

`RouterProvider` (`inference/provider/router.rs`) survives only as the **per-call
BYOK alias resolver *inside* the wrapped `Provider`** — at dispatch it maps a
tier alias (`reasoning-v1`, …) → concrete `(provider, model)` (issue #2079: raw
aliases 400 on OpenAI/DeepSeek). The registered tier models are still
`ProviderModel`s that wrap this host `Provider`. So the harness holds a
`Provider` for exactly one reason: **`build_turn_models` builds the per-turn
primary + routes + summarizer `ProviderModel`s from one `Provider` handle**
(`tinyagents/mod.rs:1139-1175`, `routes::build_route_models`).

Two independent motions remain, in priority order:

### Motion A — Harness holds crate `ChatModel`s (this Phase; host-only)
Move `build_turn_models` construction from the harness turn path
(`agent/harness/graph.rs::run_channel_turn_via_graph`, session turn path) to the
**producer/factory boundary**, so `agent/` holds crate model types, not
`Provider`. The harness turn path today reads four `Provider` methods before
building — all are already available without the trait:

| harness reads today | crate-native source |
| --- | --- |
| `provider.supports_native_tools()` | `TurnModels.primary.profile().tool_calling` |
| `provider.supports_vision()` | `…profile().modalities.image_in` |
| `provider.effective_context_window(model).await` | resolved at build time → `…profile().max_input_tokens` |
| `provider.telemetry_provider_id()` | new `TurnModels.provider_id: String` |

So `TurnModels` becomes the unit the harness holds. Design points:

1. **Extend `TurnModels`** with `provider_id: String` and small accessors
   (`native_tools()`, `supports_vision()`, `context_window()`) reading
   `primary.profile()`. Removes every raw-`Provider` read in the harness graph.
2. **New factory entry** `inference::provider::factory::create_turn_models(
   role_or_model, config, temperature) -> anyhow::Result<TurnModels>`: builds the
   `Provider` internally (existing `create_chat_provider` path), resolves the
   async `effective_context_window`, computes `telemetry_provider_id`, and calls
   the seam `build_turn_models`. All `Provider` naming stays in
   `inference/provider/` + the seam.
3. **Lifecycle:** `build_turn_models` is per-`(provider, model)`; the harness
   builds a fresh `TurnModels` per turn today. Keep that — the producer builds
   `TurnModels` at turn-request assembly (channels processor, session turn entry,
   subagent runner) instead of the harness graph doing it. `error_slot` stays
   per-turn (correct — it recovers *this* turn's provider error).
4. **Type swap:** `AgentTurnRequest.provider: Arc<dyn Provider>` → carries the
   built `TurnModels` (+ the `model`/`provider_name` it already has);
   `Agent`/`AgentBuilder.provider` likewise. `run_channel_turn_via_graph` /
   session `turn/graph.rs` receive `TurnModels`, read caps via the accessors, and
   pass it straight to `run_turn_via_tinyagents_shared` (which already takes
   `TurnModels`). `IntelligentRoutingProvider` stays a `Provider` impl
   (provider-stack member); the factory wraps it before `build_turn_models`.

Exit: no `agent/` file names the `Provider` trait; `ProviderModel` built only in
the seam (`model.rs` / `build_turn_models`), reached only via the factory entry.
Zero behavior change (same `ProviderModel`s, same registry/fallback wiring).

### Motion B — Registered models become crate-native (later; the big LOC win)
Replace `ProviderModel` wrappers with crate `providers::openai` clients built
from config (BYOK slugs, Ollama/LM Studio base URLs), registering per-tier
clients directly so `RouterProvider`'s per-call alias resolution disappears
(each tier is its own registered client). This is inference-plan **Phase 2**
(client swap, deletes `compatible*.rs`) + the remainder of Phase 3, and keeps the
bespoke providers (managed backend, claude-code, codex) as host `ChatModel`
impls (Phase 4). Out of scope for Motion A; unblocked by it.

## 3. Optional crate nicety (not required)
If we later want capability-*aware* fallback (so a hand-built chain isn't the
only safety net), the one-line crate change is to re-apply `model_eligible`
to `FallbackPolicy::next_after` targets in `invoke_model_resolving`
(`vendor/tinyagents/src/harness/agent_loop/model_call.rs:186-204`). File as a
separate small upstream PR only if Motion B introduces capability-divergent
fallback chains. Not needed for Motion A.

## 4. First implementation slice (Motion A)
1. `TurnModels`: add `provider_id` + `native_tools()/supports_vision()/context_window()` accessors (seam-internal; behavior-neutral).
2. `create_turn_models(...)` factory entry (wraps `create_chat_provider` +
   async context-window resolve + `build_turn_models`).
3. Cut `run_channel_turn_via_graph` to take `TurnModels` and read caps via
   accessors (delete the 4 raw-provider reads); channel producer
   (`channels/runtime/dispatch/processor.rs`) builds `TurnModels` via the factory
   and puts it on `AgentTurnRequest`.
4. Repeat for the session turn path + subagent runner; swap `Agent.provider`.
5. Update tests to build `TurnModels`/crate `MockModel` instead of hand-rolled
   `Provider` impls.
6. Verify: both Cargo worlds green; `json_rpc_e2e`; streaming/cost/tool-timeline
   parity on a mock-backend turn (the #4460 / $0-turn / tool-timeline hazards).

## 5. Verification & parity locks (unchanged from the plan)
Provider-string grammar, `inference.*` RPC, tier alias set, fallback ordering
(single same-family alternate; vision primary-only), the once-per-logical-call
FIFO usage push (charged-USD-over-estimate precedence, graceful degradation), and
the `FallbackSelected` event must all be preserved. These are already crate-wired
— Motion A only moves *where the models are built*, not how they route.
