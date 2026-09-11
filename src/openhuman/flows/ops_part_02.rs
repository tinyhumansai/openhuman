
/// Produces host-side, **non-fatal** validation warnings for a graph — today
/// exactly one: "this trigger kind does not fire automatically yet". Returns
/// an empty vec when the trigger fires (`manual`/`schedule`/`app_event`), when
/// the graph has no single resolvable trigger node, or when the trigger has no
/// `trigger_kind` discriminator (a legacy/manual-only graph authored before
/// B2 simply never self-fires — not a warnable surprise, matching
/// `bus::extract_trigger_kind`'s "no automatic binding" treatment).
///
/// This lives host-side (NOT in `tinyflows::validate`, which is host-agnostic
/// and only does structural checks) because "which trigger kinds this host has
/// wired" is an OpenHuman fact, not a property of the portable graph.
pub(crate) fn graph_trigger_warnings(graph: &WorkflowGraph) -> Vec<String> {
    let Some(trigger) = graph.trigger() else {
        return Vec::new();
    };
    let Some(kind_value) = trigger.config.get("trigger_kind") else {
        return Vec::new();
    };
    let kind: TriggerKind = match serde_json::from_value(kind_value.clone()) {
        Ok(k) => k,
        Err(_) => return Vec::new(),
    };
    if trigger_kind_fires(&kind) {
        return Vec::new();
    }
    let label = trigger_kind_label(&kind);
    vec![format!(
        "Trigger kind '{label}' does not fire automatically yet — this flow will be saved and \
         can be enabled, but nothing will run it on its own until that trigger is wired up. Run \
         it manually with flows_run, or switch to a `schedule` or `app_event` trigger."
    )]
}

/// Author-time wiring warnings for Composio `tool_call` nodes: flags every
/// **required** arg (per the action's schema, best-effort cached lookup) that
/// is absent or a literal `null` in `config.args` — the exact mis-wiring that
/// would later fail the run's required-arg preflight.
///
/// Static by design: an arg carrying an `=`-expression counts as wired (only
/// the runtime preflight can tell whether it resolves), a `=`-derived slug is
/// skipped (can't know the action), and native `oh:` tools are skipped (no
/// Composio schema). Best-effort like the runtime preflight — no schema, no
/// warning, never a block.
pub(crate) async fn graph_wiring_warnings(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::flows::tinyflows::caps::{composio_required_args, missing_required_args};

    let mut warnings = Vec::new();
    for node in &graph.nodes {
        if node.kind != tinyflows::model::NodeKind::ToolCall {
            continue;
        }
        let Some(slug) = node.config.get("slug").and_then(Value::as_str) else {
            continue;
        };
        // `=`-derived slugs are resolved at runtime; native tools have no
        // Composio schema to check against.
        if slug.starts_with('=') || slug.starts_with("oh:") {
            continue;
        }
        let Some(required) = composio_required_args(config, slug).await else {
            tracing::debug!(target: "flows", node = %node.id, %slug, "[flows] wiring check: no schema — skipping node");
            continue;
        };
        let args = node.config.get("args").cloned().unwrap_or(Value::Null);
        for missing in missing_required_args(&required, &args) {
            tracing::warn!(
                target: "flows",
                node = %node.id,
                %slug,
                arg = %missing,
                "[flows] wiring check: required arg not wired"
            );
            warnings.push(format!(
                "Node '{}': required arg `{missing}` of `{slug}` is not wired — set \
                 args.{missing}, e.g. \"=nodes.<upstream_id>.item.json.<field>\" (an agent \
                 feeding this value needs an output schema — `output_parser.schema` — so its \
                 fields are addressable).",
                node.id
            ));
        }
    }

    warnings.extend(graph_output_field_warnings(config, graph).await);
    warnings.extend(graph_split_out_path_warnings(config, graph).await);
    warnings
}

