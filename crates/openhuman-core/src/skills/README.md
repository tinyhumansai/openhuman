# skills

Host policy over agentskills.io-style skills. A skill (also called a workflow
in the code and UI) is a directory holding a `SKILL.md` or `WORKFLOW.md` with
YAML frontmatter and Markdown instructions, optional resource folders, and an
optional `skill.toml` or `workflow.toml` sidecar. This folder decides which
directories are scanned and which are trusted, reads bundle resources,
creates, installs and uninstalls skills, logs runs, and wraps all of that as
agent tools and the `skills.*` RPC namespace.

The portable mechanics (parsing, scanning a root, collision rules, safe
resource reads, URL guards, scaffolding) live in the vendored `tinyskills`
crate. Agents see installed skills as a compact `## Installed Skills` catalog
in the orchestrator prompt and launch one with the `run_workflow` tool, which
starts a separate agent run. Skill bodies are never spliced into a chat turn.

## How it works

### Discovery and scope

`load_workflow_metadata(workspace_dir)` ([`ops_discover/api.rs`](./ops_discover/api.rs)) is what most
callers use. It returns a cached `Vec<Workflow>`; on a miss it runs
`discover_filtered` ([`ops_discover/scan.rs`](./ops_discover/scan.rs)), which walks the roots in this
order:

```text
 1. Builtin   <workspace>/.openhuman/builtin-skills/   no trust check,
                                                       bytes must match binary
 2. User      ~/.openhuman/skills/
              ~/.agents/skills/
              ~/.openhuman/workflows/            (current layout)
 3. Project   <workspace>/.openhuman/skills/     only when the marker
              <workspace>/.agents/skills/        <workspace>/.openhuman/trust
              <workspace>/.openhuman/workflows/  exists
 4. Legacy    <workspace>/skills/                back-compat, no trust check
        |
        v
 tinyskills::resolve_collisions_with
   precedence Builtin < Legacy < User < Project
   ties within a scope: last root scanned wins
   Profile scope excluded, id noun "workflow"
        |
        v
 Vec<Workflow>  (cached per workspace until invalidated)
```

The trust marker is presence-only: `is_workspace_trusted` checks that
`<workspace>/.openhuman/trust` exists and ignores its contents. Within a scope
the `workflows/` root is scanned last, so a same-named entry there beats an
older `skills/` copy.

`discover_automations` runs the same scan over the `workflows/` roots only.
The Automations UI uses it so capability skills do not show up as task
templates. `WorkflowScope::Flow` is a separate, non-colliding scope: a row
from the flows database presented in the same catalog by
`flows/catalogue.rs`, not a bundle on disk.

Create, install and uninstall call `invalidate_workflow_metadata_cache` and
publish `DomainEvent::WorkflowsChanged` so open sessions rebuild their
catalog on the next snapshot.

### Creating, installing, uninstalling

- `create_workflow` ([`ops_create.rs`](./ops_create.rs)) picks the root for the requested scope,
  calls `tinyskills::scaffold_bundle`, writes the `workflow.toml` sidecar
  (`render_workflow_toml`) from validated `[[inputs]]`, then re-discovers the
  new skill.
- `install_workflow_from_url` ([`ops_install/fetch.rs`](./ops_install/fetch.rs)) normalizes and
  validates the URL with `tinyskills`' SSRF guards (HTTPS only, no private
  addresses, GitHub blob URLs turned into raw URLs), fetches with `reqwest`
  under a size cap and a clamped timeout, validates the document and writes
  it. Plain HTTP to localhost is accepted only with
  `OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP=1`, for local test fixtures.
- `uninstall_workflow` ([`ops_install/uninstall.rs`](./ops_install/uninstall.rs)) searches the roots in
  order and calls `tinyskills::remove_bundle`, which refuses symlinks and
  anything outside the root.

### Running a skill

Execution lives in [`runtime/`](runtime/README.md). Both the `skills.run` RPC
and the `run_workflow` tool (`agent/tools/run_workflow.rs`) call
`runtime::run_machinery::spawn_workflow_run_background`:

```text
 skills.run RPC / run_workflow tool
        |
        v
 registry::get_workflow          AgentDefinition + [[inputs]] from the sidecar
        |
        v
 registry::missing_required_inputs   reject synchronously
        |
        v
 preflight::run_github_preflight     only when the skill declares [github]
        |
        v
 spawn orchestrator run in background
        |                                   progress events
        +-------------------------------> run_log  <workspace>/skills/.runs/
        v                                           <skill>_<UTC-ts>_<run>.log
 await_run_outcome  (polls the log for a terminal result)
```

[`registry.rs`](./registry.rs) treats a skill as an `AgentDefinition` plus declared inputs,
flattened from the same sidecar, and `render_inputs_block` renders the
provided inputs into the run's prompt. [`preflight.rs`](./preflight.rs) holds gates that must
pass before the orchestrator boots, so a failure reaches the caller as a
plain error rather than confusing agent output. Today that is the GitHub gate:
Composio GitHub connected, `git` on PATH, `user.name` and `user.email` set,
and an optional strict identity match.

### Triggered skills

[`bus.rs`](./bus.rs) indexes skills whose frontmatter declares `triggers:`
(`TriggeredWorkflowIndex`) and registers the `skills::triggered_skill`
subscriber on `BUS`. `ensure_triggered_workflow_subscriber` is called from
`core/runtime/bootstrap.rs` and `channels/runtime/startup/start_channels.rs`.
The subscriber only logs which skills match a `DomainEvent`; launching a
session for a match is not implemented.

### Bundled skills

[`bundled/mod.rs`](./bundled/mod.rs) holds the compiled-in `BUNDLED` table: `desktop-control`
(behind `modules`) and the flows `FLOW_AUTHORING` manual (behind `flows`).
`install_bundled_skills` materializes them under
`<workspace>/.openhuman/builtin-skills/`, and discovery accepts a directory
there only while its bytes still match the binary
(`is_current_materialization`). Nothing on the boot path calls
`install_bundled_skills` today; only tests do.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | The facade and the `skills` feature gate (see Gotchas). |
| [`ops.rs`](./ops.rs) | Re-exports of the create, discover, install and parse entry points. |
| [`ops_types.rs`](./ops_types.rs) | Ungated. OpenHuman names for the `tinyskills` model (`Workflow`, `WorkflowFrontmatter`, `WorkflowScope`), `SKILL_TOML`, `WORKFLOW_TOML`, the trust marker name. |
| [`types.rs`](./types.rs) | Ungated. `ToolResult` and `ToolContent` re-exported from `tinytools` for older import paths. |
| [`ops_discover.rs`](./ops_discover.rs), [`ops_discover/`](./ops_discover/) | `api.rs` (public entry points, metadata cache, trust check), `scan.rs` (root order and scan engine), `resource.rs` (`read_workflow_resource`). |
| [`ops_create.rs`](./ops_create.rs) | Scaffold a new skill with its sidecar. |
| [`ops_install.rs`](./ops_install.rs), [`ops_install/`](./ops_install/) | `fetch.rs` (URL installer: fetches through `tinyskills::fetch_skill_document`, then validates and writes with `install_validated_document`, which catalog installs share), `scan_gate.rs` (supply-chain scan with one re-fetch before refusing), `url_validation.rs`, `uninstall.rs`. |
| [`registry.rs`](./registry.rs) | Skills as agent definitions with inputs; `render_inputs_block`, `prune_legacy_default_workflows`. |
| [`preflight.rs`](./preflight.rs) | Pre-run gates (GitHub). |
| [`run_log.rs`](./run_log.rs) | Per-run streaming logs; `read_run_log_slice`, `scan_runs`. |
| [`search.rs`](./search.rs) | `SkillSearchTool` (`skill_search`): a capped projection of one matching skill instead of the whole catalog. |
| [`tools.rs`](./tools.rs), [`tools/`](./tools/), [`tools_uninstall.rs`](./tools_uninstall.rs) | Agent tool wrappers, split into `read.rs`, `write.rs`, `helpers.rs` and the uninstall tool. |
| [`bus.rs`](./bus.rs) | Triggered-skill index and subscriber. |
| [`bundled/`](./bundled/) | Compiled-in skills and the builtin root. |
| [`schemas/`](./schemas/) | `controller_schemas.rs`, `handlers.rs`, `helpers.rs` (workspace resolution through `Config::load_or_init()` with a 30 second timeout), `wire_types.rs`. |
| [`stub.rs`](./stub.rs) | Facade used when the `skills` feature is off. |
| [`catalog/`](catalog/README.md) | The `tinyskills` registry host (`skill_registry.*`): transport, configuration, paged browse, search and detail, install by entry id, the `skill_setup` agent. |
| [`runtime/`](runtime/README.md) | Run execution (`skill_runtime.*`): start, cancel, recent runs, log reads, Node/Python runtime resolution. |
| [`webhooks/`](webhooks/README.md) | Webhook tunnel routing. Nested here for historical reasons; not part of the `skills` feature. |

