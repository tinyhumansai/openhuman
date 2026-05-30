# inference

Unified inference domain: the canonical home for everything LLM/STT/TTS/embedding-related. It owns the local-runtime manager (Ollama / LM Studio / Whisper / Piper), the unified cloud + local provider abstraction (trait, factory, router, reliability/retry wrapper), voice transcription and TTS inference, OpenAI/Codex subscription OAuth, and an OpenAI-compatible `/v1/chat/completions` HTTP endpoint. It consolidates the previously separate `local_ai/`, `providers/`, and inference parts of `voice/` under one domain root. The RPC surface is still split across the `inference.*` and `local_ai.*` namespaces for backwards compatibility.

## Responsibilities

- Resolve workload names (`chat`, `reasoning`, `agentic`, `coding`, `memory`, `embeddings`, `heartbeat`, `learning`, `subconscious`, etc.) and provider strings (`openhuman`, `cloud`, `ollama:<model>`, `lmstudio:<model>`, `claude_agent_sdk:<model>`, `<slug>:<model>[@<temp>]`) to a concrete `Box<dyn Provider>` + model id.
- Manage the local AI runtime: detect/spawn/adopt `ollama serve` and LM Studio, install/run Whisper (STT) and Piper (TTS), track download progress, and enforce a minimum-context-window floor.
- Provide chat, vision (multimodal), summarization, embeddings, sentiment, and "should react" inference operations.
- Wrap providers with retry/backoff and config-rejection/billing-error classification (`reliable`, `config_rejection`, `billing_error`).
- Multi-model routing via a hint table (`RouterProvider`) keyed off abstract tier model names (`reasoning-v1`, `agentic-v1`, `coding-v1`, etc.).
- Run ChatGPT/Codex OAuth (PKCE) for the `openai` cloud slug and persist tokens in the encrypted auth-profile store.
- Expose an OpenAI-compatible `/v1/*` HTTP endpoint guarded by a stable user-managed external bearer.
- Detect device hardware profile and recommend/apply local model presets/tiers.

## Key files

| File / dir | Role |
| --- | --- |
| `mod.rs` | Domain root; module decls + re-exports; wires `inference.*` controller schemas/controllers. |
| `ops.rs` | Canonical handler file — `inference_*` business logic returning `RpcOutcome<T>`; delegates to `local`, `provider`, `sentiment`, `device`, `presets`, `openai_oauth`. Includes Sentry-noise suppression for expected provider/user-config failures. |
| `schemas.rs` | `inference.*` controller schemas + `handle_*` fns + param DTOs. |
| `types.rs` | Serde DTOs: `LocalAiStatus`, `LocalAiAssetsStatus`, `LocalAiDownloadsProgress`, `LocalAiEmbeddingResult`, `LocalAiSpeechResult`, `LocalAiTtsResult`, etc. |
| `device.rs` | `DeviceProfile` hardware detection (RAM/CPU/GPU/OS), cached. |
| `model_ids.rs` | Effective chat/vision/embedding/STT/TTS/quantization model id resolution from config. |
| `model_context.rs` | Known model context-window sizes (`context_window_for_model`) for pre-dispatch budgeting. |
| `presets.rs` | `ModelPreset`, `ModelTier`, `VisionMode`; tier recommendation + apply-to-config; MVP preset gating. |
| `sentiment.rs` | `SentimentResult` + emotion/valence analysis via the local model. |
| `parse.rs` / `paths.rs` | Output parsing helpers / on-disk model artifact paths. |
| `local/` | Local runtime manager (was `local_ai/`). |
| `local/core.rs` | `LocalAiService` singleton (`global`/`try_global`), `model_artifact_path`. |
| `local/ops.rs` | Local RPC entrypoints (`local_ai_status/prompt/summarize/vision_prompt/embed/should_react`, `ReactionDecision`); re-exported as `local::rpc`. |
| `local/schemas.rs` | `local_ai.*` controller schemas + handlers. |
| `local/ollama.rs`, `local/lm_studio.rs` | Provider-specific runtime drivers; base-url resolution. |
| `local/install*.rs`, `local/voice_install_common.rs` | Whisper/Piper install + shared download logic. |
| `local/model_requirements.rs` | `MIN_CONTEXT_TOKENS`, `evaluate_context`, `ContextEligibility`. |
| `local/service/` | `LocalAiService` impl split: `bootstrap`, `ollama_admin`, `public_infer`, `speech`, `vision_embed`, `whisper_engine`, `assets`, `spawn_marker`. |
| `provider/` | Unified provider abstraction (was `providers/`). |
| `provider/traits.rs` | `Provider` trait + `ChatMessage`/`ChatRequest`/`ChatResponse`/`ToolCall`/`UsageInfo`/`ProviderDelta` etc. |
| `provider/factory.rs` | `create_chat_provider`, `provider_for_role`, provider-string grammar, local/cloud construction; `BYOK_INCOMPLETE_SENTINEL`. |
| `provider/router.rs` | `RouterProvider` hint-based multi-model routing. |
| `provider/reliable.rs` | Retry/backoff wrapper. |
| `provider/compatible*.rs` | OpenAI-compatible provider (request dump/parse/stream/types). |
| `provider/openhuman_backend.rs` | Managed OpenHuman backend provider (session JWT). |
| `provider/claude_agent_sdk/` | Claude Agent SDK subprocess provider (`protocol.rs`, `subprocess.rs`). |
| `provider/config_rejection.rs`, `provider/billing_error.rs` | Error classifiers (unknown-model / config rejection / budget exhausted). |
| `provider/temperature.rs`, `provider/thread_context.rs` | Per-workload temperature override; thread context plumbing. |
| `provider/ops.rs` | `list_configured_models`, SessionExpired publishing on auth failure. |
| `provider/schemas.rs` | Provider-layer schemas. |
| `voice/` | Inference implementations imported by `crate::openhuman::voice`. |
| `voice/cloud_transcribe.rs`, `voice/local_transcribe.rs`, `voice/local_speech.rs` | STT (cloud + local) and local TTS. |
| `voice/streaming.rs`, `voice/postprocess.rs`, `voice/hallucination.rs` | Streaming transcription, post-processing, hallucination filtering. |
| `openai_oauth/` | ChatGPT/Codex OAuth: `config.rs` (Codex OAuth config), `flow.rs` (start/complete/status/disconnect), `store.rs` (token persistence). |
| `http/` | OpenAI-compatible endpoint: `server.rs` (`router()`), `types.rs`; `EXTERNAL_OPENAI_COMPAT_PROVIDER` bearer id. |