/// Author-time WARN (systemic tool-contract fix, Part 2c): any
/// `=nodes.<id>.item.json.data.<field>` binding — anywhere in the graph, not
/// just `tool_call` args — whose `<id>` names a `tool_call` node calling a
/// REAL Composio action with a KNOWN live output schema, but whose `<field>`
/// is not one of that action's real `output_fields`. Also warns (a distinct
/// message) when the binding is missing the `data.` segment entirely — a
/// Composio `tool_call`'s real runtime output always wraps its payload in
/// `data` (`ComposioExecuteResponse`; see
/// [`crate::openhuman::flows::tinyflows::caps::ToolContract::output_fields`]'s doc),
/// so `=nodes.<id>.item.json.<field>` (no `data.`) is GUARANTEED to resolve
/// `null` even when `<field>` names a real output field — that used to be
/// silently accepted here (B1: the exact bug that produces a hollow run).
/// Advisory, not fatal: a binding to an unknown field could still resolve to
/// something useful at runtime for an action whose output schema is
/// incomplete, so this warns rather than rejects — mirroring
/// `graph_wiring_warnings`'s existing required-arg warnings.
///
/// Skipped entirely when the referenced action's output schema is
/// **unknown** (`ToolContract::output_schema` is `None`) — there is nothing
/// real to check the field against, so warning would just be noise (or a
/// false positive for a still-legitimate binding). Also skipped for a
/// binding that dereferences `.item.<field>` without `.json` on an
/// enveloping node — that shape is already a HARD reject in
/// [`validate_binding_resolvability`], not a warning here.
///
/// Also skipped for a binding that addresses the whole payload
/// (`=nodes.<id>.item.json.data`, e.g. as an agent `input_context`) or one
/// of `ComposioExecuteResponse`'s OTHER top-level envelope fields —
/// `successful`, `error`, `costUsd`, `markdownFormatted` — which live
/// alongside `data`, not inside it. `OpenHumanTools::invoke` serializes the
/// whole `ComposioExecuteResponse` verbatim, so these ARE real
/// `.item.json.<x>` fields with no `data.` prefix; flagging them as
/// "missing the `data.` segment" would rewire an already-correct binding to
/// a nonsense path (e.g. suggesting `.item.json.data.successful`).
async fn graph_output_field_warnings(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::flows::tinyflows::caps::fetch_live_toolkit_catalog;
    // Reading a graph's `=`-bindings is the engine's grammar, not this host's:
    // both helpers were a private copy here until the gates moved upstream.
    use tinyflows::bindings::{collect_expressions, parse_node_binding};
    use tinymemory_api::composio::toolkit_from_slug;

    let mut warnings = Vec::new();
    for node in &graph.nodes {
        for (location, expr) in collect_expressions(&node.config) {
            let Some(binding) = parse_node_binding(&expr) else {
                continue;
            };
            if !binding.through_envelope {
                continue;
            }
            let (ref_id, field_path) = (binding.node_id, binding.field_path);
            let Some(ref_node) = graph.node(&ref_id) else {
                continue;
            };
            if ref_node.kind != NodeKind::ToolCall {
                continue;
            }
            let Some(ref_slug) = ref_node.config.get("slug").and_then(Value::as_str) else {
                continue;
            };
            if ref_slug.starts_with('=') || ref_slug.starts_with("oh:") {
                continue;
            }
            let Some(ref_toolkit) = toolkit_from_slug(ref_slug) else {
                continue;
            };
            let Some(catalog) = fetch_live_toolkit_catalog(config, &ref_toolkit).await else {
                continue;
            };
            let Some(contract) = catalog
                .iter()
                .find(|c| c.slug.eq_ignore_ascii_case(ref_slug))
            else {
                continue;
            };
            // B12: a real-output probe (`get_tool_output_sample`) for this
            // exact slug overrides the schema-derived `output_fields` — most
            // relevant for an action whose live listing publishes no output
            // schema at all (e.g. every GitHub action, verified live).
            let contract =
                crate::openhuman::flows::tinyflows::caps::apply_probe_override(contract.clone());
            // Nothing real to check `field_path` against — schema unknown AND
            // no probed output fields either.
            if contract.output_schema.is_none() && contract.output_fields.is_empty() {
                continue;
            }

            // Whole-payload access (`.item.json.data`, e.g. an agent's
            // `input_context`) or one of `ComposioExecuteResponse`'s OTHER
            // top-level envelope fields — these live alongside `data`, not
            // inside it, and are real fields regardless of this action's
            // `output_fields` (see this fn's doc). Not a "missing `data.`"
            // mistake.
            const COMPOSIO_ENVELOPE_METADATA_FIELDS: &[&str] =
                &["successful", "error", "costUsd", "markdownFormatted"];
            if field_path == "data"
                || COMPOSIO_ENVELOPE_METADATA_FIELDS
                    .contains(&field_path.split('.').next().unwrap_or(&field_path))
            {
                continue;
            }

            // A real Composio tool_call's payload is always nested one level
            // under `data` (see this fn's doc) — a binding missing that
            // segment is wrong regardless of whether the rest of the path
            // happens to name a real field.
            let Some(field) = field_path.strip_prefix("data.") else {
                tracing::warn!(
                    target: "flows",
                    node = %node.id,
                    %location,
                    ref_node = %ref_id,
                    ref_slug,
                    %field_path,
                    "[flows] wiring check: downstream binding is missing the Composio `data.` wrapper segment"
                );
                warnings.push(format!(
                    "Node '{}': binding `{location}` (`{expr}`) reads `.item.json.{field_path}` off \
                     tool_call `{ref_id}` (`{ref_slug}`), but a Composio tool_call's real output \
                     wraps its payload in `data` — this resolves null at runtime. Bind via \
                     `=nodes.{ref_id}.item.json.data.{field_path}` instead.",
                    node.id
                ));
                continue;
            };
            let field = field.split('.').next().unwrap_or(field);
            if !contract.output_fields.iter().any(|f| f == field) {
                tracing::warn!(
                    target: "flows",
                    node = %node.id,
                    %location,
                    ref_node = %ref_id,
                    ref_slug,
                    %field,
                    output_fields = ?contract.output_fields,
                    "[flows] wiring check: downstream binding reads a field not in the tool's real output_fields"
                );
                warnings.push(format!(
                    "Node '{}': binding `{location}` (`{expr}`) reads field `{field}` off \
                     tool_call `{ref_id}` (`{ref_slug}`), but that is not one of its real \
                     output fields ({}) — call get_tool_contract {{ slug: \"{ref_slug}\" }} to \
                     see the real output field names.",
                    node.id,
                    contract.output_fields.join(", "),
                ));
            }
        }
    }
    warnings
}

