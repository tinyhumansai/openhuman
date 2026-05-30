# routing

Intelligent model routing — a policy-driven layer that sits between callers (agent harness, channels, tools) and the concrete inference providers, deciding per request whether to run inference on a **local** model server or the **remote** OpenHuman backend. It classifies each request by task complexity (from `hint:*` model strings), checks cached local-model health, applies privacy/latency/cost hints, dispatches to the chosen backend, and transparently falls back to remote when a local call fails or returns a low-quality response. It is not an RPC-facing domain — it exposes no controllers, agent tools, event-bus subscribers, or persisted state; its only output side-effect is structured telemetry via `tracing`.

## Responsibilities

- Classify a model string (possibly `hint:*`) into a `TaskCategory` (`Lightweight` / `Medium` / `Heavy`).
- Produce a deterministic routing decision `(primary, fallback)` from task category, local availability, and per-call routing hints (privacy, latency budget, cost sensitivity).
- Construct an `IntelligentRoutingProvider` wrapping a remote provider plus a locally-resolved OpenAI-compatible provider (Ollama / LM Studio / llama.cpp / custom OpenAI), honoring the `OPENHUMAN_LOCAL_INFERENCE_URL` env override.
- Probe and cache local model-server health (`GET {base}/api/tags` for Ollama, `GET {base}/models` for OpenAI-compat backends) with a 30 s TTL and 3 s probe timeout.
- On a local primary: dispatch locally, and on error **or** low-quality output retry on remote — unless `privacy_required` forbids leaving the device.
- Heuristically score local responses for low quality (length floor, empty-noise utterances, refusal phrases) to drive fallback.
- Normalize heavy `hint:*` model strings to backend-valid model IDs (`reasoning` → `MODEL_REASONING_V1`, `chat` → `MODEL_REASONING_QUICK_V1`, `agentic` → `MODEL_AGENTIC_V1`, `coding` → `MODEL_CODING_V1`).
- Force remote when native tool-calling is required (tools present) and refuse to silently bypass local routing for streaming.
- Emit a structured `RoutingRecord` (category, target, resolved model, health, fallback flag, latency, tokens, cost) per completed call.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/routing/mod.rs` | Module docstring + `pub mod` decls and `pub use` re-exports of the public surface. |
| `src/openhuman/routing/policy.rs` | `TaskCategory`, `RoutingTarget`, `RoutingHints` (`LatencyBudget`, `CostSensitivity`, `privacy_required`); `classify()` (hint → category) and `decide()` (the pure routing-decision function). Inline tests. |
| `src/openhuman/routing/factory.rs` | `new_provider()` — resolves local provider kind/base URL/health probe from `LocalAiConfig` + env override and assembles an `IntelligentRoutingProvider`. Inline tests. |
| `src/openhuman/routing/provider.rs` | `IntelligentRoutingProvider` — `impl Provider`; resolves targets, dispatches `chat_with_system` / `chat_with_history` / `chat` / streaming, performs fallback, and emits telemetry. Tests in sibling `provider_tests.rs`. |
| `src/openhuman/routing/health.rs` | `LocalHealthChecker` — async, `parking_lot::Mutex`-cached health probe (lock never held across `await`). Inline tests. |
| `src/openhuman/routing/quality.rs` | `is_low_quality()` — allocation-free hot-path heuristic over a `LazyLock` Aho-Corasick refusal DFA, empty-noise token list, and length gate. Inline tests. |
| `src/openhuman/routing/telemetry.rs` | `RoutingRecord` + `emit()` — structured `tracing::info!`/`warn!` under target `"routing"`. Inline tests. |
| `src/openhuman/routing/provider_tests.rs` | Sibling test module for `provider.rs` (`#[path = ...]`). |

## Public surface

Re-exported from `mod.rs`:

- `factory::new_provider` — build an `IntelligentRoutingProvider` from a remote provider + `LocalAiConfig`.
- `health::LocalHealthChecker` — cached local-server health checker.
- `policy::{classify, decide, RoutingTarget, TaskCategory}` — classification + decision primitives.
- `provider::IntelligentRoutingProvider` — the `Provider` implementation.
- `quality::is_low_quality` — response-quality heuristic.
- `telemetry::{emit as emit_routing_record, RoutingRecord}` — telemetry record + emitter.

Not re-exported but public within the module: `policy::{RoutingHints, LatencyBudget, CostSensitivity}` and `IntelligentRoutingProvider::with_hints`.