## Key types and entry points

- `Workflow`, `WorkflowScope` ([`ops_types.rs`](./ops_types.rs)): the discovered skill and its
  scope (`Builtin`, `Legacy`, `User`, `Project`, `Profile`, `Flow`).
- `load_workflow_metadata`, `discover_workflows`, `discover_automations`,
  `is_workspace_trusted`, `init_workflows_dir` ([`ops_discover/api.rs`](./ops_discover/api.rs)).
  `config/workspace/ops.rs` calls `init_workflows_dir` during workspace
  bootstrap.
- `read_workflow_resource` ([`ops_discover/resource.rs`](./ops_discover/resource.rs)): wraps
  `tinyskills::resolve_skill` and `read_resource`, which enforce traversal,
  symlink, size (`MAX_WORKFLOW_RESOURCE_BYTES`, 128 KiB) and UTF-8 checks.
- `create_workflow`, `install_workflow_from_url`, `uninstall_workflow`
  (re-exported from [`ops.rs`](./ops.rs)).
- `registry::{get_workflow, load_workflows, render_inputs_block}`.
- `run_log::run_log_path` and the readers.

Agent tools (re-exported through `tools/mod.rs` behind the `skills` feature):

| Tool | Struct | Default |
| --- | --- | --- |
| `list_workflows` | `WorkflowListTool` | on |
| `describe_workflow` | `WorkflowDescribeTool` | on |
| `read_workflow_resource` | `WorkflowReadResourceTool` | on |
| `list_workflow_runs` | `WorkflowRecentRunsTool` | on |
| `read_workflow_run_log` | `WorkflowReadRunLogTool` | on |
| `skill_search` | `SkillSearchTool` | on |
| `create_skill` | `WorkflowCreateTool` | off (`workflow_manage`) |
| `install_workflow_from_url` | `WorkflowInstallFromUrlTool` | off (`workflow_manage`) |
| `uninstall_workflow` | `WorkflowUninstallTool` | off (`workflow_manage`) |

The create tool is `create_skill` because `create_workflow` belongs to the
flows domain. Defaults are applied by `tools/user_filter.rs`. Launching a run
is the separate `run_workflow` and `await_workflow` pair.

## RPC surface

Namespace `skills` ([`schemas/controller_schemas.rs`](./schemas/controller_schemas.rs)), wired in `core/all.rs`:

- `skills.list`, `skills.describe`: installed skills and one skill's detail.
- `skills.read_resource`: one bundled resource file.
- `skills.create`, `skills.install_from_url`, `skills.uninstall`: change the
  installed set.
- `skills.run`, `skills.cancel`: start or cancel a background run.
- `skills.recent_runs`, `skills.read_run_log`: run history and log slices.

The subfolders add `skill_registry.*` ([catalog/README.md](catalog/README.md))
and `skill_runtime.*` ([runtime/README.md](runtime/README.md)).

## Boundaries

`vendor/tinyskills` owns everything host-independent. This folder calls it
rather than re-implementing it:

| In `tinyskills` | Here (host policy) |
| --- | --- |
| Frontmatter parsing, bounded document reads, resource inventory, name and description limits, `RESOURCE_DIRS` | Which roots exist, the trust marker, `OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP` |
| `scan_root`, `resolve_collisions_with` (precedence, shadow warnings, `LastWins`) | Root order, `source_format` normalization (`agentskills` to `openhuman`), the metadata cache and its invalidation |
| `resolve_skill`, `read_resource` (traversal, symlink, size, UTF-8 guards) | The `read_workflow_resource` wrapper and the empty `skill_id` check |
| `slugify`, `validate_*`, `scaffold_bundle` (containment, body preservation, `SKILL.md` to `WORKFLOW.md` migration) | Scope-to-root choice, the `workflow.toml` sidecar, `[[inputs]]` validation, re-discovery |
| `remove_bundle` | Root search order, error wording, the `WorkflowsChanged` publish |
| Install URL normalization, the guarded `fetch_skill_document` (HTTPS only, non-public addresses refused, pinned connections, re-validated redirects, size cap, timeout), document validation, `write_installed_document`, `redact_url` | `Retry-After` and rate-limit messages, Sentry reporting |
| `TriggerPattern` grammar and matching | `TriggeredWorkflowIndex` and the bus subscriber |
| Bundled-skill validate, digest, install, `is_current_materialization` | The `BUNDLED` table and the builtin root |

Other owners: the orchestrator prompt renders the catalog
(`agent/registry/agents/orchestrator/prompt.rs`), channel turns render their
own skills list (`agent/context/channels_prompt.rs`), fork context carries the
parent's skill list to sub-agents (`agent/harness/fork_context.rs`), and the
flows domain contributes `Flow`-scope entries (`flows/catalogue.rs`).

## Gotchas

- The `skills` Cargo feature (default on) gates every behavioral submodule.
  `pub mod skills` itself always compiles as a facade: when the feature is off,
  [`stub.rs`](./stub.rs) supplies the functions always-on callers use with empty or no-op
  bodies (`load_workflow_metadata` returns `[]`, `init_workflows_dir` returns
  `Ok(())`, the controller aggregators return nothing), so callers need no
  `#[cfg]`. [`types.rs`](./types.rs) and `ops_types.rs` stay compiled in both builds because
  agent-harness and prompt signatures name them; the stub re-exports them
  rather than redeclaring them. [`catalog`](./catalog) and [`runtime`](./runtime) are facade and stub
  pairs of their own, and [`webhooks`](./webhooks) is ungated. Stub signatures must match
  the real ones exactly, and `cargo check --no-default-features` is the only
  thing that catches drift.
- Skill discovery rejects symlinked bundles. Copy a skill into a library
  agent's `agents/<id>/skills/` rather than linking it.
- Project-scope skills load only with the trust marker. Builtin and legacy
  roots ignore it.

## Tests

Host wiring tests sit beside their modules ([`ops_tests.rs`](./ops_tests.rs) and its
[`ops_discovery_tests.rs`](./ops_discovery_tests.rs), [`ops_create_and_url_tests.rs`](./ops_create_and_url_tests.rs),
[`ops_uninstall_tests.rs`](./ops_uninstall_tests.rs) siblings, [`preflight_tests.rs`](./preflight_tests.rs), [`registry_tests.rs`](./registry_tests.rs),
[`run_log_tests.rs`](./run_log_tests.rs), [`search_tests.rs`](./search_tests.rs), [`tools_tests.rs`](./tools_tests.rs), [`bus_tests.rs`](./bus_tests.rs) and
others). [`e2e_plumbing_tests.rs`](./e2e_plumbing_tests.rs) is the mock-LLM end-to-end test: create and
registry round-trip, an orchestrator turn calling `list_workflows` and
`run_workflow`, and `await_run_outcome` polling. Tests for the portable pieces
live in `vendor/tinyskills/crates/tinyskills/tests/`. Run with
`cargo test -p openhuman skills::` or `pnpm debug rust skills`, and check the
disabled build with `cargo check --no-default-features`.

## Further reading

- [MCP servers and skills](../../../../docs/gitbooks/en/features/integrations/mcp-and-skills.md)
- [tinyskills submodule](../../../../vendor/tinyskills/README.md)
- [Agent harness architecture](../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