/// Given a Composio action's payload-only `output_schema` (see
/// [`crate::openhuman::flows::tinyflows::caps::ToolContract::output_fields`]'s doc —
/// NEVER includes the runtime `data` envelope) and a `split_out.path`
/// addressed relative to the ENVELOPE (`json.<envelope_field…>`, e.g.
/// `"json.data"` or `"json.data.issues"`), resolves whether the path lands on
/// something that is DEFINITELY not an array.
///
/// `Some(true)` — non-array (an object or scalar): a `split_out` over this
/// path fans out over exactly ONE item, the classic "wrong array path"
/// signal [`graph_split_out_path_warnings`]'s generic enforcement flags.
/// `Some(false)` — array: the path is fine. `None` — the path can't be
/// resolved against the schema at all (an unpublished/unknown nested field,
/// or a path missing the `data.` segment entirely) — stay silent rather than
/// guess; that's a distinct failure mode from "resolves to a non-array".
fn schema_says_path_is_non_array(output_schema: &Value, configured_path: &str) -> Option<bool> {
    let relative = configured_path
        .strip_prefix("json.")
        .unwrap_or(configured_path);
    if relative == "data" {
        // Whole-payload access (`json.data`) — non-array unless the payload's
        // own root schema type is literally "array" (a bare-array response,
        // e.g. a REST endpoint that returns `[...]` directly), in which case
        // `json.data` legitimately IS the real list.
        let ty = output_schema.get("type").and_then(Value::as_str)?;
        return Some(ty != "array");
    }
    let rest = relative.strip_prefix("data.").filter(|r| !r.is_empty())?;
    let mut node = output_schema;
    for seg in rest.split('.') {
        node = node.get("properties")?.get(seg)?;
    }
    let ty = node.get("type").and_then(Value::as_str)?;
    Some(ty != "array")
}

