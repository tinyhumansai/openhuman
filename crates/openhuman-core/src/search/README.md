# search

Host policy over the TinySearch module. Provider implementations, tool
schemas, role dispatch, and fallback all live in
[`vendor/tinysearch`](../../../../vendor/tinysearch). This domain decides the
OpenHuman side: which providers are usable for this process, how a search
response reads to the model and to the chat UI, and how a module call becomes
an agent tool.

Callers are the tool registry (`tools/ops.rs`), the settings RPC
(`config/ops/search.rs`), the MCP server catalog, and the module adapter in
[`modules/search/`](../modules/search) that loads and talks to TinySearch over
the bus.

## How it works

### Providers, routes, and roles

TinySearch knows eleven providers (`tinysearch_bus::PROVIDERS`): `exa`, `gemini`,
`gemini_deep_research`, `tinyfish`, `parallel`, `brave`, `querit`, `tavily`,
`seltz`, `searxng`, and `keenable`. Each provider the user enables has a route:

- `managed`: called through the TinyHumans backend, billed to the session or
  API key. Only `exa` and `gemini` support it
  (`tinysearch_bus::BACKEND_PROVIDERS`, re-exported as
  `MANAGED_SEARCH_PROVIDERS`). The backend does not proxy `tinyfish` or
  `parallel`.
- `direct`: the user's own key, or a base URL for SearXNG. Seltz reads its key
  from `[seltz]`, SearXNG from `[searxng].base_url`, everyone else from
  `[search]` credentials. `gemini_deep_research` uses the direct Gemini key.
  Keenable's direct route works with no key (keyless public endpoints,
  rate-limited per IP); a key in `[search.keenable]` raises the limits.

There are three roles, each with an ordered provider list:

| Role | Routed tool | What it returns |
| --- | --- | --- |
| `search` | `web_search_tool` | Ranked results. |
| `answer` | `web_answer_tool` | A grounded answer with citations. `depth: "deep"` uses Gemini Deep Research when a key is set. |
| `contents` | `web_contents_tool` | Page contents for a list of URLs. |

The first usable provider in a role's order serves the call. TinySearch falls
back to the next one on balance, rate-limit, and availability errors, and the
response lists the providers it skipped in `fallback_from`.

### Resolving what is usable

`providers::resolve` walks the catalog and produces one `ResolvedProvider` per
provider: whether the user enabled it, its route, whether it can be managed,
whether a backend credential exists (`managed_available`, from
`resolve_backend_credential`), whether the direct route is configured
(`key_configured`), whether it also works without a key (`key_optional`, only
Keenable), and the result, `usable`. A provider is usable when search is on,
the provider is enabled, and its chosen route is reachable.
`ResolvedProvider::status` turns that into the settings badge
(`Ready`, `Disabled`, `NeedsKey`, `SignInRequired`, `SearchOff`).

`role_order` returns the configured order for a role, or TinySearch's default
(`default_role_providers`), filtered to providers that can serve the role.
`effective_role_providers` intersects that with the usable set.
`modules::search::module_config` turns the same view into the configuration
the module receives.

### A tool call

```text
 tools/ops.rs
   build_search_tools(config)
     modules::search::configured_tool_specs(config)  (sync, from config)
     -> one TinySearchTool per ToolSpec
                 |
                 v  (agent calls e.g. web_search_tool)
 TinySearchTool::execute_with_options
   1. live config (spawn-time copy, or reload for a recorded tool)
   2. provider_signature(config); refuse if already exhausted under it
   3. local_only_search_block (egress policy)  -> error if blocked
   4. modules::search::execute_tool(config, ExecuteToolRequest)
                 |                              (bus call to TinySearch)
        ok       |       err
        v        |        v
 render::render  |  user_facing_error(code)
   text + markdown        UNAVAILABLE -> mark_exhausted_for(signature)
   + metadata
```

Declarations are computed synchronously from config, so a turn's tool list is
stable and needs no module round-trip. Each call re-reads the live config, so a
key added or a provider switched mid-session takes effect on the next call
without rebuilding the session.

### The exhaustion latch

