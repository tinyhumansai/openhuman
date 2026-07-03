# 02 — Models & Providers

Make tinyagents the model abstraction: registry-resolved models with real
profiles, SDK-owned retry/fallback, native reasoning streaming, and catalog-
driven capability/pricing data. OpenHuman keeps credentials, OAuth, billing
classification, and provider construction (`factory.rs`).

Target SDK surface: `ModelRegistry::resolve(ModelSelection)`, `ModelProfile`/
`CapabilitySet` (incl. `reasoning`), `RetryPolicy`/`FallbackPolicy` on
`RunPolicy` + `ModelFallbackMiddleware`, `ProviderError` (normalized,
`retryable` flag), `ModelStreamItem::MessageDelta { text, reasoning,
tool_call }`, `registry::ModelCatalog{,Entry,Pricing,Capabilities}`.

Steps:

1. `01-model-registry.md` — workload routes as registry entries.
2. `02-fallback-retry.md` — collapse `reliable.rs` double-retry.
3. `03-streaming-reasoning.md` — stream reasoning natively; shrink
   `ThinkingForwarder` to the remaining tool-arg/non-streaming fallback seams.
4. `04-catalog.md` — one catalog for pricing/windows/capabilities.

Done when: `provider/reliable.rs` is deleted, reasoning/tool-argument streaming
rides the crate stream without OpenHuman side channels, model resolution is
capability-checked pre-dispatch, and pricing/context-window data has one source.

Keep (product): `provider/factory.rs`, `provider/router.rs` route *names*,
credential/OAuth/billing modules, `local/` daemon, `model_ids.rs` config
policy.
