# provider

Native TinyAgents `ChatModel` construction plus cloud/local inference policy,
auth, error taxonomy, and RPC helpers for every chat-model transport OpenHuman
supports. This is the chat-model half of OpenHuman's pluggable-engine story
(see [gitbooks/developing/engines.md](../../../../../gitbooks/developing/engines.md)):
the model a turn runs on is chosen by config, not by code change. Was
previously `providers/` (pre-consolidation single-crate layout); see
`../README.md` for how this fits into the wider `inference` domain.

## Public surface

- **Factory** (`factory.rs` + `factory/`: `routing.rs`, `tiers.rs`, `turn_model.rs`, `subprocess_providers.rs`, `access_gates.rs`, `chat_model.rs`, `cloud_slug.rs`, `credentials.rs`, `local_runtime.rs`, `managed_backend.rs`, `primary_cloud.rs`), `create_chat_model`,
  `create_chat_model_from_string[_with_model_id]`,
  `create_chat_model_with_model_id`, `provider_for_role`, `role_for_model_tier`,
  `probe_inference_readiness`, `BYOK_INCOMPLETE_SENTINEL`. Parses the
  provider-string grammar (`openhuman`, `cloud`, `ollama:<model>`,
  `lmstudio:<model>`, `mlx:<model>`, `omlx:<model>`, `local-openai:<model>`,
  `claude_agent_sdk[:<model>]`, `claude-code:<model>`, `<slug>:<model>[@<temp>]`)
  and applies the BYOK sentinel, Privacy-Mode `LocalOnly`
  (`enforce_local_only_inference`), and managed-session (`verify_session_active`)
  gates before building a model. See
  [gitbooks/features/model-routing/local-and-byok-models.md](../../../../../gitbooks/features/model-routing/local-and-byok-models.md)
  for the user-facing side of local and BYOK routing.
- **Models**: `OpenHumanBackendModel` + `PROVIDER_LABEL`
  (`openhuman_backend_model.rs`); OpenAI-compatible and Anthropic builders live
  in `tinyinference_llm::providers` and are called directly.
- **DTOs** (`types.rs`): `ChatResponse`, `ProviderDelta`,
  `ToolCall`, `UsageInfo`, `AGENT_TURN_MAX_OUTPUT_TOKENS`.
- **Error classifiers**: reusable classifiers live in
  `tinyinference_llm::classification`; this directory retains OpenHuman managed-backend and telemetry policy.

## Transports

| Transport | File | Provider-string prefix |
| --- | --- | --- |
| Managed OpenHuman backend | `openhuman_backend_model.rs` | `openhuman` / `cloud` (session JWT + billing metadata) |
| OpenAI-compatible (BYOK cloud slugs, local runtimes) | `tinyinference_llm::providers::openai` | `<slug>:<model>`, `ollama:<model>`, `lmstudio:<model>`, `mlx:<model>`, `omlx:<model>`, `local-openai:<model>` |
| Anthropic Messages API (prompt caching) | `tinyinference_llm::providers::anthropic` | `<slug>:<model>` whose endpoint is the first-party Messages API and native tool calling is on |
| Codex OAuth / Responses API | `openai_codex.rs` host OAuth selection plus `tinyinference_llm::providers::openai::codex` metadata | the `openai` cloud slug once Codex OAuth tokens exist |
| Claude Agent SDK subprocess | `tinyagents_harness::providers::claude_agent_sdk` | `claude_agent_sdk` / `claude_agent_sdk:<model>` |
| Claude Code CLI subprocess | `tinyagents_harness::providers::claude_code` (see the [Claude Code provider guide](../../../../../gitbooks/developing/providers/claude-code.md)) | `claude-code:<model>` |

## Calls into

- `tinyinference_llm::model::ChatModel` (`vendor/tinyagents/vendor/tinyinference`):
  the trait every transport implements.
- `crate::config`: cloud-provider schema (`AuthStyle`, slug reservation),
  `Config::claude_agent_sdk`, abstract tier model constants.
- `crate::security::credentials`: auth-profile store for BYOK keys and OAuth
  tokens.
