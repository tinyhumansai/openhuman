# composio

Backend-proxied (and optionally direct/BYO-key) access to Composio's 1000+ OAuth integrations (Gmail, Notion, GitHub, Slack, Google Calendar, …). The Rust counterpart to the backend routes under `src/routes/agentIntegrations/composio.ts`. In **backend mode** the openhuman backend owns the Composio API key, billing/margin, the toolkit allowlist, HMAC webhook verification, and Socket.IO trigger fan-out — the core never hits the Composio API directly. In **direct mode** (`composio.mode = "direct"`, gated by a user-supplied API key in the encrypted keychain) the core talks to Composio's v3 API with the user's own key against their personal tenant. This domain exposes toolkit/connection/tool/trigger management over JSON-RPC, model-facing agent tools for discovery + execution, an OAuth handoff flow, a persistent trigger-event archive, and a per-action execute pipeline (prepare → retry → error classification).

A third path underlies most of the above: profile fetch, action execution,
and sync now go through the loaded `tinyconnectors` module (via
`module_client.rs`) rather than an in-process engine — `tinymemory` v1.13.4
deleted the old in-process Composio pipeline outright (72 files, ~18.3k
lines) because reaching a connected account needs a credential this crate
must not hold. `tinyconnectors-bus` is the wire contract for that module
call; see [Module boundary](#module-boundary) below.

## Responsibilities

- List toolkits, connections, agent-ready toolkits, and a local capability matrix.
- Begin OAuth handoffs (`authorize`) and delete connections (with optional source-scoped memory cleanup).
- Discover Composio action tool schemas (`list_tools`) and execute actions (`execute`), mode-aware over the backend/direct split.
- Manage triggers: list available/active, create, enable, disable, plus a persistent trigger-event history archive.
- Fetch normalized per-toolkit user profiles, persist identity facets, and drive periodic connection sync through the `tinyconnectors` module.
- Gate agent action visibility/execution by per-toolkit user scope preferences (read/write/admin) and curated catalogs sourced from the `tinymemory-api` contract crate.
- Gate late-bound per-action tool calls behind a full live contract on first use (#4853).
- Manage the Composio routing mode and the direct-mode API key (`get_mode` / `set_api_key` / `clear_api_key`), including a BYO-key repeated-401 short-circuit.
- Classify execute failures into stable error classes; funnel op-layer errors to Sentry under `domain="composio"`.

## Module boundary

Every Composio operation (profile fetch, action execution, connection sync)
now lives in the `tinyconnectors` module, loaded through `crate::modules`
behind the `modules` feature. `module_client.rs` is the single `#[cfg]`
switch for this: rather than feature-gating each of a dozen handlers
individually — twelve chances to gate one differently, with a gates-off
build failing wherever the compiler happens to hit first — the gate lives
once, here, and every handler reads the same either way.

`tinyconnectors-bus` (`vendor/tinyconnectors/crates/tinyconnectors-bus`,
declared in `crates/openhuman-core/Cargo.toml` ~486–496) supplies the member names
(`module_client::methods`) and payload types with no transport and no
behaviour; it compiles none of the connector itself. A gates-off build can
still name a member and match on the contract, it just cannot call one.
`module_client::is_unsupported_by_route` distinguishes "this route does not
offer this operation" (e.g. direct mode has no webhook endpoint for
triggers) from a real transport failure.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/integrations/composio/mod.rs` | Module doc + declarations; re-exports types, ops, schemas, agent tools, trigger-history, and provider/bus types. |
| `crates/openhuman-core/src/integrations/composio/types.rs` | Serde domain types mirroring backend response envelopes (toolkits, connections, tools, execute, triggers, trigger events/history). Includes drift-tolerant `de_string_or_object` deserializers. |
| `crates/openhuman-core/src/integrations/composio/ops/` | RPC-facing `composio_*` operations returning `RpcOutcome<T>`, split by concern (see [Ops layout](#ops-layout)). |
| `crates/openhuman-core/src/integrations/composio/schemas.rs` + `schemas/` (`registry.rs`, `definitions.rs`, `handlers_identity.rs`, `handlers_tools.rs`, `handlers_connections.rs`, `handlers_triggers.rs`, `params.rs`, `util.rs`) | Controller schemas + `handle_*` handlers; `all_controller_schemas` / `all_registered_controllers` (`schemas/registry.rs`). |
| `crates/openhuman-core/src/integrations/composio/client.rs` + `client/` (`connections.rs`, `factory.rs`, `direct.rs`, `execute.rs`, `triggers.rs`) | `ComposioClient` (thin HTTP wrapper over `IntegrationClient` for backend routes, `client/connections.rs`) + `ComposioClientKind` (Backend/Direct), `create_composio_client` (`client/factory.rs`), and direct-mode v3 helpers (`direct_list_connections`, `direct_list_tools`, `direct_execute`, `direct_authorize`, all in `client/direct.rs`). |
| `crates/openhuman-core/src/integrations/composio/module_client.rs` | The `tinyconnectors` module bridge: the one `modules`-feature `#[cfg]` switch, `methods` re-exported from `tinyconnectors_bus`, and member-failure classification (`is_unsupported_by_route`, error-prefix peeling). |
| `crates/openhuman-core/src/integrations/composio/tools.rs` + `tools/` (`authorize.rs`, `connect.rs`, `list_connections.rs`, `list_toolkits.rs`, `list_tools.rs`, `execute.rs`, `registry.rs`, `visibility.rs`) | Agent tools (`ComposioListToolkitsTool`, `ComposioListConnectionsTool`, `ComposioAuthorizeTool`, `ComposioConnectTool`, `ComposioListToolsTool`, `ComposioExecuteTool`) + `all_composio_agent_tools`; scope/visibility gating (`resolve_action_scope`, `evaluate_tool_visibility`). |
| `crates/openhuman-core/src/integrations/composio/tools/direct.rs` + `tools/direct/` (`types.rs`, `connections.rs`, `discovery.rs`, `construction.rs`, `execution.rs`, `http_errors.rs`, `tool_impl.rs`) | Direct-mode Composio tool provider hitting Composio v2/v3 APIs with the user's key (`ComposioTool`, `ComposioAction`, `ComposioConnectedAccount`). |
| `crates/openhuman-core/src/integrations/composio/action_tool.rs` | `ComposioActionTool` — a `Tool` wrapping exactly one Composio action, constructed dynamically when `integrations_agent` is spawned with a toolkit. |
| `crates/openhuman-core/src/integrations/composio/contract_gate.rs` | Late-bound action contract gate (#4853): on an action's first call this turn, surfaces the full live contract (via `catalog::fetch_live_toolkit_catalog`) as a recoverable tool error before executing, so the retry has the real schema in context. |
| `crates/openhuman-core/src/integrations/composio/catalog.rs` + `catalog/` (`contract.rs`, `lookups.rs`, `probe.rs`) | Live Composio tool contracts (`fetch_live_toolkit_catalog`, `ToolContract`, in `catalog/contract.rs`): the unfiltered mode-aware `list_tools` schema (backend `ComposioClient::list_tools` or direct v3 `direct_list_tools`) plus a bounded read-only response probe (`probe_tool_output_sample`, `catalog/probe.rs`) when a schema publishes no `output_parameters`. |
| `crates/openhuman-core/src/integrations/composio/connected_integrations.rs` + `connected_integrations/` (`cache.rs`, `fetch.rs`, `fetch_uncached.rs`) | Cached active-connections lookups (`cached_active_integrations`, `fetch_connected_integrations*`, `fetch_toolkit_actions`) reconciled against `list_connections`. |
| `crates/openhuman-core/src/integrations/composio/execute_dispatch.rs` | Execute path for the `composio_execute` agent tool: prepare args → retry policy (auth/rate-limit) → error mapping over `ComposioClient` / `direct_execute`. The `composio.execute` RPC op and `ComposioActionTool` call the module's `EXECUTE` member through `module_client` instead. |
| `crates/openhuman-core/src/integrations/composio/execute_prepare.rs` | Local pre-flight argument validation/preparation for action calls. |
| `crates/openhuman-core/src/integrations/composio/auth_retry.rs` | Single-shot retry for the post-OAuth token-propagation gap ("Connection error, try to authenticate"). |
| `crates/openhuman-core/src/integrations/composio/error_mapping.rs` | `ComposioErrorClass` + classifier/formatter so tool failures aren't bucketed as generic gateway 502s (#1797). |
| `crates/openhuman-core/src/integrations/composio/oauth_handoff.rs` | OAuth handoff helpers + Meta (Instagram/Facebook) rate-limit mitigations (#1952); rate-limit error wrapping. |
| `crates/openhuman-core/src/integrations/composio/googlecalendar_args.rs` | Default-args transformer for Google Calendar list/find slugs (timezone/`singleEvents` defaults, #1714). |
| `crates/openhuman-core/src/integrations/composio/identity.rs` | Resolves the connected account username for a toolkit via the connector module profile-fetch path (used by skill preflight identity gate). |
| `crates/openhuman-core/src/integrations/composio/identity_store.rs` | Persists Composio-sourced identity facets through the bound memory driver (`MemoryProfile::upsert_provider_facet`); ported from the deleted in-process engine, minus its downstream stability-scoring signal. |
| `crates/openhuman-core/src/integrations/composio/profile_md.rs` | Mirrors managed identity facet blocks into `{workspace_dir}/PROFILE.md` between `<!-- openhuman:<block>:start/end -->` markers, ported verbatim from the deleted engine's `profile_md`. |
| `crates/openhuman-core/src/integrations/composio/task_window.rs` | Recency-window narrowing for Composio task-fetch actions used by `morning_briefing`: best-effort server-side arg injection plus an authoritative client-side post-filter. |
| `crates/openhuman-core/src/integrations/composio/direct_auth/mod.rs` | Direct-mode API-key health tracking: a process-local consecutive-401-failure counter (keyed by a non-logged key fingerprint) that short-circuits repeated invalid-key polling. |
| `crates/openhuman-core/src/integrations/composio/trigger_history.rs` | Persistent JSONL trigger-event archive partitioned by UTC day; global `OnceLock` store (`init_global`/`global`). |
| `crates/openhuman-core/src/integrations/composio/bus.rs` | Compatibility shim re-exporting `crate::memory::sync::composio::bus` (trigger/config-change subscribers still live there). |
| `crates/openhuman-core/src/integrations/composio/periodic.rs` | Host-owned periodic connection sync loop. **Not** a shim: it used to re-export the engine's `memory_sync::composio::periodic`, but tinymemory v1.13.4 deleted that module and its would-be replacement in the `tinymemory-module` never materialized, so this file implements the tick loop directly against `ops::run_sync_within_budget`. |
| `crates/openhuman-core/src/integrations/composio/providers/mod.rs` | Two unrelated halves reunified by one former shim: curated catalogs / scope verdicts / identity vocabulary re-exported from the `tinymemory-api` contract crate (pure data, no provider needed), plus this host's own `slack` RPC layer over the connector module. The old `ComposioProvider` trait and provider registry are gone with the deleted engine. |
| `*_tests.rs` | Sibling test suites for each file. |

## Ops layout

`ops/mod.rs`'s module doc lists the submodule split:

| Sub-module | Contents |
| --- | --- |
| `error_utils` | `OpResult`, `resolve_client`, `report_composio_op_error`, helpers |
| `toolkits` | `composio_list_toolkits`, `composio_list_capabilities`, ... |
| `connections` | `composio_list_connections`, `composio_authorize`, `_delete_...` |
| `memory_cleanup` | Memory-cleanup helpers for connection deletion |
| `tools_ops` | `composio_list_tools` |
| `execute` | `composio_execute` |
| `triggers` | GitHub repos + trigger CRUD + trigger history |
| `providers_ops` | `composio_get_user_profile`, `_refresh_...`, `composio_sync` |
| `direct_mode` | `composio_get_mode`, `composio_set_api_key`, `_clear_...` |
| `user_scopes` | per-toolkit agent scope prefs, over the bound memory driver |

`pass_budget.rs` holds `run_sync_within_budget`, the tinyconnectors-mediated
sync pass shared by every entry point that syncs once per invocation
(periodic tick, manual provider sync, `connection_created`, the Slack ingest
RPC).

## Public surface

From `mod.rs` re-exports:

- **Client**: `ComposioClient`, `ComposioActionTool`.
- **Ops**: `cached_active_integrations`, `cached_active_integrations_including_expired`, `connected_set_hash`, `fetch_connected_integrations`, `fetch_connected_integrations_status`, `FetchConnectedIntegrationsStatus`, `fetch_toolkit_actions`, `invalidate_connected_integrations_cache`.
- **Prompt type**: `ConnectedIntegration` (re-exported from `crate::agent::prompts::types`).
- **Schemas**: `all_composio_controller_schemas`, `all_composio_registered_controllers`.
- **Agent tools**: `all_composio_agent_tools`.
- **Identity**: `connection_identity`.
- **Trigger history**: `init_composio_trigger_history`, `global_composio_trigger_history`.
- **Types**: `ComposioConnection`, `ComposioConnectionsResponse`, `ComposioToolkitsResponse`, `ComposioToolSchema`/`ComposioToolFunction`, `ComposioToolsResponse`, `ComposioAuthorizeResponse`, `ComposioExecuteResponse`, `ComposioDeleteResponse`, `ComposioCapability`/`ComposioCapabilitiesResponse`, `ComposioAgentReadyToolkitsResponse`, `ComposioTriggerEvent`/`ComposioTriggerMetadata`, `ComposioTriggerHistoryEntry`/`ComposioTriggerHistoryResult`.
- **Periodic sync**: `record_sync_success`, `start_periodic_sync` (from `periodic.rs`, host code — see above).
- **Re-exported from `providers::{ProviderUserProfile, SyncOutcome, SyncReason}`** (contract types, `tinymemory_api::composio`), plus `crate::memory::sync::composio::bus::{register_composio_trigger_subscriber, ComposioConfigChangedSubscriber, ComposioTriggerSubscriber}`.

## RPC / controllers

Namespace `composio`, exposed as `openhuman.composio_*`:

| Method | Purpose |
| --- | --- |
| `composio.list_toolkits` | Backend allowlist of enabled toolkits (empty in direct mode). |
| `composio.list_capabilities` | Local capability matrix (no signed-in session needed). |
| `composio.list_agent_ready_toolkits` | Toolkit slugs that ship a curated agent catalog (#2283). |
| `composio.list_connections` | Active OAuth connections (mode-aware; reconciles integrations cache). |
| `composio.authorize` | Begin OAuth handoff; returns `connectUrl` + `connectionId`. |
| `composio.delete_connection` | Delete connection; optional source-scoped memory cleanup. |
| `composio.list_tools` | OpenAI function-calling tool schemas (optional toolkit/tag filter). |
| `composio.execute` | Execute an action slug with `{tool, arguments}`. |
| `composio.list_github_repos` | Repos for an authorized GitHub connection. |
| `composio.create_trigger` | Create a trigger instance for a connection. |
| `composio.list_available_triggers` | Catalog of enableable triggers for a toolkit. |
| `composio.list_triggers` | Currently enabled triggers. |
| `composio.enable_trigger` / `composio.disable_trigger` | Enable / delete a trigger. |
| `composio.list_trigger_history` | Recent archived trigger events + JSONL archive paths. |
| `composio.get_user_profile` | Normalized provider profile for a connection. |
| `composio.refresh_all_identities` | Re-fetch + persist identities for all active connections (#1365). |
| `composio.sync` | Spawn a background provider sync pass (`manual`/`periodic`/`connection_created`). |
| `composio.get_user_scopes` / `composio.set_user_scopes` | Read/write per-toolkit read/write/admin scope prefs. |
| `composio.get_mode` | Current routing mode + whether a direct-mode key is set (never returns the key). |
| `composio.set_api_key` / `composio.clear_api_key` | Store/clear direct-mode Composio API key (key never logged/returned). |

Handlers delegate to `ops/`; scope handlers delegate to `ops::user_scopes`. Exports wired into `crates/openhuman-core/src/core/all.rs`.

## Agent tools

From `tools.rs` (`all_composio_agent_tools`, registered only when `subagent_runner::user_is_signed_in_to_composio` is true): `composio_list_toolkits`, `composio_list_connections`, `composio_authorize`, `composio_connect` (inline OAuth approval card, #3993), `composio_list_tools`, `composio_execute`. Plus `ComposioActionTool` (one tool per action, spawned for `integrations_agent`, gated by `contract_gate.rs` on first call) and the direct-mode `ComposioTool` provider (`tools/direct.rs`). Scope elevation is deliberately NOT an agent tool — the user toggles it in the UI. Visibility/execution is gated by curated catalogs (`providers::` contract re-exports) + per-toolkit user-scope prefs and sandbox mode; unparseable slugs default to `Write` (fail-closed).

## Events

Subscribers/handlers for trigger and config-change events still live in `crate::memory::sync::composio::bus` (re-exported here via `bus.rs`). All three are registered by one call, `register_composio_trigger_subscriber()`, from `crates/openhuman-core/src/core/jsonrpc.rs` (~2158, right after `init_composio_trigger_history`):

- **`ComposioTriggerSubscriber`** — reacts to `DomainEvent::ComposioTriggerReceived` (published by `platform::socket::event_handlers` when the backend emits `composio:trigger`); archives the event to `trigger_history` and routes it through `agent::triage::run_triage` unless `OPENHUMAN_TRIGGER_TRIAGE_DISABLED`, `composio.triage_disabled`, or `composio.triage_disabled_toolkits` opts out.
- **`ComposioConnectionCreatedSubscriber`** — reacts to `DomainEvent::ComposioConnectionCreated` (published by `composio_authorize`); waits for the connection to go active, invalidates and eagerly warms the integrations cache, then runs the initial profile fetch + sync.
- **`ComposioConfigChangedSubscriber`** — reacts to `DomainEvent::ComposioConfigChanged` (mode/api-key changes).

Published from `ops/` via `crate::core::bus::BUS.publish` (`crate::core::events::DomainEvent`): `DomainEvent::ComposioConnectionCreated` (authorize), `DomainEvent::ComposioConnectionDeleted` (delete), `DomainEvent::ComposioActionExecuted` (execute success/failure, with cost + elapsed).

## Persistence

- **Trigger history** (`trigger_history.rs`): JSONL records under `<workspace>/state/triggers/YYYY-MM-DD.jsonl`, partitioned by UTC day, behind a process-global `OnceLock` store (file-locked via `fs2`; a process-local mutex on Windows). Exposed via `composio.list_trigger_history`.
- **Direct-mode API key**: stored in the encrypted keychain (via `credentials`); never logged/returned. `direct_auth/mod.rs` additionally tracks a process-local (non-persisted) consecutive-401 counter for the same key.
- **Identity facets**: written through the bound memory driver via `identity_store.rs` (`MemoryProfile::upsert_provider_facet`) and mirrored into `PROFILE.md` by `profile_md.rs`; **user scope prefs** persist through `ops::user_scopes` over the same bound driver (`crate::memory::binding`).
- **Connection-scoped cleanup**: `ops::memory_cleanup` deletes through `MemorySourceSink::forget_matching` (the `Source` / `SourcePrefix` / `Owner` selectors) rather than through the engine's chunk store (#5560). A driver that does not serve `Sources` is refused per target, and the refusal is reported beside `memory_chunks_deleted` rather than read as a delete of nothing.
- **Integrations cache**: in-process cache of active connections (`cached_active_integrations` / `invalidate_connected_integrations_cache`, `connected_integrations.rs`), reconciled on each `list_connections`.

## Dependencies

- `crate::integrations` — shared `IntegrationClient` (Bearer JWT, timeouts, envelope parsing, proxy) backing backend-mode calls.
- `crate::config` — `Config` / `ComposioConfig` (`mode`, `entity_id`), `config::rpc` config loading.
- `crate::modules` — loads the `tinyconnectors` native module that `module_client.rs` calls into (behind the `modules` feature).
- `tinyconnectors_bus` — the wire contract (member names, payload types) for the `tinyconnectors` module call surface; a plain dependency, not feature-gated.
- `tinymemory_api::composio` — curated catalogs, scope verdicts, identity vocabulary, and task-fetch types (`providers/mod.rs`, `identity_store.rs`, `profile_md.rs`).
- `crate::memory::binding` — the workspace's bound memory driver. Connection-scoped cleanup deletes through `MemorySourceSink::forget_matching`; identity facets and user-scope prefs are also read/written through this binding.
- `crate::memory::sync::composio` — still owns the trigger/config-change bus subscribers (this module re-exports them via `bus.rs`) and the `slack` RPC layer re-exported from `providers/mod.rs`.
- `crate::agent::harness` — sandbox mode (`current_sandbox_mode` / `SandboxMode`) for tool gating; `current_task_recency_window` consumed by `task_window.rs`.
- `tinytools` — `Tool`, `ToolResult`, `ToolCategory`, `PermissionLevel`, `ToolCallOptions`.
- `crate::security` — `SecurityPolicy` / `ToolOperation` for direct-tool gating.
- `crate::security::credentials` — encrypted store for the direct-mode API key.
- `crate::agent::prompts`, `agent::prompts` — prompt/profile injection of connected identities.
- `crate::core::all` — `ControllerFuture` / `RegisteredController` registry types.
- `crate::core::bus` (`BUS`) / `crate::core::events::DomainEvent` — event publish/subscribe.
- `crate::core::observability` — Sentry error classification/reporting.
- `crate::rpc` — `RpcOutcome<T>`.

## Used by

- `crates/openhuman-core/src/core/all.rs` — registers the controllers.
- `crates/openhuman-core/src/tools/{mod,ops}.rs`, `tools/schemas/composio.rs` — wires agent tools into the tool registry.
- `crates/openhuman-core/src/core/jsonrpc.rs` — at startup initializes trigger history and registers the three bus subscribers.
- `crates/openhuman-core/src/channels/runtime/startup/start_channels.rs` (`start_channels`) — the one caller of `start_periodic_sync()`. `core/runtime/services.rs`'s `composio_integration_sync` job only runs `memory::sources::reconcile::ensure_composio_sources`; its comment explains why the periodic loop is not started there.
- `crates/openhuman-core/src/agent/**` — session_host/subagent spawning (`integrations_agent`), triage escalation, debug (e.g. `agent/harness/subagent_runner/`, `agent/orchestration/tools/`, `agent/debug/mod.rs`).
- `crates/openhuman-core/src/platform/socket/event_handlers.rs` — parses `composio:trigger` and publishes `ComposioTriggerReceived`.
- `crates/openhuman-core/src/agent/learning/linkedin_enrichment*.rs`, `agent/learning/profile_md_renderer.rs` — connected-identity enrichment consumers.
- `crates/openhuman-core/src/agent/prompts/connected_identities.rs` — renders connected identities into the agent prompt.
- `crates/openhuman-core/src/skills/preflight.rs` — identity gate via `connection_identity`.
- `crates/openhuman-core/src/security/credentials/ops/composio.rs` — direct-mode API key storage.
- `crates/openhuman-core/src/channels/runtime/dispatch/routing.rs` — channel routing over connected integrations.
- `crates/openhuman-core/src/flows/**` (`ops/catalog.rs`, `ops/connection_ref_gate.rs`, `ops/tool_contract_gate.rs`, `ops/connections.rs`, `ops/wiring_warnings.rs`, `ops/approval_manifest.rs`, `ops/builder.rs`, `tinyflows/caps/tools/composio.rs`) — workflow builder capability adapters over the catalog.
- `crates/openhuman-core/src/integrations/task_sources/**` — re-exports `NormalizedTask`/`TaskContainer`/`TaskFetchFilter`/`TaskKind` from `providers/mod.rs` (its fetch stage is stubbed since `ComposioProvider::fetch_tasks` went away).
- `crates/openhuman-core/src/memory/**` (`ops/sync.rs`, `sources/rpc/registry_crud.rs`/`source_sync.rs`/`status_toolkits.rs`/`apply_all.rs`, `tree/tree/mod.rs`) — memory sync and read paths over Composio-sourced data.
- `crates/openhuman-core/src/modules/{connectors_tests,memory_host}.rs` — module loader tests/wiring for the connector bridge.
- `crates/openhuman-core/src/core/observability.rs` — matches `direct_auth::COMPOSIO_INVALID_API_KEY_ANCHOR` / `_USER_MESSAGE` when classifying errors.

## Notes / gotchas

- **`bus.rs` is a compatibility shim** — trigger/config-change subscribers still live under `crates/openhuman-core/src/memory/sync/composio/`. **`periodic.rs` and `providers/mod.rs` are not shims anymore** — read their own module docs; both were rewritten host-side or reunified from a contract crate after tinymemory v1.13.4 deleted the in-process Composio pipeline they used to glob-import.
- **Mode-aware routing (#1710)**: the `ops/` layer calls the connector module (`module_client::call*`) for every member, and the module reconciles the backend/direct route from live config on each call, so a `composio.mode` toggle is honoured per call. Two host-side exceptions: `list_connections` in direct mode uses `create_composio_client` + `direct_list_connections` (the v3 response mapper and loopback overrides live here), and the `composio_execute` agent tool still dispatches through `execute_dispatch.rs`. Operations a route does not offer (e.g. the toolkit allowlist or triggers in direct mode) come back as a named refusal, recognised by `module_client::is_unsupported_by_route`, and are rendered as empty rather than as an outage. `ops::error_utils::resolve_client` survives only for tests.
- **`ops/` is split by concern** (see [Ops layout](#ops-layout)) rather than one large file; `pass_budget.rs` is the one piece of shared sync-pass plumbing every sync entry point calls through.
- **Error classification matters for the UI**: `execute` may return pre-classified `[composio:error:<class>] …` strings (parsed by `app/src/lib/composio/formatters.ts`); `ops/execute.rs` preserves them rather than re-wrapping.
- **Sentry funnel**: `report_composio_op_error` re-tags op-layer failures under `domain="composio"` with `failure="non_2xx"|"transport"` (+ extracted backend status) so transient 5xx leaks are dropped by `before_send` while genuine bugs surface.
- **Type drift tolerance**: trigger types use `de_string_or_object` / `de_opt_string_or_object` to accept upstream fields that flip between string and object shapes.
- **Direct-mode 401 short-circuit** (`direct_auth/mod.rs`): after `DIRECT_INVALID_API_KEY_THRESHOLD` (3) consecutive `401 Invalid API key` responses for the same fingerprinted key, further polls short-circuit with a stable user-facing message instead of re-hitting Composio.
- **Contract gate is per-action, per-turn** (`contract_gate.rs`, #4853): only gates `ComposioActionTool` (the per-action surface used by `integrations_agent`), not the generic `composio_execute` dispatcher, MCP bridges, or Workflow dispatchers — those are tracked as follow-up.
</content>
