# Skills

Discovery and parsing of agentskills.io-style skills (a directory containing `SKILL.md`/`WORKFLOW.md` with YAML frontmatter and Markdown instructions). Owns scope resolution (Builtin / User / Project / Legacy / Flow), trust-marker enforcement, resource reading, create/install/uninstall, run logging, search, and the agent-tool wrappers over all of it. Skills are surfaced to agents as a compact catalog (`## Installed Skills` in the orchestrator prompt) and launched through the `run_workflow` tool as a separate agent run; skill bodies are not spliced into chat turns. Remote catalog browsing lives in [`catalog/`](catalog/README.md) and run execution lives in [`runtime/`](runtime/README.md); this module owns local metadata only.

## Compile-time gate (`skills` feature, see `mod.rs`)

`pub mod skills` is always compiled — it is a facade. Its behavioural submodules (`ops`, `ops_create`, `ops_discover`, `ops_install`, `ops_parse`, `bundled`, `bus`, `preflight`, `registry`, `run_log`, `schemas`, `search`, `tools`) are gated on the default-ON `skills` Cargo feature; when it is off, [`stub.rs`](stub.rs) takes their place, mirroring **functions only** with no-op/empty bodies (`load_workflow_metadata` → `[]`, `init_workflows_dir` → `Ok(())`, empty controller aggregators) so always-on callers need no `#[cfg]`.

[`types.rs`](types.rs) and [`ops_types.rs`](ops_types.rs) are an ungated carve-out compiled in both directions: they are inert serde/std-only definitions with load-bearing consumers outside this domain — `tinytools` re-exports `ToolResult`/`ToolContent` from `types` as the crate-wide tool-result type, and `Workflow`/`WorkflowFrontmatter`/`WorkflowScope` from `ops_types` appear in always-on agent-harness and prompt signatures. Gating them would take down the tool trait system, MCP, and the Node runtime. The stub therefore re-exports these types verbatim instead of redeclaring them.

`catalog` and `runtime` are themselves facade+stub pairs for the same feature, so their `pub mod` lines stay ungated. `webhooks` is ungated outright — it is not part of the `skills` feature at all and has always-compiled callers under `crates/openhuman-core/src/core/`.

Stub signatures must match the real ones exactly; `cargo check --no-default-features` is the only thing that catches drift.

## Key files

