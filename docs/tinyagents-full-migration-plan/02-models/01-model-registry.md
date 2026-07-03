# 02.1 — Workload routes as ModelRegistry entries

Today `run_turn_via_tinyagents_shared` registers exactly one crate-internal
`ProviderModel` per run. Target: register the run's full resolved route set so fallback and
capability resolution happen inside the SDK.

## Steps

1. Extend `assemble_turn_harness` to register one `ProviderModel` per
   workload route the turn may use (`chat`, `agentic`, `reasoning`, `coding`,
   `memory`, `subconscious`, `burst`, `summarization`, `vision` — from
   `provider/router.rs` tier names), each
   with its real `ModelProfile` (already built at construction; add
   structured-output/reasoning flags from provider capabilities as they
   gain accessors).
2. Use `ModelRequest::with_required_capabilities` /
   `ModelSelection`/`ModelHint` so per-call needs (vision, tools, reasoning)
   reject unfit models pre-dispatch instead of failing at the provider.
3. `router.rs` keeps owning tier-name → provider-string policy; the
   translation to registry entries lives in one adapter fn
   (`tinyagents/model.rs` or a new `tinyagents/routes.rs`).
4. Record resolution in events (`ResolvedModel`/`ModelResolutionSource`
   already on `ModelResponse`).

## Deletions

- Ad hoc model-capability checks scattered on turn paths (vision gates in
  `subagent_runner/ops/graph.rs`, `model_vision` param threading) once
  `CapabilitySet` covers them.

## Acceptance

- A turn requesting vision on a non-vision model fails pre-dispatch with a
  typed error; fallback picks the next capable route.
- Adapter-inventory test asserts the registered route set.