- `crate::agent::tinyagents::{routes, host}`: workload routing and explicit run
  explicit run-thread plumbing consumed while building a managed model.
- `crate::security::live_policy` + `crate::security::egress`: Privacy-Mode
  `LocalOnly` refusal and `EgressDescriptor` emission at the factory chokepoint
  (`factory/access_gates.rs`).
- `crate::inference::host_runtime`: `profile::is_local_provider_string`, Ollama /
  LM Studio base-url resolution for local provider strings.
- `crate::inference::auth_error_registry`: surfaces OpenHuman per-provider auth errors
  back to the UI.
- `crate::core::bus` (`BUS.publish`) / `crate::core::events::DomainEvent`:
  `ops/http_error/auth_failure.rs::publish_backend_session_expired` and
  `openhuman_backend_model.rs` publish `DomainEvent::SessionExpired` when the
  managed backend reports an auth failure, so the credentials layer can
  clear/refresh the session; `ops/http_error/auth_failure.rs` also publishes
  `DomainEvent::ProviderApiKeyRejected` the first time a BYO key is rejected.
- `crate::mcp::server::local` (via the vendored `claude_code` driver), the Claude Code
  provider points the sandboxed `claude` subprocess at the in-process MCP
  server so it can reach OpenHuman's memory/tools over loopback without the
  MCP server inheriting CC's OS jail.

## Called by

`grep -rn 'inference::provider::' crates/openhuman-core/src` shows the main
consumers: the agent harness (`agent/session_host/builder/factory.rs`,
`agent/session_host/runtime*.rs`, `agent/subagent_host/ops/*`,
`agent/tinyagents/host/model_resolver.rs`), `web_chat/session.rs` and
`web_chat/web_errors/` (`classify.rs`, `budget.rs`, `retry.rs`, `timeout.rs`,
`backend_error_code.rs`, `provider_detail.rs`, `response_predicates.rs`), `voice/factory/{helpers,mod}.rs`,
`inference/ops.rs` / `inference/schemas/` /
`inference/http/server.rs`,
`flows/tinyflows/caps/{llm,prompt,agent}.rs`, `cron/scheduler/failure_classification.rs`
(`is_budget_exhausted_message`), and `threads/ops/usage.rs` (`UsageInfo`).

## Sub-modules

- `ops/`: `http_error` (HTTP error
  classification, Sentry routing, `api_error`), `models`
  (`list_configured_models`), `provider_factory` (`ProviderRuntimeOptions`,
  `list_providers`).
  Preserves the original `pub use ops::*` contract split out of a single `ops.rs`.
- Claude Code CLI provider: `tinyagents_harness::providers::claude_code` in `vendor/tinyagents`; see the [Claude Code provider guide](../../../../../gitbooks/developing/providers/claude-code.md).

## Tests

- `factory_tests.rs`, `factory_crate_native_tests.rs`,
  `factory_egress_fallback_tests.rs`, `factory_route_resolution_tests.rs`,
  `factory_test_provider_override_tests.rs`: provider-string parsing, access
  gates, and model construction.
- `ops_tests.rs`, `ops_tests_error_suppression_tests.rs`,
  `ops_tests_models_parsing_tests.rs`, `ops/http_error_tests.rs`, `ops/models_tests.rs`: error
  classification and model listing.
- Claude Code provider tests live with the provider in
  `vendor/tinyagents/crates/tinyagents-harness/src/providers/claude_code/`.
- `openhuman_backend_model_tests.rs`: managed host transport; reusable provider
  builder and Codex tests live in TinyInference and TinyAgents.

Local runtime chat providers (Ollama, LM Studio, MLX, oMLX, local-openai)
and caller-authenticated Claude Code/Agent SDK subprocesses do not require
an OpenHuman session, including when used by channel agents. Claude subprocesses
still contact their external provider and remain subject to the LocalOnly privacy
gate. Managed inference and custom cloud routes retain their session checks.

Flow readiness checks the initial provider role used by a known harness agent,
including node model overrides, custom registry pins, and the session builder's
configured default. It shares the builder's role resolver before deciding whether
a session is required. A separate summarization provider does not determine the
session requirement for that agent.