/// Author-time WARN/suggest (systemic tool-contract fix, Part 2d, extended by
/// B12): a `split_out` node whose direct predecessor is a `tool_call` calling
/// a REAL Composio action, checked two ways:
///
/// 1. **KNOWN `primary_array_path`** (see
///    [`crate::openhuman::flows::tinyflows::caps::compute_composio_array_path`] —
///    this already bakes in the `data.` segment Composio's execute-response
///    wrapper adds, so `expected` below comes out `"json.data.<…>"` with no
///    extra handling needed here — and, via
///    [`crate::openhuman::flows::tinyflows::caps::apply_probe_override`], a real
///    `get_tool_output_sample` probe for this slug overrides a schema that
///    never named an array at all): if the configured `config.path` doesn't match the
///    `json.<primary_array_path>` convention, suggest the real path.
/// 2. **UNKNOWN `primary_array_path`, but a KNOWN `output_schema`/probe that
///    proves the configured path is definitely NOT an array** (B12
///    enforcement, "regardless" of whether a correct path can be suggested —
///    catches the class at build time even when nothing to suggest is
///    derivable): warn generically. This is exactly the live bug this fix
///    closes — `GITHUB_LIST_REPOSITORY_ISSUES` publishes no output schema at
///    all, so a builder without a probe guessed the whole-payload
///    `"json.data"`, silently fanning out over ONE item (the `{issues:
///    [...]}` container) instead of the real per-issue list.
///
/// Both are advisory: a mismatched/non-array path degrades the fan-out (or
/// silently produces one item instead of many) rather than crashing.
///
/// Skipped entirely when `split_out`'s predecessor isn't a `tool_call` at all
/// (no envelope/array-path convention applies), or when NEITHER a
/// `primary_array_path` NOR an `output_schema` is known (truly nothing to
/// check against).
async fn graph_split_out_path_warnings(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::flows::tinyflows::caps::{
        apply_probe_override, fetch_live_toolkit_catalog,
    };
    use tinymemory_api::composio::toolkit_from_slug;

    let mut warnings = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::SplitOut {
            continue;
        }
        let configured_path = node.config.get("path").and_then(Value::as_str);

        for edge in graph.edges.iter().filter(|e| e.to_node == node.id) {
            let Some(pred) = graph.node(&edge.from_node) else {
                continue;
            };
            if pred.kind != NodeKind::ToolCall {
                continue;
            }
            let Some(pred_slug) = pred.config.get("slug").and_then(Value::as_str) else {
                continue;
            };
            if pred_slug.starts_with('=') || pred_slug.starts_with("oh:") {
                continue;
            }
            let Some(pred_toolkit) = toolkit_from_slug(pred_slug) else {
                continue;
            };
            let Some(catalog) = fetch_live_toolkit_catalog(config, &pred_toolkit).await else {
                continue;
            };
            let Some(contract) = catalog
                .iter()
                .find(|c| c.slug.eq_ignore_ascii_case(pred_slug))
            else {
                continue;
            };
            // B12: a real-output probe overrides the schema-derived
            // `primary_array_path` for this exact slug when one is cached.
            let contract = apply_probe_override(contract.clone());

            match contract.primary_array_path.as_deref() {
                Some(primary) => {
                    let expected = format!("json.{primary}");
                    if configured_path != Some(expected.as_str()) {
                        tracing::warn!(
                            target: "flows",
                            node = %node.id,
                            predecessor = %pred.id,
                            pred_slug,
                            configured_path,
                            %expected,
                            "[flows] wiring check: split_out.path does not match the predecessor tool's real array path"
                        );
                        let configured_display = configured_path
                            .map(|p| format!("\"{p}\""))
                            .unwrap_or_else(|| "unset".to_string());
                        warnings.push(format!(
                            "Node '{}': split_out.path is {configured_display} but its predecessor \
                             tool_call `{}` (`{pred_slug}`) wraps its real array at `{expected}` — set \
                             config.path to \"{expected}\" to fan out over the actual response list.",
                            node.id, pred.id,
                        ));
                    }
                }
                // No known array anywhere in this action's real output — the
                // generic non-array enforcement is the only thing left that
                // can catch a wrong path here (nothing to suggest, but a
                // known-non-array hit is still a strong signal).
                None => {
                    let Some(cp) = configured_path else { continue };
                    let Some(schema) = contract.output_schema.as_ref() else {
                        continue;
                    };
                    if schema_says_path_is_non_array(schema, cp) == Some(true) {
                        tracing::warn!(
                            target: "flows",
                            node = %node.id,
                            predecessor = %pred.id,
                            pred_slug,
                            configured_path = cp,
                            "[flows] wiring check: split_out.path resolves to a non-array — likely the wrong array path"
                        );
                        warnings.push(format!(
                            "Node '{}': split_out.path is \"{cp}\" but tool_call `{}` (`{pred_slug}`)'s \
                             known real output does not name an array at that path (or names no array \
                             property at all) — this fans out over a single object instead of a real \
                             list. If the action's real output nests the list under a named field (e.g. \
                             `data.issues`), call get_tool_output_sample {{ slug: \"{pred_slug}\" }} to \
                             sample the real response, then re-check with get_tool_contract.",
                            node.id, pred.id,
                        ));
                    }
                }
            }
        }
    }
    warnings
}

