# inference

The inference domain turns OpenHuman configuration into working language models. Given a workload
name such as `chat` or `reasoning`, it decides which provider serves it (the managed OpenHuman
backend, a bring-your-own-key cloud provider, a local runtime the user runs, or a CLI subprocess),
builds a native `ChatModel`, and applies product policy around it: credentials, Privacy Mode, error
copy, Sentry demotion, and context-window sizing. It also owns the `inference.*`, `embeddings.*`
and `tokenjuice.*` RPC surfaces and the OpenAI-compatible `/v1` HTTP endpoint.

Reusable model, provider-transport, embedding, capability, temperature and failure-classification
code lives in the vendored `tinyinference` crates. The agent loop and model-call retries live in
`tinyagents`. This folder is the product layer over both. The agent harness, voice, memory, web
chat, flows, scheduling and channels all call into it.

## How it works

### Resolving a workload to a model

Every caller starts with a role (`chat`, `reasoning`, `agentic`, `coding`, `vision`, `memory`,
`embeddings`, `learning`, and so on) or a model hint like `hint:reasoning`. The factory in
[`provider/factory.rs`](./provider/factory.rs) resolves that in three steps.

```text
  caller (agent turn, memory summarizer, flows node, RPC prompt)
     |
     |  role or hint:*         e.g. "reasoning", "hint:coding"
     v
  routing.rs                 role -> provider string from config
     |                        (chat_provider, reasoning_provider, ...)
     |  provider string       e.g. "openhuman", "ollama:qwen3@0.2"
     v
  access_gates.rs           Privacy Mode LocalOnly check, session check,
     |                        egress descriptor
     v
  constructor by prefix:
     +-- managed_backend.rs       "openhuman" / "cloud" -> tier pinning, gate, egress, then the registered chat model
     +-- local_runtime.rs         ollama / lmstudio / mlx / local-openai
     +-- cloud_slug.rs            "<slug>:<model>" BYOK: host gates, headers, TLS client
     +-- cloud_spec.rs            CloudProviderCreds -> ProviderSpec -> tinyinference ProviderRegistry
     +-- subprocess_providers.rs  claude_agent_sdk:, claude-code:
     v
  (Arc<dyn ChatModel<()>>, resolved model id)
```

The provider-string grammar is documented at the top of `provider/factory.rs`:

```text
"openhuman"                     managed backend, model = config.default_model
"cloud" / empty                 primary_cloud (legacy inference_url wins while
                                primary still points at OpenHuman)
"ollama:<model>[@<temp>]"       local Ollama at config.local_ai.base_url
"lmstudio:<model>[@<temp>]"     local LM Studio
"mlx:<model>[@<temp>]"          local MLX-compatible server
"local-openai:<model>[@<temp>]" any local OpenAI-compatible server
"<slug>:<model>[@<temp>]"       cloud_providers entry keyed by slug
"claude_agent_sdk:<model>"      Claude Agent SDK subprocess
"claude-code:<model>"           Claude Code CLI subprocess
```

The optional `@<temp>` suffix pins a temperature for that workload. It is stripped
(`tinyinference_providers::route::split_model_and_temperature`) before the model id goes upstream. Unknown slugs and
missing credentials produce actionable errors instead of a silent fallback.

Background roles (`vision`, `embeddings`, `memory`, `agentic`, `burst`) fall back to the primary
cloud provider when their own route is unset, because they run tier models that local runtimes and
most BYOK slugs do not serve. [`provider/fallback_diagnostics.rs`](./provider/fallback_diagnostics.rs) exists so that when this fallback
lands on a provider with no credentials, the error says it was a background role that fell back,
instead of naming a slug the user never configured.

Provider model aliases, defaults, and capabilities come from the registered provider-model hooks
in `registry.rs`. The `provider_model.rs` adapter uses TinyAgents' `WorkloadRouter` for workload
role and hint resolution while preserving provider-owned aliases and concrete ids.

One-shot callers use `create_chat_model*` in `factory/chat_model.rs`. Agent turns use the
TinyAgents `TurnModelSet` through `TurnModelSource`, which builds the primary, workload routes,
and summarizer under the same host gates.

### The managed backend model

Managed chat uses TinyInference's `TinyHumansModel` through the host adapter in
`openhuman-tinyhumans/src/inference/managed_model.rs`. The adapter supplies the live bearer
resolver and host error observer; TinyInference owns request construction and response parsing,
including the managed usage envelope. The host checks stored-session expiry before a request and
publishes `DomainEvent::SessionExpired` locally when that check fails. Its typed authentication-
failure callback publishes the same event for managed endpoint authentication failures, using the
owner's explicit provenance rather than inferring ownership from the provider label.