## RPC / controllers

None. This module exposes no `schemas.rs`, no `all_*_controller_schemas`, and no `openhuman.routing_*` RPC methods. It is consumed in-process by the inference layer.

## Agent tools

None (no `tools.rs`).

## Events

None published or subscribed (no `bus.rs`). The only observability output is `tracing` telemetry under target `"routing"` (see `telemetry.rs`).

## Persistence

None (no `store.rs`). The only state is the in-memory, TTL'd health cache inside `LocalHealthChecker` (`parking_lot::Mutex<Option<HealthCache>>`), which is ephemeral and not persisted.

## Configuration

- `LocalAiConfig` (`openhuman::config`): `runtime_enabled`, `provider`, `base_url`, `api_key`, `chat_model_id` drive local-provider selection and whether local routing is active at all.
- `OPENHUMAN_LOCAL_INFERENCE_URL` (env): full `/v1` base URL of a local OpenAI-compatible server; when set, takes precedence over `config.base_url` and switches the health probe to `GET {base}/models`.
- Model-ID constants from `openhuman::config`: `MODEL_REASONING_V1`, `MODEL_REASONING_QUICK_V1`, `MODEL_AGENTIC_V1`, `MODEL_CODING_V1`.

## Dependencies

- `openhuman::config` — `LocalAiConfig` and backend model-ID constants used for local-provider resolution and heavy-hint normalization.
- `openhuman::inference::provider` — `Provider` trait + `traits::*` (`ChatRequest`, `ChatResponse`, `ChatMessage`, `StreamChunk`/`StreamError`/`StreamOptions`/`StreamResult`, `ProviderCapabilities`, `ToolsPayload`); the routing provider implements `Provider` and wraps two `Box<dyn Provider>`s.
- `openhuman::inference::provider::compatible` — `OpenAiCompatibleProvider` + `AuthStyle` used to build the local provider in `factory.rs`.
- `openhuman::inference::local` — `ollama_base_url`, `lm_studio::lm_studio_base_url_from_local_ai`, `provider::normalize_provider` for base-URL/provider-kind resolution.
- `openhuman::tools` — `ToolSpec` (passed through `convert_tools` and to detect tool presence forcing remote routing).
- `openhuman::util` — `floor_char_boundary` for safe UTF-8 truncation of log previews.
- External crates: `reqwest` (health probe), `parking_lot` (cache mutex), `aho-corasick` (refusal DFA), `async-trait`, `futures-util`, `anyhow`, `tracing`.

## Used by

- `src/openhuman/inference/provider/ops.rs` — calls `crate::openhuman::routing::new_provider(...)` to wrap the backend provider with intelligent routing.
- `src/openhuman/agent/triage/routing.rs` — references the routing wiring (mirrors `routing::factory::new_provider`'s local arm).

## Notes / gotchas

- **`hint:chat` goes remote, not local.** Despite being the front-line conversational tier, `classify()` maps it (and any unrecognized `hint:*` or exact model name) to `Heavy`, which always routes remote — the local model is too slow for the TTFT budget that motivated the hint.
- **`privacy_required` fails closed.** It forces local routing with no remote fallback for *every* category (including heavy and when local is unhealthy), and disables streaming (`supports_streaming()` returns `false`).
- **Local streaming is intentionally unsupported.** If policy selects local for a streaming call, the provider returns a single `StreamError::Provider` chunk rather than silently delegating to remote (avoids bypassing privacy/local routing).
- **Tools force remote.** When a `chat` request carries tools and policy chose local, routing overrides to remote (`remote_fallback_model`) because local lacks native tool calling.
- **Fallback also triggers on low quality, not just errors** (`should_fallback` / `is_low_quality`) — the heuristic is deliberately conservative toward flagging low quality, since serving a refusal is more user-visible than an extra remote call.
- **Health cache lock discipline:** `LocalHealthChecker` never holds the `Mutex` across an `await`; it reads/writes the cache, releases, then probes.
- **Token/cost telemetry is only populated for `chat()`** (which carries `ChatResponse.usage`); `chat_with_system` and `chat_with_history` emit records with zeroed token/cost fields.
- Medium tasks default to **remote** unless at least one local-bias hint is set (`LatencyBudget::Low` or `CostSensitivity::High`); lightweight tasks are local-first when local is healthy.