## Public surface

From `mod.rs` re-exports:

- `device::DeviceProfile`
- `model_context::context_window_for_model`
- `presets::{ModelPreset, ModelTier, VisionMode}`
- `sentiment::SentimentResult`
- `types::{LocalAiStatus, LocalAiAssetStatus, LocalAiAssetsStatus, LocalAiDownloadProgressItem, LocalAiDownloadsProgress, LocalAiEmbeddingResult, LocalAiSpeechResult, LocalAiTtsResult}`
- `local::all_local_ai_controller_schemas` / `local::all_local_ai_registered_controllers`
- `rpc` (alias for `ops`) and `all_inference_controller_schemas` / `all_inference_registered_controllers`

Provider-layer (via `provider::`): `Provider`, `ChatMessage`, `ChatRequest`, `ChatResponse`, `create_chat_provider`, `provider_for_role`, `BYOK_INCOMPLETE_SENTINEL`, plus error classifiers. Local runtime: `local::{global, try_global}` → `Arc<LocalAiService>`.

## RPC / controllers

Two namespaces, both wired into the controller registry (`src/core/all.rs`).

`inference.*` (`schemas.rs`): `status`, `get_client_config`, `update_model_settings`, `update_local_settings`, `list_models`, `device_profile`, `presets`, `apply_preset`, `diagnostics`, `openai_oauth_start`, `openai_oauth_complete`, `openai_oauth_status`, `openai_oauth_disconnect`, `summarize`, `prompt`, `vision_prompt`, `test_provider_model`, `should_react`, `analyze_sentiment`.

`local_ai.*` (`local/schemas.rs`): `agent_chat`, `agent_chat_simple`, `transcribe`, `transcribe_bytes`, `tts`, `assets_status`, `downloads_progress`, `download_asset`, `install_whisper`, `install_piper`, `whisper_install_status`, `piper_install_status`, `test_connection`.

Also exposes a non-RPC HTTP router (`http::router()`) nested at `/v1` by `src/core/jsonrpc.rs` (`/v1/chat/completions`, `/v1/models`), accepting either the core bearer or a stable external API key.

## Events

- Publishes `DomainEvent::SessionExpired` from `provider/ops.rs` when a provider chat attempt fails auth, so the credentials layer can clear/refresh the session.
- No `bus.rs` / `EventHandler` subscribers in this domain.

