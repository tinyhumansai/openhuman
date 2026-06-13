# composio

Backend-proxied (and optionally direct/BYO-key) access to Composio's 1000+ OAuth integrations (Gmail, Notion, GitHub, Slack, Google Calendar, …). The Rust counterpart to the backend routes under `src/routes/agentIntegrations/composio.ts`. In **backend mode** the openhuman backend owns the Composio API key, billing/margin, the toolkit allowlist, HMAC webhook verification, and Socket.IO trigger fan-out — the core never hits the Composio API directly. In **direct mode** (`composio.mode = "direct"`, gated by a user-supplied API key in the encrypted keychain) the core talks to Composio's v3 API with the user's own key against their personal tenant. This domain exposes toolkit/connection/tool/trigger management over JSON-RPC, model-facing agent tools for discovery + execution, an OAuth handoff flow, a persistent trigger-event archive, and a per-action execute pipeline (prepare → retry → error classification).

## Responsibilities

- List toolkits, connections, agent-ready toolkits, and a local capability matrix.
- Begin OAuth handoffs (`authorize`) and delete connections (with optional source-scoped memory cleanup).
- Discover Composio action tool schemas (`list_tools`) and execute actions (`execute`), mode-aware over the backend/direct split.
- Manage triggers: list available/active, create, enable, disable, plus a persistent trigger-event history archive.
- Fetch normalized per-toolkit user profiles and refresh stored identities; dispatch sync passes to per-toolkit providers.
- Gate agent action visibility/execution by per-toolkit user scope preferences (read/write/admin) and curated catalogs.
- Manage the Composio routing mode and the direct-mode API key (`get_mode` / `set_api_key` / `clear_api_key`).
- Classify execute failures into stable error classes; funnel op-layer errors to Sentry under `domain="composio"`.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/composio/mod.rs` | Export-focused module root; re-exports types, ops, schemas, agent tools, trigger-history, and (via `memory_sync::composio`) bus/periodic/providers. |
| `src/openhuman/composio/types.rs` | Serde domain types mirroring backend response envelopes (toolkits, connections, tools, execute, triggers, trigger events/history). Includes drift-tolerant `de_string_or_object` deserializers. |
| `src/openhuman/composio/ops.rs` | RPC-facing `composio_*` operations returning `RpcOutcome<T>`; mode-aware client routing, memory-cleanup targets on delete, provider dispatch for profile/sync, mode + api-key ops, Sentry funnel `report_composio_op_error`. |
| `src/openhuman/composio/schemas.rs` | Controller schemas + `handle_*` handlers; `all_controller_schemas` / `all_registered_controllers`. |
| `src/openhuman/composio/client.rs` | `ComposioClient` (thin HTTP wrapper over `IntegrationClient` for backend routes) + `ComposioClientKind` (Backend/Direct), `create_composio_client`, and direct-mode v3 helpers (`direct_list_connections`, `direct_list_tools`, `direct_execute`, `direct_authorize`). |
| `src/openhuman/composio/tools.rs` | Agent tools (`ComposioListToolkitsTool`, `ComposioListConnectionsTool`, `ComposioAuthorizeTool`, `ComposioListToolsTool`, `ComposioExecuteTool`) + `all_composio_agent_tools`; scope/visibility gating (`resolve_action_scope`, `evaluate_tool_visibility`). |
| `src/openhuman/composio/tools/direct.rs` | Direct-mode Composio tool provider hitting Composio v2/v3 APIs with the user's key (`ComposioTool`, `ComposioAction`, `ComposioConnectedAccount`). |
| `src/openhuman/composio/action_tool.rs` | `ComposioActionTool` — a `Tool` wrapping exactly one Composio action, constructed dynamically when `integrations_agent` is spawned with a toolkit. |
| `src/openhuman/composio/execute_dispatch.rs` | Centralized execute path: prepare args → retry policy (auth/rate-limit) → error mapping; mode-aware over backend/direct. |
| `src/openhuman/composio/execute_prepare.rs` | Local pre-flight argument validation/preparation for action calls. |
| `src/openhuman/composio/auth_retry.rs` | Single-shot retry for the post-OAuth token-propagation gap ("Connection error, try to authenticate"). |
| `src/openhuman/composio/error_mapping.rs` | `ComposioErrorClass` + classifier/formatter so tool failures aren't bucketed as generic gateway 502s (#1797). |
| `src/openhuman/composio/oauth_handoff.rs` | OAuth handoff helpers + Meta (Instagram/Facebook) rate-limit mitigations (#1952); rate-limit error wrapping. |
| `src/openhuman/composio/googlecalendar_args.rs` | Default-args transformer for Google Calendar list/find slugs (timezone/`singleEvents` defaults, #1714). |
| `src/openhuman/composio/identity.rs` | Resolves the connected account username for a toolkit via the provider `fetch_user_profile` path (used by skill preflight identity gate). |
| `src/openhuman/composio/trigger_history.rs` | Persistent JSONL trigger-event archive partitioned by UTC day; global `OnceLock` store (`init_global`/`global`). |
| `src/openhuman/composio/bus.rs` | Compatibility shim re-exporting `memory_sync::composio::bus`. |
| `src/openhuman/composio/periodic.rs` | Compatibility shim re-exporting `memory_sync::composio::periodic` (periodic sync loop). |
| `src/openhuman/composio/providers/mod.rs` | Compatibility shim re-exporting `memory_sync::composio::providers` (native per-toolkit provider registry, curated catalogs, user-scope prefs). |
| `*_tests.rs` | Sibling test suites for each file. |

## Public surface

From `mod.rs` re-exports:

- **Client**: `ComposioClient`, `ComposioActionTool`.
- **Ops**: `cached_active_integrations`, `connected_set_hash`, `fetch_connected_integrations`, `fetch_connected_integrations_status`, `FetchConnectedIntegrationsStatus`, `fetch_toolkit_actions`, `invalidate_connected_integrations_cache`.
- **Schemas**: `all_composio_controller_schemas`, `all_composio_registered_controllers`.
- **Agent tools**: `all_composio_agent_tools`.
- **Identity**: `connection_identity`.
- **Trigger history**: `init_composio_trigger_history`, `global_composio_trigger_history`.
- **Types**: `ComposioConnection`, `ComposioConnectionsResponse`, `ComposioToolkitsResponse`, `ComposioToolSchema`/`ComposioToolFunction`, `ComposioToolsResponse`, `ComposioAuthorizeResponse`, `ComposioExecuteResponse`, `ComposioDeleteResponse`, `ComposioCapability`/`ComposioCapabilitiesResponse`, `ComposioAgentReadyToolkitsResponse`, `ComposioTriggerEvent`/`ComposioTriggerMetadata`, `ComposioTriggerHistoryEntry`/`ComposioTriggerHistoryResult`.
- **Re-exported from `memory_sync::composio`**: `ComposioProvider`, `ProviderContext`, `ProviderUserProfile`, `SyncOutcome`, `SyncReason`, `all_composio_providers`, `get_composio_provider`, `init_default_composio_providers`; `ComposioTriggerSubscriber`, `ComposioConfigChangedSubscriber`, `register_composio_trigger_subscriber`; `record_sync_success`, `start_periodic_sync`.

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

Handlers delegate to `ops.rs`; scope handlers delegate to `providers::user_scopes`. Exports wired into `src/core/all.rs`.

## Agent tools

From `tools.rs` (`all_composio_agent_tools`): `composio_list_toolkits`, `composio_list_connections`, `composio_authorize`, `composio_list_tools`, `composio_execute`. Plus `ComposioActionTool` (one tool per action, spawned for `integrations_agent`) and the direct-mode `ComposioTool` provider. Scope elevation is deliberately NOT an agent tool — the user toggles it in the UI. Visibility/execution is gated by curated catalogs + per-toolkit user-scope prefs and sandbox mode; unparseable slugs default to `Write` (fail-closed).

## Events

Subscribers/handlers live in `memory_sync::composio::bus` (re-exported here):

- **`ComposioTriggerSubscriber`** — reacts to `DomainEvent::ComposioTriggerReceived` (published by `socket::event_handlers` when the backend emits `composio:trigger`); archives to trigger history and drives memory ingestion.
- **`ComposioConnectionCreatedSubscriber`** — reacts to `DomainEvent::ComposioConnectionCreated` (published by `composio_authorize`); eagerly warms the integrations cache.
- **`ComposioConfigChangedSubscriber`** — reacts to `DomainEvent::ComposioConfigChanged` (mode/api-key changes).

Published from `ops.rs`: `DomainEvent::ComposioConnectionCreated` (authorize), `DomainEvent::ComposioConnectionDeleted` (delete), `DomainEvent::ComposioActionExecuted` (execute success/failure, with cost + elapsed).

## Persistence

- **Trigger history** (`trigger_history.rs`): JSONL records under `<workspace>/state/triggers/YYYY-MM-DD.jsonl`, partitioned by UTC day, behind a process-global `OnceLock` store (file-locked via `fs2`; a process-local mutex on Windows). Exposed via `composio.list_trigger_history`.
- **Direct-mode API key**: stored in the encrypted keychain (via `credentials`); never logged/returned.
- **User scope prefs** + **identity facets / sync state**: persisted through the memory layer (`memory`, `memory_store`, `memory_tree`) by the providers in `memory_sync::composio` and by delete-time cleanup in `ops.rs`.
- **Integrations cache**: in-process cache of active connections (`cached_active_integrations` / `invalidate_connected_integrations_cache`), reconciled on each `list_connections`.

## Dependencies

- `crate::openhuman::integrations` — shared `IntegrationClient` (Bearer JWT, timeouts, envelope parsing, proxy) backing backend-mode calls.
- `crate::openhuman::config` — `Config` / `ComposioConfig` (`mode`, `entity_id`), `config::rpc` config loading.
- `crate::openhuman::memory`, `memory_store`, `memory_tree` — memory client + chunk store/tree for ingestion and connection-scoped memory cleanup.
- `crate::openhuman::memory_sync::composio` — the actual home of providers, periodic sync, and bus subscribers (this module re-exports them via shims).
- `crate::openhuman::agent::harness` — sandbox mode (`current_sandbox_mode` / `SandboxMode`) for tool gating.
- `crate::openhuman::tools::traits` — `Tool`, `ToolResult`, `ToolCategory`, `PermissionLevel`, `ToolCallOptions`.
- `crate::openhuman::security` — `SecurityPolicy` / `ToolOperation` for direct-tool gating.
- `crate::openhuman::credentials` — encrypted store for the direct-mode API key.
- `crate::openhuman::context::prompt`, `agent::prompts` — prompt/profile injection of connected identities.
- `crate::core::all` — `ControllerFuture` / `RegisteredController` registry types.
- `crate::core::event_bus` — `DomainEvent`, `publish_global`.
- `crate::core::observability` — Sentry error classification/reporting.
- `crate::rpc` — `RpcOutcome<T>`.

## Used by

- `src/core/all.rs` — registers the controllers.
- `src/openhuman/tools/{mod,ops,schemas}.rs` — wires agent tools into the tool registry.
- `src/openhuman/agent/**` — harness/session/subagent spawning (`integrations_agent`), triage escalation, debug.
- `src/openhuman/socket/event_handlers.rs` — parses `composio:trigger` and publishes `ComposioTriggerReceived`.
- `src/openhuman/heartbeat/planner/collectors.rs`, `subconscious/situation_report` — read connected integrations / calendar.
- `src/openhuman/learning/{linkedin_enrichment,profile_md_renderer}.rs`, `memory/read_rpc.rs`, `memory_tree/score/store.rs` — profile/identity + memory consumers.
- `src/openhuman/skills/preflight.rs` — identity gate via `connection_identity`.
- `src/openhuman/credentials/ops.rs`, `channels/runtime/dispatch.rs`, `src/bin/slack_backfill.rs`.

## Notes / gotchas

- **`bus.rs`, `periodic.rs`, `providers/mod.rs` are compatibility shims** — the real implementations live under `src/openhuman/memory_sync/composio/`. Edit there.
- **Mode-aware routing (#1710)**: `authorize`/`execute`/`list_*` go through `create_composio_client` so a `composio.mode` toggle is honoured per call; direct mode never surfaces backend-tenant data (empty allowlist, user's own connections). Backend-only ops (`delete_connection`, `list_github_repos`, the `triggers/*` family, provider profile/sync) use `resolve_client` and require a backend session token.
- **`ops.rs` is large (~2380 lines)** — only the first ~1363 lines were read in full when authoring this doc; mode/api-key ops and `sync_cache_with_connections` were located by grep at lines ~1574–2354.
- **Error classification matters for the UI**: `execute` may return pre-classified `[composio:error:<class>] …` strings (parsed by `app/src/lib/composio/formatters.ts`); `ops.rs` preserves them rather than re-wrapping.
- **Sentry funnel**: `report_composio_op_error` re-tags op-layer failures under `domain="composio"` with `failure="non_2xx"|"transport"` (+ extracted backend status) so transient 5xx leaks are dropped by `before_send` while genuine bugs surface.
- **Type drift tolerance**: trigger types use `de_string_or_object` / `de_opt_string_or_object` to accept upstream fields that flip between string and object shapes.