| File | Role |
| --- | --- |
| `ops.rs` | Facade re-exporting `ops_create`/`ops_discover`/`ops_install`/`ops_parse`/`bundled::install_bundled_skills`; the module doc explains scope precedence and the trust marker. |
| `ops_create.rs` | Scaffolds new `WORKFLOW.md`/`SKILL.md` skills on disk from declared `[[inputs]]`. |
| `ops_discover.rs` | Scans workspace, user, bundled, and legacy root directories; resolves scope precedence and collisions; skips symlinked bundle entries; and creates the legacy `<workspace>/skills/` directory. |
| `ops_install.rs` | Facade over submodules `ops_install/fetch.rs`/`ops_install/url_validation.rs`: the hardened HTTPS skill-URL installer (size cap, timeout clamp, non-https/private-IP/non-SKILL.md rejection, GitHub blob→raw normalization). Localhost HTTP installs require `OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP=1` and are for local fixtures only. |
| `ops_parse.rs` | Splits `SKILL.md`/`WORKFLOW.md` into frontmatter + body, builds the resource inventory, reads a single resource. |
| `ops_types.rs` | Ungated carve-out: `Workflow`, `WorkflowFrontmatter`, `WorkflowScope` (`Builtin`, `User`, `Project`, `Legacy`, `Flow`), filename/size constants (`MAX_WORKFLOW_RESOURCE_BYTES = 128 KiB`). |
| `types.rs` | Ungated carve-out: `ToolResult`/`ToolContent`, the crate-wide tool-result content-block types re-exported through `tinytools`. |
| `preflight.rs` | Gates that must pass before the orchestrator boots for a `skills_run` (currently the GitHub gate: Composio GitHub connected, `git` on PATH, `user.name`/`user.email` configured, optional strict identity match). Failures surface as a plain `Err` instead of cryptic orchestrator output. |
| `registry.rs` | A skill is an `AgentDefinition` plus declared `[[inputs]]`, flattened from the same `skill.toml`/`workflow.toml`; `render_inputs_block` renders them into the prompt. Also `prune_legacy_default_workflows`. |
| `run_log.rs` | Per-run streaming logs at `<workspace>/skills/.runs/<skill>_<UTC-ts>_<run>.log`, written live off the agent's `AgentProgress` channel; read back by `read_run_log_slice`/`scan_runs`. |
| `search.rs` | `skill_search` — return a capped projection (id, name, description, scope, tags) for one matching skill instead of serializing the whole catalog, mirroring `tool_search`'s deferred-schema bargain. |
| `tools.rs` | LLM-callable wrappers: `WorkflowListTool` (`list_workflows`), `WorkflowDescribeTool` (`describe_workflow`), `WorkflowReadResourceTool` (`read_workflow_resource`), `WorkflowRecentRunsTool` (`list_workflow_runs`), `WorkflowReadRunLogTool` (`read_workflow_run_log`) are default-enabled; `WorkflowCreateTool` (`create_skill` — `create_workflow` belongs to the flows domain), `WorkflowInstallFromUrlTool` (`install_workflow_from_url`) and `WorkflowUninstallTool` (`uninstall_workflow`) form the default-OFF `workflow_manage` family in `tools/user_filter.rs`. Re-exported through `tools/mod.rs` behind `#[cfg(feature = "skills")]`. Launching a run is a separate tool (`RunWorkflowTool`/`AwaitWorkflowTool` in `agent/tools/run_workflow.rs`). |
| `bundled/` | Skills shipped **inside the binary** (including portable assets embedded by upstream crates such as `tinyflows-copilot`'s `flow-authoring` manual). `install`/`install_bundled_skills` materialise them under `<workspace>/.openhuman/builtin-skills/`; discovery scans that root as `WorkflowScope::Builtin` and only accepts a directory whose bytes still match the compiled bundle (`is_current_materialization`). Lowest scope precedence, so a user/project skill of the same name always wins. Nothing in the boot path calls `install_bundled_skills` today — only `search_tests.rs` does. |
| `bus.rs` | `TriggeredWorkflowIndex` + `TriggeredSkillSubscriber`: indexes skills that declare a `triggers:` list in frontmatter; `ensure_triggered_workflow_subscriber` (called from `channels/runtime/startup/start_channels.rs` and `core/jsonrpc.rs`) subscribes `skills::triggered_skill` on `BUS`. It only logs which skill(s) match a `DomainEvent`; launching an agent session for a match is not implemented here. |
| `schemas/` | Controller schemas and thin handlers, split into `controller_schemas.rs`, `handlers.rs`, `helpers.rs`, `wire_types.rs`. Handlers resolve the workspace through `helpers.rs` (`resolve_workspace_dir`/`resolve_config`: `Config::load_or_init()` under a 30 s timeout, falling back to the default workspace). |
| `stub.rs` | Disabled-feature facade; see above. |
| `catalog/` | `skill_registry` — remote catalog fetch/cache/search, install/uninstall by entry id, the `skill_setup` agent. See [catalog/README.md](catalog/README.md). |
| `runtime/` | `skill_runtime` — start/cancel runs, runtime resolution (Node/Python), the `skill_executor` agent. See [runtime/README.md](runtime/README.md). |
| `webhooks/` | Tunnel routing for skill/agent/echo webhook targets; ungated, unrelated to the `skills` feature. See [webhooks/README.md](webhooks/README.md). |

## RPC surface

Namespace `skills` (`schemas/controller_schemas.rs`, `skills_schemas`): `skills.list`, `skills.run`, `skills.cancel`, `skills.read_resource`, `skills.create`, `skills.install_from_url`, `skills.read_run_log`, `skills.recent_runs`, `skills.describe`, `skills.uninstall`.

Plus the sub-domain namespaces: `skill_registry.*` (`browse`, `search`, `sources`, `categories`, `install`, `uninstall`, `schemas` — see [catalog/README.md](catalog/README.md)) and `skill_runtime.*` (`run`, `cancel`, `recent_runs`, `read_run_log`, `resolve_runtimes`, `schemas` — see [runtime/README.md](runtime/README.md)).

## Calls into

- `crates/openhuman-core/src/config/` — `Config::load_or_init()` for workspace resolution in the RPC handlers; the trust marker is `<workspace>/.openhuman/trust` (`ops_types::TRUST_MARKER`).
- `crates/openhuman-core/src/config/workspace/ops.rs` — calls `skills::init_workflows_dir` during workspace bootstrap.
- `crates/openhuman-core/src/agent/registry/agents/orchestrator/prompt.rs` — renders the `## Installed Skills` catalog, fed by the skill list on `PromptContext` (`agent/session_host/turn/context.rs`).
- `crates/openhuman-core/src/agent/context/channels_prompt.rs` — renders the `## Available Skills` list for channel-driven turns (`agent/prompts/` no longer emits a skills section).
- `crates/openhuman-core/src/core/bus.rs` / `crates/openhuman-core/src/core/events.rs` — `bus.rs` subscribes to `DomainEvent` for triggered skills; `ops_create.rs` and `ops_install/fetch.rs` publish `DomainEvent::WorkflowsChanged` after create/install/uninstall so open sessions refresh their catalog. (`WorkflowLoaded`/`WorkflowStopped`/`WorkflowStartFailed`/`WorkflowExecuted` are declared in `events.rs` but nothing in this module publishes them.)
- `crates/openhuman-core/src/agent/harness/definition.rs` — `registry.rs` flattens `AgentDefinition` fields from `skill.toml`.

## Called by

- `tinytools` — supplies the shared `ToolResult`/`ToolContent` shape directly.
- `crates/openhuman-core/src/agent/harness/fork_context.rs` — fork context propagates injected skills.
- `crates/openhuman-core/src/agent/session_host/turn/context.rs` and `.../turn/tools.rs` — the per-turn `workflows` list handed to `PromptContext`; `refresh_workflows` reloads it from the workspace when a `WorkflowsChanged` event is drained.
- `crates/openhuman-core/src/agent/tools/run_workflow.rs` — the separate `run_workflow`/`AwaitWorkflowTool` launch path.
- `crates/openhuman-core/src/core/all.rs` — controller registry wiring for `skills`, `skill_registry`, and `skill_runtime`.

## Tests

Behavior tests live beside their modules as `*_tests.rs` (e.g. `ops_tests.rs` and its `ops_discovery_tests.rs`, `ops_create_and_url_tests.rs`, `ops_uninstall_tests.rs` siblings, `ops_types_tests.rs`, `ops_create_render_skill_toml_tests_tests.rs`, `ops_discover_include_skills_tests_tests.rs`, `ops_install_install_fetch_tests_tests.rs`, `preflight_tests.rs`, `registry_tests.rs`, `run_log_tests.rs`, `schemas_tests.rs`, `search_tests.rs`, `tools_tests.rs`, `types_tests.rs`, `bus_tests.rs`), wired via `#[path]` from the module they cover.

`e2e_plumbing_tests.rs` and `e2e_run_tests.rs` are mock-LLM end-to-end tests: plumbing (create → registry round-trip, orchestrator turn calling `list_workflows`/`run_workflow`, `await_run_outcome` polling) and run execution (`spawn_workflow_run_background` → terminal `DONE` → `await_run_outcome`, `#[ignore]`d and serial because they set the process-global `OPENHUMAN_WORKSPACE`).

Catalog refresh in a live session (`refresh_workflows`) is covered by `crates/openhuman-core/src/agent/session_host/session_builder_and_listener_tests.rs`.

## Notes

- Per AGENTS.md, skill discovery rejects symlinked bundles — copy skills into the `Harness` workspace rather than symlinking them.
- `WorkflowScope` precedence on name collision (`ops_discover::precedence`), lowest to highest: `Builtin` < `Legacy` < `User` < `Project`. `Flow` is a distinct, non-collision-checked scope: a Flows automation row from `flows.db` surfaced in the same catalogue rather than a `SKILL.md` bundle on disk.