When ChatGPT/Codex OAuth tokens exist for the `openai` slug, the host selects the OAuth
credential and TinyInference's `resolve_codex_routing` supplies the Codex backend route. OpenHuman
adds the account and originator headers. TinyHumans owns the provider-facing OAuth start, callback
exchange, Codex CLI import and token refresh behind the core's `ProviderCredentialSource` port.
Core keeps profile persistence, credential lookup, status and disconnect state in
[`credentials/openai_oauth/`](../credentials/openai_oauth/). TinyHumans registers
the existing `inference.openai_oauth_*` RPC names.

### Local runtimes

The local runtime (Ollama, LM Studio, MLX, OMLX, any OpenAI-compatible server) is installed,
started and stocked with models by the user. OpenHuman never downloads a model, never launches or
stops the runtime, never installs Piper, and does not pick models by RAM tier. It probes the
configured endpoint (`ready`, `degraded`, `unreachable`), reports which configured models are
served, enforces a minimum context window, and points the user at `ollama pull <model>` when a
model is missing.

`tinyinference_local` does the endpoint work. `host_runtime/` keeps the product side:

```text
  config.local_ai  --local_runtime_config()-->  tinyinference_local::RuntimeConfig
                                                         |
  host_runtime::global(config)  ---- OnceCell ---->  LocalAiService (probe state)
         |
         +-- ops/runtime_ops.rs   status, summarize, prompt, vision, transcribe, tts
         +-- ops/agent_chat_simple.rs lightweight provider-only chat
         +-- service/speech.rs    transcribe / tts bindings used by crate::voice
```

`LocalAiService` is a process-lifetime singleton holding the last probe verdict.
`inference.update_local_settings` resets it so the next status poll re-probes. Local STT through
whisper.cpp is retired; speech-to-text goes through the engine configured in
`voice_server.stt_engine` (see [`config/migrations/retire_local_whisper_stt.rs`](../config/migrations/retire_local_whisper_stt.rs)), and the speech
bindings call `crate::voice::create_stt_provider` for it. Local TTS uses a Piper binary the user
installed (`PIPER_BIN` or `PATH`).

`agent_chat_for` in [`agent_runs/agent_chat/session_turn.rs`](../agent_runs/agent_chat/session_turn.rs) is the native agent-turn entry used by
`openhuman-rpc` and `openhuman-embed`. It runs the turn under the agent's own `CoreContext`,
resumes the thread's session with `ResumeMode::Session`, and is boxed on purpose so other crates
do not each instantiate its state machine.

### Context windows

The context window drives pre-dispatch trimming and the compaction trigger. [`context_window.rs`](./context_window.rs)
supplies the selected route's discovery request, config override, local profile,
catalog hint, managed-provider default, and configured CA/proxy client to
TinyInference's route resolver. TinyInference owns discovery URL normalization,
caching, remembered route limits, and the shared fallback order:

```text
  1. config override       model_registry[].context_window (non-zero)
  2. provider-reported     discovered limits and learned overflow limits for
                           the exact endpoint/model/cache identity
  3. remembered            provider limits for the route/model identity
  4. static/local          registered provider model metadata, managed default,
                           workload tier, pricing catalog, model-id hint, then
                           local profile fallback
```

The host context-window resolver builds the discovery request at the point it needs it (`context_window.rs`), using
from the same endpoint and credential resolution the chat factory uses. Route-aware synchronous
projection uses the exact discovery identity when available; model-only displays use a conservative
route projection and cannot borrow a learned limit from an unrelated provider. [`model_context.rs`](./model_context.rs) also answers vision capability through
`model_supports_vision` and the user's `model_registry` `vision` flag.

### Failures

A failed provider call passes through several layers, each with one job:

```text
  HTTP / transport error
     |
     v
  provider/http_error.rs   api_error(): tinyinference_llm::failure::http
     |  classifies; HostErrorReporter applies backend guards, upstream demotion, Sentry
     |
     +--> http_error.rs: 401 from managed backend -> SessionExpired event
     |                     401/403 on a BYO key -> auth_error_registry::record
     |                        -> ProviderApiKeyRejected (once per episode)
     v
  registry.rs                provider-owned classification metadata
  TinyInference failure classifier and retry-after parser
     v
  failure_copy/table.rs      class -> error_type, source, retryable, copy
                             (consumed by web_chat::web_errors)
```