## Persistence

- `openai_oauth/store.rs` persists OAuth tokens via the credentials auth-profile store (`AuthProfilesStore`, `auth-profiles.json`, encrypted at rest) under profile key `provider:openai` / profile `oauth`.
- `LocalAiService` holds in-process runtime state (status, whisper engine handle, owned `ollama serve` child) via the `local::global` `OnceCell` singleton — process-lifetime, not durably persisted.
- Model artifacts live under `<root>/models/local-ai/` (`local/core.rs::model_artifact_path`); installed Whisper/Piper assets via `local/install*`.
- Routing/provider/local settings persisted through `config` (no dedicated `store.rs`).

## Dependencies

- `crate::openhuman::config` — `Config`, `config::rpc` (load/save, `ModelSettingsPatch`, `LocalAiSettingsPatch`), cloud-provider schema (`AuthStyle`, slug reservation, id generation), abstract tier model constants. Heaviest dependency.
- `crate::openhuman::credentials` — `AuthService`, `AuthProfilesStore`/`AuthProfile`/`TokenSet`, state dir — for OAuth token storage and provider auth resolution.
- `crate::openhuman::tools` — `ToolSpec`/`ToolCall` types used in provider chat requests (agent tool plumbing).
- `crate::openhuman::agent` — agent harness types referenced by provider/thread-context paths.
- `crate::openhuman::voice` — voice RPC/audio layer that imports these inference STT/TTS implementations (also a consumer).
- `crate::openhuman::prompt_injection` — prompt-injection handling on the inference path.
- `crate::openhuman::util` — small shared helpers.
- `crate::core::all` — `ControllerFuture`, `RegisteredController` (controller registry).
- `crate::core::types` — `ControllerSchema`, `FieldSchema`, `TypeSchema`.
- `crate::core::event_bus` — `DomainEvent`, `publish_global` (SessionExpired).
- `crate::core::observability` — `expected_error_kind` for Sentry-noise classification.
- `crate::core::jsonrpc` — endpoint mounting reference for `/v1`.
- `crate::core::auth` — bearer auth for the OpenAI-compatible endpoint.
- External: `motosan_ai_oauth` (Codex OAuth), `sysinfo` (device profile), `reqwest`, `whisper`-cpp engine bindings.

## Used by

Widely depended on (74 internal `use` sites + many external). Top consumers (by file count) are the agent layer (`agent/harness`, `agent/harness/session`, `agent/tools`, `agent/triage`, `agent/harness/subagent_runner`), `context`, `voice`, `routing`, `memory_tree/tree_runtime`, `learning` (+ `learning/transcript_ingest`), `channels`, `embeddings`, `subconscious`, `screen_intelligence`, `threads`, and `migrations`/`config/schema`.

## Notes / gotchas

- `mod.rs` re-exports `super::{device, model_ids, parse, paths, presets, sentiment, types}` under `local::` so files migrated from the old `local_ai/` keep compiling without rewriting `super::` paths.
- Provider strings carry an optional `@<temp>` suffix that pins a per-workload temperature; the suffix is stripped before the model id is sent upstream.
- `update_model_settings` silently drops reserved cloud-provider slugs (`openhuman`/`cloud`/`pid` built-ins the frontend echoes back); `apply_model_settings` re-injects them from stored config so they aren't lost.
- `ops.rs` deliberately demotes known provider/user-config failures (unknown cloud provider, 401/429, model-not-found) to `warn!` to keep them out of Sentry; only unclassified failures escalate to `error!`.
- `apply_preset` is MVP-gated: only the 1B local preset (`ram_2_4gb`) and `disabled` are accepted; `custom` cannot be applied via this path.
- `diagnostics` returns its payload unwrapped (no `{result, logs}` envelope) to match the legacy `local_ai_diagnostics` shape that `json_rpc_e2e` asserts against.
- Adopted (externally started) `ollama serve` daemons are never killed on exit; only the child OpenHuman itself spawned (`owned_ollama`) is.
- `local::global` lazily initialises the `LocalAiService` singleton; use `try_global()` on shutdown paths to avoid creating it just to no-op.
- The `/v1/*` endpoint uses a stable external bearer (`EXTERNAL_OPENAI_COMPAT_PROVIDER`) separate from the core launch bearer, so external OpenAI-compatible harnesses can call it.
- Tests serialize through `inference_test_guard()` (a process-global mutex) since the runtime singleton and config are shared.