A backend credential alone makes the managed route look reachable. On a
deployment that is offline, firewalled, out of balance, or holding a dead key,
the tool is offered every turn and fails every call. To stop the agent burning
turns on it, `TinySearchTool` records a hash of its own provider view
(`provider_signature`: its role order plus every provider's resolved state)
when the module answers "every provider is unavailable". A repeat call under
the same signature is refused with `SEARCH_EXHAUSTED_MESSAGE`, which tells the
model to stop and answer from what it has. Any change a user could make (a
key, a route, a selection, a login) changes the signature and lets the call
through again. Rate limits, low balance, and rejected arguments do not latch;
each has its own message from `user_facing_error`.

### Rendering

`render::render` builds three things from an `ExecuteToolResponse`:

- Model-facing text with a heading such as `Search results for: <q> (via Exa)`
  or `(via Exa, after Brave)` after a fallback, then the answer, numbered
  results with URL, date, and excerpt (capped at 500 chars), and a `Sources:`
  list. The chat UI's `extractSearchProvider` reads the `(via ...)` marker.
- Optional markdown when the caller prefers it.
- Host-only metadata in `ToolResult::metadata`:
  `{"kind":"web_search", "query", "provider", "role"?, "answer"?,
  "citations"?, "fallback_from"?, "results":[...]}`, with excerpts capped at
  300 chars, which the chat UI renders as a search card.

### Configuration and presentation

Config lives in `[search]` (`config/schema/tools/search.rs`): `providers`,
`roles`, and `presentation`. Presentation modes are `roles` (the default),
`all_tools`, `router`, and `one_provider`. Files from the single-engine era are
migrated on load by `config/schema/tools/search_migrate.rs`. Parallel is kept
as a direct-only provider, and a keyless managed-Parallel selection is dropped.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module declarations. Without the `modules` feature it provides a stub `build_search_tools` that returns nothing. |
| [`providers.rs`](./providers.rs) | `ResolvedProvider`, `ProviderStatus`, `resolve`, `role_order`, `effective_role_providers`, and role key helpers (`ROLES`, `role_key`, `parse_role`). |
| [`render.rs`](./render.rs) | `render`, `subject`, `provider_label`, and the text, markdown, and metadata builders. |
| [`tools.rs`](./tools.rs) | `TinySearchTool` (the `tinytools::Tool` bridge), `build_search_tools`, `user_facing_error`, the exhaustion latch, and the egress check. `modules` feature only. |
| [`bus.rs`](./bus.rs) | `CredentialRefreshSubscriber` (`search::credential_refresh`). `modules` feature only. |

## Key types and entry points

- `build_search_tools(&Config)` ([`tools.rs`](./tools.rs), stub in [`mod.rs`](./mod.rs)) is what the tool
  registry calls. It returns an empty list when search is off or nothing is
  usable.
- `TinySearchTool` (`tools.rs`) wraps one `ToolSpec`. `TinySearchTool::new`
  takes the spawn-time config; `TinySearchTool::recorded` builds a deferred
  instance for a resumed thread that resolves config on each call.
- `providers::resolve` and `providers::effective_role_providers` are the policy
  view the settings RPC, MCP catalog, RPC precheck, and module config share.
- `render::render` is the single place a response becomes a `ToolResult`.
- `user_facing_error` maps a module error prefixed `tinysearch.<code>:` to a
  message the model can act on. The detail after the prefix may echo the
  query, so it is not logged.

## RPC / CLI surface

This module registers no controllers itself. The search controllers live in
the tools domain (`tools/schemas/web_search.rs`) and call into this policy:

| Method | What it does |
| --- | --- |
| `openhuman.tools_web_search` | Run the `search` role. |
| `openhuman.tools_web_answer` | Run the `answer` role. |
| `openhuman.tools_web_contents` | Run the `contents` role. |
| `openhuman.tools_searxng_search` | Provider-pinned SearXNG search. |

The MCP server lists `web_search`, `web_answer`, and `searxng_search` only when
a provider can serve them (`mcp/server/tools/specs.rs`). Search settings are
read and written through `config/ops/search.rs`.

## Boundaries

- Provider HTTP clients, tool schemas, role dispatch, and fallback belong to
  TinySearch (`vendor/tinysearch`, upstream `tinyhumansai/tinysearch`). The
  wire types (`ToolSpec`, `ExecuteToolRequest`, `ExecuteToolResponse`, `Role`,
  `errors`) come from `tinysearch-bus`. Do not redeclare them here.
- Loading the module, its bus proxy, and reinitialization belong to
  `modules/search/`.
- Resumed threads keep their recorded search role tools even when no provider
  is usable now. That rehydration is in `agent/session_host/recorded_tools.rs`.
- The user-facing description is in
  [`gitbooks/features/native-tools/web-search.md`](../../../../gitbooks/features/native-tools/web-search.md).

## Gotchas

- Keep `roles` as the presentation agents run on. Agent tool scopes allowlist
  the routed `web_search_tool`, `web_answer_tool`, and `web_contents_tool`, and
  TinySearch advertises and executes only the current mode's tools. Under
  `all_tools` those agents have no web search at all. Schema v3 moves files the
  v2 migration had put on `all_tools` back to `roles` once; a later explicit
  choice is kept.
- `managed_available` only means a credential exists, not that the backend
  will answer. The exhaustion latch is the runtime correction for that.
- The `modules` feature gates everything that calls the module. Without it the
  domain still compiles `providers` and `render`, but no search tools exist.
- `CredentialRefreshSubscriber` is meant to refresh a loaded module on
  `DomainEvent::CredentialChanged` (domain `auth`) so a login or logout reaches
  managed routes before the next call. At the time of writing nothing
  registers it on the bus, so the refresh only happens on the next search call
  (which reinitializes the module when its config fingerprint changes).

## Tests

Tests sit beside each file ([`providers_tests.rs`](./providers_tests.rs), [`render_tests.rs`](./render_tests.rs),
[`tools_tests.rs`](./tools_tests.rs), [`bus_tests.rs`](./bus_tests.rs)). Module config tests are in
`modules/search/config_tests.rs`.

```bash
cargo test -p openhuman search::
pnpm debug rust search::
```

## Further reading

- [Web search](../../../../gitbooks/features/native-tools/web-search.md)
- [tinysearch submodule](../../../../vendor/tinysearch/README.md)
- [Native tools overview](../../../../gitbooks/features/native-tools/README.md)