Provider-text matching itself (including the budget-phrase matcher) is upstream in
`tinyinference`. [`auth_error_registry.rs`](./auth_error_registry.rs) is the single record of rejected BYO keys: the
notification center gets one event per failure episode, and the AI settings page reads the live
list through `inference.provider_auth_errors`. Entries clear when the user updates or removes the
key in the credentials domain. [`failure_copy/halt.rs`](./failure_copy/halt.rs) holds the summaries the loop guards record
when they stop a turn for repeated tool failures.

### The OpenAI-compatible endpoint

The handlers for `/v1/chat/completions` (with SSE streaming) and `/v1/models` live in `tinyinference_llm::providers::openai::server_http`; `http/backend.rs` is the host `ChatBackend` that resolves models against `Config`.
`openhuman-rpc` mounts it with `.nest("/v1", openhuman_core::inference::http::router())`, so it sits
behind the same bearer middleware as `/rpc`. The bearer may be either the per-launch core token or
a stable user-managed key stored under the `EXTERNAL_OPENAI_COMPAT_PROVIDER` auth profile, which
lets external harnesses use OpenHuman as an OpenAI-compatible router. The request's `model` field
picks the provider the same way a provider string does.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module declarations and re-exports. Also `local_runtime_config` (projects `config.local_ai` into TinyInference's input), `local_vision_mode`, `disabled_local_ai_status`, the `LocalModelConfig` impl for `Config`, and `INFERENCE_COMPILED_IN`. |
| `ops.rs` | `inference_*` business operations returning `Outcome<T>`: status, prompt, summarize, vision, sentiment, provider tests, model listing, settings updates, diagnostics. Demotes expected user-config failures to `warn!`. |
| [`schemas.rs`](./schemas.rs), `schemas/` | The `inference.*` controller registry. `catalog.rs` has the schemas; `prompt_handlers.rs`, `settings_handlers.rs`, `oauth_handlers.rs` and `claude_code_handlers.rs` hold the thin handlers. |
| `context_window.rs` | Per-turn context-window inputs (override, discovery request, static and local windows); the precedence and remembered windows are `tinyinference_llm::model::context_window::ContextWindowResolver`. |
| `model_context.rs` | Static context-window table, the remembered window lookup, and vision capability checks. |
| `auth_error_registry.rs` | Process singleton and user-facing copy over `tinyinference_providers::auth_registry::ProviderAuthErrorRegistry` (the once-per-episode latch). |
| `failure_copy/` | `table.rs` maps a failure class to the wire `error_type`, source, retry verdict and user copy. `halt.rs` has loop-guard halt summaries. |
| `provider/` | Model construction and provider policy. See [`provider/README.md`](provider/README.md). |
| `provider/factory.rs`, [`provider/factory/`](./provider/factory/) | Provider-string grammar and model construction: the `RouteSource` adapter for TinyAgents' `WorkloadRouter`, `primary_cloud`, `access_gates`, `credentials`, `managed_backend`, `local_runtime`, `cloud_slug`, `cloud_spec`, `subprocess_providers`, `discovery`, `chat_model`, `turn_model`. |
| `provider_model.rs`, `registry.rs` | Provider-neutral model hooks and caller adapters; hosted aliases, model defaults, pricing, and request metadata are registered by `openhuman-tinyhumans`. |
| `registry.rs` | Registered provider ownership metadata; provider-neutral retry classification is supplied by TinyInference. |
| `provider/fallback_diagnostics.rs` | Explains background-role fallback when it hits a provider with no credentials. |
| [`provider/`](./provider/) | [`http_error.rs`](./provider/http_error.rs) (host reporter over `tinyinference_llm::failure::http`, `api_error`), `models/` (model catalog listing, local-runtime and OpenRouter entries), `factory/` (chat-model construction and cloud provider resolution). |
| `ops/{status,oauth,models,sentiment}.rs` | Inference RPC operations grouped by product surface; sentiment prompts and provider response types come from TinyInference. |
| `host_runtime/` | Product policy, RPC and speech bindings over `tinyinference_local`. See [`host_runtime/README.md`](host_runtime/README.md). |
| `embedding_host/` | Embedding provider selection (managed, Voyage, OpenAI, Cohere, Ollama, custom, noop) and the `embeddings.*` RPC. See [`embedding_host/README.md`](embedding_host/README.md). |
| `tokenjuice/` | Host adapter for the TinyJuice tool-output compression module, the `juice_retrieve` recovery tool, and the `tokenjuice.*` RPC. See [`tokenjuice/README.md`](tokenjuice/README.md). |
| `http/` | OpenAI-compatible `/v1` endpoint. `backend.rs` (host `ChatBackend` and `router()`) is behind the `http-server` feature; `EXTERNAL_OPENAI_COMPAT_PROVIDER` stays ungated because `core::auth` reads it. |

## Key types and entry points

- `create_chat_model`, `create_chat_model_with_model_id`, `create_chat_model_from_string` (in
  [`provider/factory/chat_model.rs`](./provider/factory/chat_model.rs)) build a model for one-shot work from a role or an explicit
  provider string.
- `TurnModelSource` ([`turn_models.rs`](./turn_models.rs)) uses TinyAgents' `TurnModelSet` to
  build models for an agent turn.
- `provider_for_role` ([`provider/factory/routing.rs`](./provider/factory/routing.rs)) returns the configured provider string for a
  role. `provider_model.rs` handles generic `hint:*` workload markers and consults provider hooks for provider-owned aliases.
- `probe_inference_readiness` (`provider/factory/chat_model.rs`) checks that a role can actually
  run: it builds the model and, for the managed backend, makes one cheap completion.
- `BYOK_INCOMPLETE_SENTINEL` (`provider/factory.rs`) marks a BYOK route with missing pieces.
- `TinyHumansModel` (from TinyInference) is the managed chat implementation; `openhuman-tinyhumans/src/inference/managed_model.rs` supplies host credential and failure policy, and `registry.rs` registers it under `PROVIDER_OPENHUMAN`.
- `host_runtime::global` ([`host_runtime/core.rs`](./host_runtime/core.rs)) returns the `LocalAiService` singleton.
- `agent_chat_for` and `agent_chat_reply_for` (`agent_runs/agent_chat/session_turn.rs`) run an agent turn
  natively. `INFERENCE_AGENT_CHAT` (`agent_runs::INFERENCE_AGENT_CHAT`) is the wire method name hosts reference.
- `context_window_for_model` (`model_context.rs`) and `resolve_context_window`
  (`context_window.rs`) answer window sizes.
- `api_error` ([`provider/http_error.rs`](./provider/http_error.rs)) is the single entry for turning a failed
  provider response into a classified error.
- `auth_error_registry::{record, clear, snapshot}` track rejected BYO keys.
- `test_provider_override` (`provider/factory.rs`) injects a mock `ChatModel` process-wide. It
  exists only under `cfg(test)` or the `e2e-test-support` / `rss-bench` features.

## RPC / CLI surface

All controllers are pushed into the registry in [`core/all.rs`](../core/all.rs) under `DomainGroup::Inference`.

`inference.*` from `schemas.rs`:

- Models and routing: `resolve_model`, `list_models`, `test_provider_model`.
- Status and settings: `status`, `get_client_config`, `update_model_settings`,
  `update_local_settings`, `diagnostics`, `provider_auth_errors`.
- One-shot inference: `summarize`, `prompt`, `vision_prompt`, `analyze_sentiment`.
- OpenAI/Codex OAuth keeps the `openai_oauth_start`, `openai_oauth_complete`,
  `openai_oauth_import_codex_cli`, `openai_oauth_status`, and
  `openai_oauth_disconnect` RPC names. TinyHumans registers the obtain/import
  controllers; core owns profile status and disconnect behavior.
- Claude Code: `claude_code_status`, `claude_code_auth_status`, `claude_code_settings`,
  `claude_code_set_full_access`.

`inference.*` from [`agent_runs/agent_chat/controllers.rs`](../agent_runs/agent_chat/controllers.rs): `agent_chat`, `agent_chat_simple`.
`inference.*` from [`host_runtime/schemas.rs`](./host_runtime/schemas.rs): `transcribe`, `transcribe_bytes`, `tts`, `test_connection`.

`embeddings.*` from [`embedding_host/schemas.rs`](./embedding_host/schemas.rs): `get_settings`, `update_settings`, `set_api_key`,
`clear_api_key`, `embed`, `test_connection`.

`tokenjuice.*` from [`tokenjuice/schemas.rs`](./tokenjuice/schemas.rs): `detect`, `compress`, `cache_stats`, `retrieve`,
`settings_get`, `settings_update`, `savings_stats`, `savings_reset`.

Legacy names such as `openhuman.local_ai_*`, `openhuman.update_local_ai_settings` and
`openhuman.providers_list_models` are rewritten to the canonical methods by
[`core/legacy_aliases.rs`](../core/legacy_aliases.rs) (and [`app/src/services/rpcMethods.ts`](../../../../app/src/services/rpcMethods.ts) on the frontend). The download,
asset-status, Piper-installer, preset and device-profile controllers no longer exist.

Outside JSON-RPC, `http::router()` serves `/v1/chat/completions` and `/v1/models`.

## Events and persistence

This domain publishes but does not subscribe; there is no `bus.rs`. `SessionExpired` comes from
[`provider/http_error.rs`](./provider/http_error.rs) using registered backend ownership metadata.
`ProviderApiKeyRejected` comes from `provider/http_error.rs`, gated by `auth_error_registry`.

There is no `store.rs`. Routing, provider and local settings persist through `config` (`config/ops/
model.rs` and friends). Provider keys live in the encrypted auth-profile store under
`provider:<slug>` ([`provider/factory/credentials.rs`](./provider/factory/credentials.rs) also tries the legacy bare `<slug>`). The
built-in BYOK presets behind Connections, API keys, LLM are in [`config/schema/cloud_providers.rs`](../config/schema/cloud_providers.rs).
OpenHuman stores no model artifacts.

## Boundaries

- `tinyinference` (vendored at [`vendor/tinyagents/vendor/tinyinference`](../../../../vendor/tinyagents/vendor/tinyinference/)) owns provider wire
  clients, model capability hints, temperature rules, provider-neutral error classification,
  endpoint probing for local runtimes, effective model-id resolution, and embedding providers.
  Fix provider-transport bugs there.
- `tinyagents` owns the agent loop, model-call retries and the Claude Agent SDK and Claude Code
  providers (`tinyagents_harness::providers::{claude_agent_sdk, claude_code}`). This domain only
  supplies routing and the MCP endpoint for Claude Code.
- `tinyjuice` owns compression; `tokenjuice/` is only the host adapter.
- OpenAI OAuth provider operations live in `openhuman-tinyhumans/src/provider_oauth/`; core owns the credential profile and its read/status/disconnect behavior.
- Privacy Mode policy and egress descriptors come from `security::live_policy` and
  `security::egress`; the factory only enforces them at `access_gates.rs`.
- Voice capture, dictation and the STT engine choice belong to `crate::voice`.
- Error classification ladders for chat replies live in `web_chat::web_errors`; this domain owns
  only the copy table.

## Gotchas

- `update_model_settings` drops reserved cloud-provider slugs (the `openhuman`/`cloud` built-ins
  the frontend echoes back), and `config::ops::model::apply_model_settings` re-injects them from
  stored config so they are not lost.
- `inference.diagnostics` returns its payload without the `{result, logs}` envelope, matching the
  legacy `local_ai_diagnostics` shape that `json_rpc_e2e` asserts on.
- `ops.rs` logs known provider and user-config failures (unknown provider, 401, 429,
  model-not-found) at `warn!` to keep them out of Sentry. Only unclassified failures reach
  `error!`.
- A managed response carrying an `errorCode` is owned by the backend for Sentry purposes; the core
  must not double-report it, except for a backend-flagged malformed `BAD_REQUEST`.
- `test_provider_override` is process-global. Tests that install it must run serially and drop the
  guard.
- `http::router()` only exists with the `http-server` feature. `INFERENCE_COMPILED_IN` reports
  whether the `inference` feature (and the `cpal` audio stack) was compiled in.
- There is no local-runtime shutdown step because OpenHuman never spawns one.

## Tests

Tests sit beside their modules as `<module>_tests.rs` (for example [`ops_tests.rs`](./ops_tests.rs),
[`context_window_tests.rs`](./context_window_tests.rs), [`provider/factory_tests.rs`](./provider/factory_tests.rs)). Tests that touch the runtime singleton or
shared config take `inference_test_guard()` / `inference_test_guard_async()` from
[`host_runtime/mod.rs`](./host_runtime/mod.rs). Run them with `cargo test -p openhuman inference::` or
`pnpm debug rust inference`. JSON-RPC behavior is covered in [`tests/json_rpc_e2e.rs`](../../../../tests/json_rpc_e2e.rs).

## Further reading

- [Parent module README](../../README.md)
- [Automatic model routing](../../../../docs/gitbooks/en/features/model-routing/README.md)
- [Local models and bring your own key](../../../../docs/gitbooks/en/features/model-routing/local-and-byok-models.md)
- [Pluggable engines](../../../../docs/gitbooks/en/developing/engines.md)
