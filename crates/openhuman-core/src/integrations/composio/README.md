# composio

Backend-proxied (and optionally direct/BYO-key) access to Composio's 1000+ OAuth integrations (Gmail, Notion, GitHub, Slack, Google Calendar, …). The Rust counterpart to the backend routes under `src/routes/agentIntegrations/composio.ts`. In **backend mode** the openhuman backend owns the Composio API key, billing/margin, the toolkit allowlist, HMAC webhook verification, and Socket.IO trigger fan-out. The core never hits the Composio API directly. In **direct mode** (`composio.mode = "direct"`, gated by a user-supplied API key in the encrypted keychain) the core talks to Composio's v3 API with the user's own key against their personal tenant. This domain exposes toolkit/connection/tool/trigger management over JSON-RPC, model-facing agent tools for discovery + execution, an OAuth handoff flow, a persistent trigger-event archive, and a per-action execute pipeline (prepare → retry → error classification).

A third path underlies most of the above: profile fetch and action execution
now go through the loaded `tinyconnectors` module (via
[`module_client.rs`](./module_client.rs)) rather than an in-process engine (`tinymemory` v1.13.4
deleted the old in-process Composio pipeline outright: 72 files, ~18.3k
lines) because reaching a connected account needs a credential this crate
must not hold. `tinyconnectors-bus` is the wire contract for that module
call; see [Module boundary](#module-boundary) below.

## Responsibilities

- List toolkits, connections, agent-ready toolkits, and a local capability matrix.
- Begin OAuth handoffs (`authorize`) and delete connections.
- Discover Composio action tool schemas (`list_tools`) and execute actions (`execute`), mode-aware over the backend/direct split.
- Manage triggers: list available/active, create, enable, disable, plus a persistent trigger-event history archive.
- Fetch normalized per-toolkit user profiles, persist connected identities through the `tinyconnectors` module.
- Gate agent action visibility/execution by per-toolkit user scope preferences (read/write/admin) and curated catalogs from `contract/`.
- Gate late-bound per-action tool calls behind a full live contract on first use (#4853).
- Manage the Composio routing mode and the direct-mode API key (`get_mode` / `set_api_key` / `clear_api_key`), including a BYO-key repeated-401 short-circuit.
- Classify execute failures into stable error classes; funnel op-layer errors to Sentry under `domain="composio"`.

## Module boundary

Every Composio operation (profile fetch, action execution)
now lives in the `tinyconnectors` module, loaded through `crate::modules`
behind the `modules` feature. `module_client.rs` is the single `#[cfg]`
switch for this: rather than feature-gating each of a dozen handlers
individually (twelve chances to gate one differently, with a gates-off
build failing wherever the compiler happens to hit first), the gate lives
once, here, and every handler reads the same either way.

`tinyconnectors-bus` ([`vendor/tinyconnectors/crates/tinyconnectors-bus`](../../../../../vendor/tinyconnectors/crates/tinyconnectors-bus/),
declared in [`crates/openhuman-core/Cargo.toml`](../../../Cargo.toml) lines ~486-496) supplies the member names
(`module_client::methods`) and payload types with no transport and no
behaviour; it compiles none of the connector itself. A gates-off build can
still name a member and match on the contract; it just cannot call one.
`module_client::is_unsupported_by_route` distinguishes "this route does not
offer this operation" (e.g. direct mode has no webhook endpoint for
triggers) from a real transport failure.

## Key files

| File | Role |
| --- | --- |
| [`crates/openhuman-core/src/integrations/composio/mod.rs`](./mod.rs) | Module doc + declarations; re-exports types, ops, schemas, agent tools, trigger-history, and provider/bus types. |
| [`crates/openhuman-core/src/integrations/composio/types.rs`](./types.rs) | Serde domain types mirroring backend response envelopes (toolkits, connections, tools, execute, triggers, trigger events/history). Includes drift-tolerant `de_string_or_object` deserializers. |
| [`crates/openhuman-core/src/integrations/composio/ops/`](./ops/) | RPC-facing `composio_*` operations returning `Outcome<T>`, split by concern (see [Ops layout](#ops-layout)). |
| [`crates/openhuman-core/src/integrations/composio/schemas.rs`](./schemas.rs) + `schemas/` (`registry.rs`, `definitions.rs`, `handlers_identity.rs`, `handlers_tools.rs`, `handlers_connections.rs`, `handlers_triggers.rs`, `params.rs`, `util.rs`) | Controller schemas + `handle_*` handlers; `all_controller_schemas` / `all_registered_controllers` ([`schemas/registry.rs`](./schemas/registry.rs)). |
| [`crates/openhuman-core/src/integrations/composio/client.rs`](./client.rs) + `client/` (`factory.rs`, `direct.rs`) | Route resolution only: `ComposioRoute` (Backend/Direct) and `resolve_composio_route` ([`client/factory.rs`](./client/factory.rs)) keep the mode/credential decision and its user-facing errors on the host; the Composio HTTP client itself is the `tinyconnectors` module. `direct_list_connections` / `direct_list_tools` ([`client/direct.rs`](./client/direct.rs)) are the two direct-mode v3 reads: they run the `tinyconnectors` library's `DirectRoute` in-process (it owns `limit=200`, `toolkit_versions=latest`, repeated `tags=` and the v3 reshaping) behind the host's invalid-key gate (`direct_auth`). |
| `crates/openhuman-core/src/integrations/composio/module_client.rs` | The `tinyconnectors` module bridge: the one `modules`-feature `#[cfg]` switch, `methods` re-exported from `tinyconnectors_bus`, and member-failure classification (`is_unsupported_by_route`, error-prefix peeling). |
| [`crates/openhuman-core/src/integrations/composio/tools.rs`](./tools.rs) + `tools/` (`authorize.rs`, `connect.rs`, `list_connections.rs`, `list_toolkits.rs`, `list_tools.rs`, `execute.rs`, `registry.rs`, `visibility.rs`) | Agent tools (`ComposioListToolkitsTool`, `ComposioListConnectionsTool`, `ComposioAuthorizeTool`, `ComposioConnectTool`, `ComposioListToolsTool`, `ComposioExecuteTool`) + `all_composio_agent_tools`; scope/visibility gating (`resolve_action_scope`, `evaluate_tool_visibility`). |
| `crates/openhuman-core/src/integrations/composio/client/{credential,network,direct}.rs` | The direct (BYO-key) reads. `credential.rs` is the key plus a validated base URL (HTTPS-or-loopback guard); `network.rs` resolves the host's `tool.composio` proxy and TLS policy for the module; `direct.rs` calls the module's stateless `ListConnectionsDirect` / `ListToolsDirect` members (the credential rides on the request, so the configured route is untouched and an unsaved key can be probed) under the host's invalid-key gate (`direct_auth`). Redirects are refused and failures are rendered with the messages users see by the module; the v3 paths and reshaping are `tinyconnectors::client::DirectRoute`. |
| [`crates/openhuman-core/src/integrations/composio/action_tool.rs`](./action_tool.rs) | `ComposioActionTool`: a `Tool` wrapping exactly one Composio action, constructed as a `ToolExposure::Deferred` tool per action of every connected toolkit ([`tools/orchestrator_tools.rs`](../../tools/orchestrator_tools.rs), and [`agent/session_host/recorded_tools.rs`](../../agent/session_host/recorded_tools.rs) when a resumed thread rebuilds its recorded actions), reached by the orchestrator through `tool_search`. |
| [`crates/openhuman-core/src/integrations/composio/contract_gate.rs`](./contract_gate.rs) | Late-bound action contract gate (#4853): on an action's first call this turn, surfaces the full live contract (via `catalog::fetch_live_toolkit_catalog`) as a recoverable tool error before executing, so the retry has the real schema in context. |
| [`crates/openhuman-core/src/integrations/composio/catalog.rs`](./catalog.rs) + `catalog/` (`contract.rs`, `lookups.rs`, `probe.rs`) | Live Composio tool contracts (`fetch_live_toolkit_catalog`, `ToolContract`, in [`catalog/contract.rs`](./catalog/contract.rs)): the unfiltered mode-aware `list_tools` schema (the module's `ListTools` member on the backend route, or direct v3 `direct_list_tools`) plus a bounded read-only response probe (`probe_tool_output_sample`, [`catalog/probe.rs`](./catalog/probe.rs)) when a schema publishes no `output_parameters`. |
| [`crates/openhuman-core/src/integrations/composio/connected_integrations.rs`](./connected_integrations.rs) + `connected_integrations/` (`cache.rs`, `fetch.rs`, `fetch_uncached.rs`) | Cached active-connections lookups (`cached_active_integrations`, `fetch_connected_integrations*`) reconciled against `list_connections`. |
| [`crates/openhuman-core/src/integrations/composio/execute_dispatch.rs`](./execute_dispatch.rs) | Host entry for running one action: applies egress enforcement/disclosure, then calls the module's `EXECUTE` member (prepare → retry → error mapping all run in `tinyconnectors`). Used by the `composio_execute` agent tool, `tools.composio_execute`, the memory host, LinkedIn enrichment and the output probe. |
| [`crates/openhuman-core/src/integrations/composio/googlecalendar_args.rs`](./googlecalendar_args.rs) | Host IANA time-zone lookup; the calendar defaulting itself is `tinyconnectors::execute::apply_calendar_query_defaults` (#1714). |
| [`crates/openhuman-core/src/integrations/composio/identity.rs`](./identity.rs) | Resolves the connected account username for a toolkit via the connector module profile-fetch path (used by skill preflight identity gate). |
| [`crates/openhuman-core/src/integrations/composio/identity_store.rs`](./identity_store.rs) | Persists connected-account identities in `<workspace>/integrations/composio_identities.json`. |
| [`crates/openhuman-core/src/integrations/composio/file_store.rs`](./file_store.rs) | Atomic JSON file helpers for the identity and user-scope files under `<workspace>/integrations/`. With a storage backend configured the same values are `composio_state` documents under the acting agent's scope ([`file_store_documents.rs`](./file_store_documents.rs)) and the files are not used. |
| [`crates/openhuman-core/src/integrations/composio/direct_auth/mod.rs`](./direct_auth/mod.rs) | Direct-mode API-key health tracking: a process-local consecutive-401-failure counter (keyed by a non-logged key fingerprint) that short-circuits repeated invalid-key polling. |
| [`crates/openhuman-core/src/integrations/composio/trigger_history.rs`](./trigger_history.rs) | Lazy host lifecycle owner of a module-side opaque archive handle (`init_global`/`global`). |
| [`crates/openhuman-core/src/integrations/composio/bus.rs`](./bus.rs) + `bus/` | Trigger, connection-created and config-changed subscribers, and their registration. |
| [`crates/openhuman-core/src/integrations/composio/providers/mod.rs`](./providers/mod.rs) | Re-exports the curated catalogs, scope verdicts, identity vocabulary and run types from `contract/`. |
| [`crates/openhuman-core/src/integrations/composio/contract/`](./contract/) | The Composio vocabulary: catalogs, scopes, profiles, run shapes, task shapes. Plain data and pure functions. |
| `*_tests.rs` | Sibling test suites for each file. |

## Ops layout

[`ops/mod.rs`](./ops/mod.rs)'s module doc lists the submodule split:

| Sub-module | Contents |
| --- | --- |
| `error_utils` | `OpResult`, `resolve_client`, `report_composio_op_error`, helpers |
| `toolkits` | `composio_list_toolkits`, `composio_list_capabilities`... |
| `connections` | `composio_list_connections`, `composio_authorize`, `_delete_...`, `active_connection_ids` |
| `tools_ops` | `composio_list_tools` |
| `execute` | `composio_execute` |
| `triggers` | GitHub repos + trigger CRUD + trigger history |
| `providers_ops` | `composio_get_user_profile`, `composio_refresh_all_identities` |
| `direct_mode` | `composio_get_mode`, `composio_set_api_key`, `_clear_...` |
| `user_scopes` | per-toolkit agent scope prefs in `<workspace>/integrations/composio_user_scopes.json` |

There is no periodic loop here, and Composio data is not synced into memory.

## Public surface

From `mod.rs` re-exports:

- **Client**: `ComposioRoute`, `resolve_composio_route`, `ComposioActionTool`.
- **Ops**: `cached_active_integrations`, `cached_active_integrations_including_expired`, `connected_set_hash`, `fetch_connected_integrations`, `fetch_connected_integrations_status`, `FetchConnectedIntegrationsStatus`, `invalidate_connected_integrations_cache`.
- **Prompt type**: `ConnectedIntegration` (re-exported from `crate::agent::prompts::types`).
- **Schemas**: `all_composio_controller_schemas`, `all_composio_registered_controllers`.
- **Agent tools**: `all_composio_agent_tools`.
- **Identity**: `connection_identity`.
- **Trigger history**: `init_composio_trigger_history`, `global_composio_trigger_history`.
- **Types**: `ComposioConnection`, `ComposioConnectionsResponse`, `ComposioToolkitsResponse`, `ComposioToolSchema`/`ComposioToolFunction`, `ComposioToolsResponse`, `ComposioAuthorizeResponse`, `ComposioExecuteResponse`, `ComposioDeleteResponse`, `ComposioCapability`/`ComposioCapabilitiesResponse`, `ComposioAgentReadyToolkitsResponse`, `ComposioTriggerEvent`/`ComposioTriggerMetadata`, `ComposioTriggerHistoryEntry`/`ComposioTriggerHistoryResult`.
- **Contract types**: `ProviderUserProfile` (via `providers`).
- **Bus**: `register_composio_trigger_subscriber`, `ComposioTriggerSubscriber`, `ComposioConnectionCreatedSubscriber`, `ComposioConfigChangedSubscriber`.

## RPC / controllers

Namespace `composio`, exposed as `openhuman.composio_*`:

| Method | Purpose |
| --- | --- |
| `composio.list_toolkits` | Backend allowlist of enabled toolkits (empty in direct mode). |
| `composio.list_capabilities` | Local capability matrix (no signed-in session needed). |
| `composio.list_agent_ready_toolkits` | Toolkit slugs that ship a curated agent catalog (#2283). |
| `composio.list_connections` | Active OAuth connections (mode-aware; reconciles integrations cache). |
| `composio.authorize` | Begin OAuth handoff; returns `connectUrl` + `connectionId`. |
| `composio.delete_connection` | Delete connection and its stored identity facets. |
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
| `composio.get_user_scopes` / `composio.set_user_scopes` | Read/write per-toolkit read/write/admin scope prefs. |
| `composio.get_mode` | Current routing mode + whether a direct-mode key is set (never returns the key). |
| `composio.set_api_key` / `composio.clear_api_key` | Store/clear direct-mode Composio API key (key never logged/returned). |

Handlers delegate to `ops/`; scope handlers delegate to `ops::user_scopes`. Exports wired into [`crates/openhuman-core/src/core/all.rs`](../../core/all.rs).

## Agent tools

From `tools.rs` (`all_composio_agent_tools`, registered only when `agent::subagent_host::user_is_signed_in_to_composio` is true): `composio_list_toolkits`, `composio_list_connections`, `composio_authorize`, `composio_connect` (inline OAuth approval card, #3993), `composio_list_tools`, `composio_execute`. Plus `ComposioActionTool` (one `Deferred` tool per action, found through `tool_search`, gated by `contract_gate.rs` on first call). Scope elevation is deliberately NOT an agent tool; the user toggles it in the UI. Visibility/execution is gated by curated catalogs (`providers::` contract re-exports) + per-toolkit user-scope prefs and sandbox mode; unparseable slugs default to `Write` (fail-closed).

## Events

The subscribers live in `bus/`. All three are registered by one call, `register_composio_trigger_subscriber()`, from [`crates/openhuman-core/src/core/runtime/subscribers.rs`](../../core/runtime/subscribers.rs) after trigger history initialization:

- **`ComposioTriggerSubscriber`**, reacts to `DomainEvent::ComposioTriggerReceived` (published by `platform::socket::event_handlers` when the backend emits `composio:trigger`); archives the event to `trigger_history` and routes it through `agent::triage::run_triage` unless `OPENHUMAN_TRIGGER_TRIAGE_DISABLED`, `composio.triage_disabled`, or `composio.triage_disabled_toolkits` opts out.
- **`ComposioConnectionCreatedSubscriber`**, reacts to `DomainEvent::ComposioConnectionCreated` (published by `composio_authorize`); waits for the connection to go active, invalidates and eagerly warms the integrations cache, then (after onboarding, for toolkits with a native identity provider) fetches the profile.
- **`ComposioConfigChangedSubscriber`**, reacts to `DomainEvent::ComposioConfigChanged` (mode/api-key changes).

Published from `ops/` via `crate::core::bus::BUS.publish` (`crate::core::events::DomainEvent`): `DomainEvent::ComposioConnectionCreated` (authorize), `DomainEvent::ComposioConnectionDeleted` (delete), `DomainEvent::ComposioActionExecuted` (execute success/failure, with cost + elapsed).

## Persistence

- **Trigger history** (`trigger_history.rs`): JSONL records under `<workspace>/state/triggers/YYYY-MM-DD.jsonl`, partitioned by UTC day, written inside the TinyConnectors module (exclusive file lock on append). The host keeps an opaque lease, closes it on sign-out/shutdown, and releases the old lease before switching user directories. Exposed via `composio.list_trigger_history`.
- **Direct-mode API key**: stored in the encrypted keychain (via `credentials`); never logged/returned. `direct_auth/mod.rs` additionally tracks a process-local (non-persisted) consecutive-401 counter for the same key.
- **Connected identities**: `<workspace>/integrations/composio_identities.json` (`identity_store.rs`). **User scope prefs**: `<workspace>/integrations/composio_user_scopes.json` (`ops::user_scopes`). Both written atomically (temp file + rename) under one process-wide lock (`file_store.rs`).
- **Integrations cache**: warmed in the background after app startup/sign-in, then kept for the process lifetime. Connection create/delete, config changes, and a divergent `list_connections` response invalidate it; the change paths eagerly re-warm it. Idle time does not trigger a backend fetch on a chat turn (`connected_integrations.rs`).

## Dependencies

- `crate::integrations`: shared `IntegrationClient` (Bearer JWT, timeouts, envelope parsing, proxy) backing backend-mode calls.
- `crate::config`: `Config` / `ComposioConfig` (`mode`, `entity_id`), `config::rpc` config loading.
- `crate::modules`: loads the `tinyconnectors` native module that `module_client.rs` calls into (behind the `modules` feature).
- `tinyconnectors_bus`: the wire contract (member names, payload types) for the `tinyconnectors` module call surface; a plain dependency, not feature-gated.
- `crate::agent::harness`: sandbox mode (`current_sandbox_mode` / `SandboxMode`) for tool gating; `current_task_recency_window` applied through `tinyconnectors::execute::{apply_window_args, filter_response}` in [`tools/execute.rs`](./tools/execute.rs).
- `tinytools`: `Tool`, `ToolResult`, `ToolCategory`, `PermissionLevel`, `ToolCallOptions`.
- `crate::security`: `SecurityPolicy` / `ToolOperation` for direct-tool gating.
- `crate::security::credentials`: encrypted store for the direct-mode API key.
- `crate::agent::prompts`, `agent::prompts`: prompt/profile injection of connected identities.
- `crate::core::all`: `ControllerFuture` / `RegisteredController` registry types.
- `crate::core::bus` (`BUS`) / `crate::core::events::DomainEvent`: event publish/subscribe.
- `crate::core::observability`: Sentry error classification/reporting.
- `crate::core`: `Outcome<T>`.

## Used by

- `crates/openhuman-core/src/core/all.rs`: registers the controllers.
- `crates/openhuman-core/src/tools/{mod,ops}.rs`, [`tools/schemas/composio.rs`](../../tools/schemas/composio.rs): wires agent tools into the tool registry.
- `crates/openhuman-core/src/core/runtime/subscribers.rs`: at startup initializes trigger history and registers the three bus subscribers.
- `crates/openhuman-core/src/agent/**`: session-host tool assembly (deferred per-action tools, recorded-tool rebuild), triage escalation and debug (e.g. [`agent/subagent_host/`](../../agent/subagent_host/), [`agent/orchestration/tools/`](../../agent/orchestration/tools/), [`agent/debug/mod.rs`](../../agent/debug/mod.rs)).
- [`crates/openhuman-core/src/platform/socket/event_handlers.rs`](../../platform/socket/event_handlers.rs): parses `composio:trigger` and publishes `ComposioTriggerReceived`.
- [`crates/openhuman-core/src/agent/prompts/connected_identities.rs`](../../agent/prompts/connected_identities.rs): renders connected identities into the agent prompt.
- [`crates/openhuman-core/src/skills/preflight.rs`](../../skills/preflight.rs): identity gate via `connection_identity`.
- [`crates/openhuman-core/src/security/credentials/ops/composio.rs`](../../security/credentials/ops/composio.rs): direct-mode API key storage.
- [`crates/openhuman-core/src/channels/runtime/dispatch/routing.rs`](../../channels/runtime/dispatch/routing.rs): channel routing over connected integrations.
- `crates/openhuman-core/src/flows/**` (`ops/catalog.rs`, `ops/connection_ref_gate.rs`, `ops/tool_contract_gate.rs`, [`ops/connections.rs`](./ops/connections.rs), `ops/wiring_warnings.rs`, `ops/approval_manifest.rs`, `ops/builder.rs`, `tinyflows/caps/tools/composio.rs`), workflow builder capability adapters over the catalog.
- `crates/openhuman-core/src/integrations/task_sources/**`: re-exports `NormalizedTask`/`TaskContainer`/`TaskFetchFilter`/`TaskKind` from `providers/mod.rs` (its fetch stage is stubbed since `ComposioProvider::fetch_tasks` went away).
- `crates/openhuman-core/src/modules/{connectors_tests,memory_host}.rs`: module loader tests/wiring for the connector bridge.
- [`crates/openhuman-core/src/core/observability.rs`](../../core/observability.rs): matches `direct_auth::COMPOSIO_INVALID_API_KEY_ANCHOR` / `_USER_MESSAGE` when classifying errors.

## Notes / gotchas

- **Mode-aware routing (#1710)**: the `ops/` layer calls the connector module (`module_client::call*`) for every member, and the module reconciles the backend/direct route from live config on each call, so a `composio.mode` toggle is honoured per call. Two host-side exceptions: `list_connections` in direct mode uses `resolve_composio_route` + `direct_list_connections` (the in-process `DirectRoute` over the host transport, so the host proxy/TLS settings, per-client loopback overrides and the pre-store key probe apply); execution goes through `execute_dispatch.rs`, which is a thin egress gate over the module's `EXECUTE`. Operations a route does not offer (e.g. the toolkit allowlist or triggers in direct mode) come back as a named refusal, recognised by `module_client::is_unsupported_by_route`, and are rendered as empty rather than as an outage. `ops::error_utils::resolve_client` survives only for tests.
- **`ops/` is split by concern** (see [Ops layout](#ops-layout)) rather than one large file.
- **Error classification matters for the UI**: `execute` may return pre-classified `[composio:error:<class>] …` strings (parsed by [`app/src/lib/composio/formatters.ts`](../../../../../app/src/lib/composio/formatters.ts)); [`ops/execute.rs`](./ops/execute.rs) preserves them rather than re-wrapping.
- **Sentry funnel**: `report_composio_op_error` re-tags op-layer failures under `domain="composio"` with `failure="non_2xx"|"transport"` (+ extracted backend status) so transient 5xx leaks are dropped by `before_send` while genuine bugs surface.
- **Type drift tolerance**: trigger types use `de_string_or_object` / `de_opt_string_or_object` to accept upstream fields that flip between string and object shapes.
- **Direct-mode 401 short-circuit** (`direct_auth/mod.rs`): after `DIRECT_INVALID_API_KEY_THRESHOLD` (3) consecutive `401 Invalid API key` responses for the same fingerprinted key, further polls short-circuit with a stable user-facing message instead of re-hitting Composio.
- **Contract gate is per-action, per-turn** (`contract_gate.rs`, #4853): only gates `ComposioActionTool` (the per-action surface the orchestrator reaches through `tool_search`), not the generic `composio_execute` dispatcher, MCP bridges, or Workflow dispatchers; those are tracked as follow-up.
</content>

## Further reading

- [Parent module README](../README.md)
- [Third-party integrations](../../../../../gitbooks/features/integrations/README.md)
- [tinyconnectors](../../../../../vendor/tinyconnectors/README.md)

Connector argument preparation, calendar defaults, task-window filtering and
provider classification use the v0.14.0 module's contract 1.13 operations. The
host supplies configuration and policy context; it links only
`tinyconnectors-bus`. Trigger history opens lazily through `OpenArchive`, with
`RecordTrigger`, `ReadArchive` and `CloseArchive` handling all filesystem work.
Cancelling an archive-open caller does not discard its eventual resource handle.