// ─────────────────────────────────────────────────────────────────────────────
// Enforcing binding-resolvability gate
// ─────────────────────────────────────────────────────────────────────────────
//
// `graph_wiring_warnings` (above) is advisory — it, and `dry_run_workflow`'s
// null-resolution check, only WARN that a binding resolves null. The gate
// below is the HARD counterpart, run before
// `propose_workflow`/`revise_workflow`/`save_workflow` accept a graph at all,
// so the builder is forced to fix the wiring rather than merely being told.
//
// The analysis is `tinyflows::gates`': every rule in it is a statement about
// the DSL — that agent/tool_call/http_request output is wrapped in
// `{json, text, raw}`, that a `=`-prefixed prose string is not a jq program,
// that an agent produces only what its `output_parser.schema` declares — and
// none of it depends on which host is asking. This host used to carry a
// near-identical private copy, including its own `collect_expressions` and
// `parse_node_binding`; that copy is gone.
//
// Anything that DOES depend on this host's vocabulary — which agent ids
// resolve, which tool slugs exist, which integrations are connected — stays
// here, in the gates that follow.

/// Refuses a graph whose bindings are statically proven unresolvable.
///
/// A non-empty `Vec` rejects; empty passes. Delegates wholesale to
/// [`tinyflows::gates::failures`] — see the section header for why nothing in
/// it is host-specific.
pub(crate) fn validate_binding_resolvability(graph: &WorkflowGraph) -> Vec<String> {
    tinyflows::gates::failures(graph)
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent-ref resolvability gate: an `agent` node's `agent_ref` must name a
// real agent, not the runtime's `RegistryFallback` "unknown agent_ref" case
// ─────────────────────────────────────────────────────────────────────────────
//
// `run_via_registry_fallback` (`tinyflows/caps.rs`) hard-errors mid-run with
// "unknown agent_ref '…'" the moment an `agent` node's `config.agent_ref`
// doesn't resolve to either a harness `AgentDefinition` or a custom agent
// registry entry. Today that is the FIRST time an author finds out — the
// graph proposes, saves, and even passes every other builder gate, then
// fails on the very node whose whole job was to run. This gate moves that
// same check to propose/edit/save time so a broken `agent_ref` is rejected
// before it's ever persisted, using the exact resolution the runtime uses
// (`route_for_agent_ref` + `agent_registry::get_agent`) rather than
// re-implementing it.
//
// A plain `agent` node with NO `agent_ref` is unaffected (and must stay
// that way) — it runs on the default LLM completion (`caps.llm`), never
// touches `OpenHumanAgentRunner`'s routing at all, so there is nothing to
// resolve.

/// Rejects an `agent` node whose `config.agent_ref` would hit the runtime's
/// `RegistryFallback` "unknown agent_ref" hard error mid-run
/// (`run_via_registry_fallback` in `tinyflows/caps.rs`) — a real ref is one
/// that resolves via [`crate::openhuman::flows::tinyflows::caps::route_for_agent_ref`]
/// to a harness [`AgentDefinition`](crate::openhuman::agent::harness::definition::AgentDefinition)
/// (`AgentRoute::Harness`), OR — when it routes to `AgentRoute::RegistryFallback`
/// — resolves to an *enabled*
/// [`AgentRegistryEntry`](crate::openhuman::agent::registry::AgentRegistryEntry)
/// via [`crate::openhuman::agent::registry::get_agent`]. Both are exactly the
/// checks `OpenHumanAgentRunner::run_agent` performs at run time, reused here
/// rather than duplicated so the two planes cannot drift.
///
/// A node with no `agent_ref` (or a blank one) is a plain agent node — it
/// runs on the default LLM completion, never reaches this routing at all —
/// and is skipped, not rejected. A registry lookup failure (e.g. config
/// unavailable) fails OPEN (skipped, logged) like the sibling
/// `validate_connection_refs` gate: this gate must never false-reject a
/// graph because of a transient local read.
///
/// Takes `config` for two reasons. First (CodeRabbit/Codex review on #5114):
/// one-shot contexts — the generic `openhuman <namespace> <function>` CLI
/// dispatcher (`default_state()`, no bootstrap), cron, tests — may reach this
/// gate before the full server bootstrap has called
/// [`AgentDefinitionRegistry::init_global`]. Without it, `route_for_agent_ref`
/// sees an empty global registry and routes EVERY ref — including a real
/// workspace-TOML harness definition — to `RegistryFallback`, which then only
/// checks the custom agent registry and would reject a valid harness agent
/// as unknown. So this gate defensively (re-)initialises the harness registry
/// itself, same idempotent (`OnceLock`) idiom as
/// `memory_goals::enrich::enrich`, before resolving any ref — the two planes
/// (author-time gate and `OpenHumanAgentRunner::run_agent` at actual run
/// time) then always see the same registry state. Second, it threads through
/// to `agent_registry::get_agent`'s underlying config load.
///
/// Also lazily caches the custom agent registry snapshot on the first
/// `RegistryFallback` node (CodeRabbit nitpick): a graph with several
/// non-harness `agent_ref`s previously triggered one `config_rpc::
/// load_config_with_timeout` per node; an all-`Harness`/no-custom-ref graph
/// still never reads it at all.
pub(crate) async fn validate_agent_refs(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;
    use crate::openhuman::agent::registry::AgentRegistryEntry;
    use crate::openhuman::flows::tinyflows::caps::{route_for_agent_ref, AgentRoute};

    let mut errors = Vec::new();
    let mut harness_registry_init_attempted = false;
    let mut custom_registry: Option<Result<Vec<AgentRegistryEntry>, String>> = None;

    for node in &graph.nodes {
        if node.kind != NodeKind::Agent {
            continue;
        }
        let Some(agent_ref) = node.config.get("agent_ref").and_then(Value::as_str) else {
            continue;
        };
        let agent_ref = agent_ref.trim();
        if agent_ref.is_empty() {
            continue;
        }

        if !harness_registry_init_attempted && AgentDefinitionRegistry::global().is_none() {
            harness_registry_init_attempted = true;
            if let Err(e) = AgentDefinitionRegistry::init_global(&config.workspace_dir) {
                tracing::debug!(
                    target: "flows",
                    error = %e,
                    "[flows] agent-ref check: harness registry init failed — falling through \
                     to route resolution with whatever state is available"
                );
            }
        }

        match route_for_agent_ref(agent_ref) {
            AgentRoute::Harness => {
                tracing::debug!(
                    target: "flows",
                    node = %node.id,
                    %agent_ref,
                    "[flows] agent-ref check: resolves to a harness agent definition"
                );
            }
            AgentRoute::RegistryFallback => {
                if custom_registry.is_none() {
                    custom_registry =
                        Some(crate::openhuman::agent::registry::list_agents(true).await);
                }
                match custom_registry.as_ref().expect("just populated") {
                    Ok(entries) => match entries.iter().find(|entry| entry.id == agent_ref) {
                        Some(entry) if entry.enabled => {
                            tracing::debug!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: resolves to an enabled custom agent \
                                 registry entry"
                            );
                        }
                        Some(_disabled) => {
                            tracing::warn!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: agent_ref is registered but disabled — \
                                 rejecting"
                            );
                            errors.push(format!(
                                "Node '{}': `agent_ref` `{agent_ref}` is registered but currently \
                                 disabled — enable it (or pick another agent_ref via \
                                 list_agent_profiles) before this node can run.",
                                node.id
                            ));
                        }
                        None => {
                            tracing::warn!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: unknown agent_ref — neither a harness \
                                 definition nor a custom agent registry entry — rejecting"
                            );
                            errors.push(format!(
                                "Node '{}': `agent_ref` `{agent_ref}` is not a real agent — it \
                                 names neither a built-in agent definition nor a custom agent \
                                 registry entry, and would fail at run time with an \"unknown \
                                 agent_ref\" error. Call list_agent_profiles to see the real, \
                                 selectable agent_ref values.",
                                node.id
                            ));
                        }
                    },
                    Err(e) => {
                        tracing::debug!(
                            target: "flows",
                            node = %node.id,
                            %agent_ref,
                            error = %e,
                            "[flows] agent-ref check: custom agent registry lookup unavailable — \
                             skipping (fail-open)"
                        );
                    }
                }
            }
        }
    }
    errors
}

// ─────────────────────────────────────────────────────────────────────────────
// Inference-readiness check: provider-connectivity (issue B45)
// ─────────────────────────────────────────────────────────────────────────────
//
// An `agent` node's completion (`OpenHumanLlm::complete` in
// `tinyflows/caps.rs`) resolves a chat model exactly like every other
// inference caller in this host — but no check previously inspected that
// resolution at all. `compute_required_connections` only walks `tool_call`
// Composio nodes; an `agent` node's own hard dependency, a working LLM
// provider, went completely unchecked. The confirmed failure: a signed-in
// user whose managed-backend account has no provider API key configured gets
// an HTTP 400 `{"success":false,"error":"API key not configured for
// provider","errorCode":"BAD_REQUEST"}` — but only mid-run, wrapped several
// layers deep as `capability error: graph error: capability error: model
// error: ...`.
//
// **Design correction (judge finding on live run 104aab90 — see git log for
// the full writeup):** this was originally wired in as a HARD author gate
// (`run_builder_gates`), rejecting `propose_workflow`/`edit_workflow`
// outright. In practice that meant a graph whose only problem was "the user
// hasn't configured a provider yet" could never be proposed at all — the
// copilot detected `provider_not_configured`, tried to propose anyway, was
// blocked, and trailed off with no workflow shown to the user. The correct
// placement is:
//
// - **Author time (`build_builder_proposal`)** — ADVISORY ONLY. Authoring
//   always succeeds; `evaluate_inference_readiness`'s result rides along on
//   the proposal payload as `inference_status`/`inference_message` so the UI
//   can render a "connect your provider" nudge next to the built workflow.
// - **Run time (`run_flow_body`)** — HARD gate. A real run (never
//   `dry_run_workflow`, which is a sandbox) checks readiness before invoking
//   the tinyflows engine and fails the run row cleanly with an actionable
//   message if the graph's agent node(s) can't currently reach a provider —
//   see `validate_inference_readiness`'s call site in `run_flow_body`.
//
// Two layers, cheapest and most decisive first:
//
// - **Layer 1 (sync)** — the desktop session itself: signed out
//   (`scheduler_gate::is_signed_out`), or no valid `app-session` JWT
//   (`inference::provider::factory::verify_session_active`, the exact check
//   every custom-provider construction already gates on).
// - **Layer 2 (async, cached)** — one cheap real probe per DISTINCT resolved
//   role (`inference::provider::probe_inference_readiness`) to catch the
//   "signed in but no provider API key configured for this account" class of
//   failure that Layer 1 cannot see. A graph can mix agent nodes pinned to
//   different models (e.g. one `hint:reasoning`, one plain `chat`) that route
//   to different provider configs — each distinct role is probed once, not
//   once per node, and every probe's result caches BOTH a successful and a
//   definitively-negative result for a short TTL — a propose → edit → save →
//   run authoring/run burst hits the network at most once per role per TTL
//   window, whichever way the probe comes back. This is safe to cache
//   negative because `probe_inference_readiness` (and, beneath it,
//   `OpenHumanBackendModel::probe_readiness`) already fails OPEN (`Ok(())`)
//   on anything transient — a timeout, a transport error, a 5xx — so an
//   `Err` reaching this cache is always the definitive, config-level "not
//   ready" signal, never a flake that a naive cache would freeze in place.
//
// [`evaluate_inference_readiness`] is the single evaluation both
// [`validate_inference_readiness`] (the hard gate) and
// [`build_builder_proposal`]'s `inference_status` payload field consume, so
// the gate and the UI-facing status can never disagree.

/// Cache TTL for the Layer-2 managed-backend/role probe.
const INFERENCE_PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Cache key: (workload role, session identity). `config.config_path` stands
/// in for "session identity" — within one desktop process there is exactly
/// one active config/session, so this is stable in production, while
/// distinct `Config`s (as every test builds its own `tempfile` workspace)
/// naturally get distinct cache entries instead of bleeding a cached result
/// from one test/session into an unrelated one. Keying on `role` alone would
/// NOT be enough: two different sessions (or two tests) can both resolve the
/// literal role `"summarization"` to entirely different, unrelated outcomes.
type InferenceProbeCacheKey = (String, std::path::PathBuf);
