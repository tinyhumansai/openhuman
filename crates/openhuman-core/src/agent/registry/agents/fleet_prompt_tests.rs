//! Invariants every built-in agent's rendered prompt must hold.
//!
//! The per-agent `prompt_tests.rs` files pin what one prompt says. These pin
//! what every prompt must never get wrong, over every definition
//! [`load_builtins`] returns, so an agent added tomorrow is covered without
//! anyone writing a test for it.

use super::load_builtins;
use crate::agent::harness::definition::{AgentDefinition, PromptSource, SubagentEntry, ToolScope};
use crate::agent::prompts::{LearnedContextData, PromptContext, ToolCallFormat};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

/// Render `def`'s prompt through its own `PromptSource`, with no tools,
/// connections or user: what is left is what the agent was written to say.
///
/// Each render gets its own workspace: building a prompt seeds identity files
/// into it, and the tests here render in parallel.
fn visible_tool_names(def: &AgentDefinition, definitions: &[AgentDefinition]) -> HashSet<String> {
    let mut visible = match &def.tools {
        ToolScope::Wildcard => HashSet::new(),
        ToolScope::Named(names) => names.iter().cloned().collect(),
    };
    visible.extend(def.extra_tools.iter().cloned());

    for subagent in &def.subagents {
        let SubagentEntry::AgentId(agent_id) = subagent else {
            continue;
        };
        if agent_id == "summarizer" {
            continue;
        }
        let Some(target) = definitions
            .iter()
            .find(|candidate| candidate.id == *agent_id)
        else {
            continue;
        };
        visible.insert(
            target
                .delegate_name
                .clone()
                .unwrap_or_else(|| format!("delegate_{}", target.id)),
        );
    }
    visible
}

fn render(def: &AgentDefinition, definitions: &[AgentDefinition]) -> String {
    let PromptSource::Dynamic(build) = &def.system_prompt else {
        panic!("built-in `{}` must carry a dynamic prompt", def.id);
    };
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let visible = visible_tool_names(def, definitions);
    let ctx = PromptContext {
        workspace_dir: workspace.path(),
        model_name: "test",
        agent_id: &def.id,
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    };
    let body = build(&ctx).unwrap_or_else(|e| panic!("`{}` prompt failed to build: {e}", def.id));
    assert!(
        !body.trim().is_empty(),
        "`{}` rendered an empty prompt",
        def.id
    );
    body
}

/// Every tool name the process can register, plus every packed name (some of
/// which are synthesised hand-offs that no registry lists).
///
/// A name outside this set is not checked. Tools a test config does not
/// register (a disabled browser, say) therefore go unexamined — the invariant
/// can miss, but it cannot misfire.
fn tool_universe() -> BTreeSet<String> {
    registered_tools()
        .iter()
        .map(|t| t.name().to_string())
        .chain(
            crate::tools::toolpacks::all_packed_tool_names()
                .into_iter()
                .map(str::to_string),
        )
        .collect()
}

/// Every tool this build registers, in a throwaway workspace.
fn registered_tools() -> Vec<Box<dyn tinytools::Tool>> {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::config::Config::default()
    };
    let tools = crate::tools::ops::all_tools(
        Arc::new(config.clone()),
        &Arc::new(crate::security::SecurityPolicy::default()),
        crate::security::AuditLogger::disabled(),
        &crate::config::BrowserConfig::default(),
        &crate::config::HttpRequestConfig::default(),
        tmp.path(),
        &std::collections::HashMap::new(),
        &config,
    );
    tools
}

/// Is `tool` on `def`'s belt at all (every tool, for a wildcard)?
fn on_belt(def: &AgentDefinition, tool: &str) -> bool {
    match &def.tools {
        ToolScope::Wildcard => true,
        ToolScope::Named(names) => names.iter().chain(&def.extra_tools).any(|n| n == tool),
    }
}

/// Can `def` call `tool`? Its belt, less what it disallows and what the pack
/// table withholds from it — plus `use_skill`, which the harness advertises
/// whenever the pack table withheld something (`strip_packed_from_visible`).
fn can_call(def: &AgentDefinition, tool: &str, universe: &BTreeSet<String>) -> bool {
    use crate::tools::toolpacks::ops::is_withheld_from;
    if tool == crate::tools::toolpacks::USE_SKILL {
        return universe
            .iter()
            .any(|n| on_belt(def, n) && is_withheld_from(&def.id, n));
    }
    if crate::agent::tinyagents::is_subagent_spawn_or_delegate_tool(tool) {
        return false;
    }
    on_belt(def, tool)
        && !def.disallowed_tools.iter().any(|n| n == tool)
        && !is_withheld_from(&def.id, tool)
}

/// Names from `not_callable` that `text` presents as directly callable.
///
/// Three exemptions, and all are about telling a *route* from a *call*:
///
/// * The generated `## Capabilities not in your tool list` block names withheld
///   tools on purpose — that block is the route, and it is the one sanctioned
///   place to write one. It is removed wholesale before scanning.
/// * A pack **id** may be backticked anywhere, since naming the skill is how a
///   route reads in prose. Two pack ids (`composio`, `goals`) are also tool
///   names inside their own pack, so a bare substring check cannot tell the
///   two apart; routes are always spelled ``skill `<id>` ``, so removing that
///   exact form is what makes the remaining occurrences calls.
/// * A full route — ``skill `<id>`, tool `<name>` ``, the exact spelling the
///   generated block emits — may name the tool it routes to, but only in that
///   form and only under the pack that owns it. A packed name backticked on its
///   own is still a call.
pub(super) fn names_presented_as_callable<'a>(
    text: &str,
    not_callable: impl IntoIterator<Item = &'a str>,
) -> Vec<&'a str> {
    const HEADING: &str = "## Capabilities not in your tool list";
    let mut prose = match text.find(HEADING) {
        Some(start) => {
            // Search for the next heading strictly after this one's own text
            // (`start + HEADING.len()`, not `start + 1`) — both indices land on
            // an ASCII byte, so this can never split a multi-byte UTF-8
            // character or run past `text.len()`.
            let search_from = start + HEADING.len();
            let end = text[search_from..]
                .find("\n## ")
                .map(|i| search_from + i)
                .unwrap_or(text.len());
            format!("{}{}", &text[..start], &text[end..])
        }
        None => text.to_string(),
    };
    for pack in crate::tools::toolpacks::PACKS {
        prose = prose.replace(&format!("skill `{}`", pack.id), "");
        for name in pack.tools {
            prose = prose.replace(&format!("skill `{}`, tool `{name}`", pack.id), "");
        }
    }
    // The workflow pack is feature-gated, but its route syntax is still
    // intentional prose when that pack is unavailable in this build.
    for route in [
        "skill `workflows`, tool `build_workflow`",
        "skill `workflows`, tool `discover_workflows`",
    ] {
        prose = prose.replace(route, "");
    }
    // The generated workflow guide can also mention the routed tool on its
    // own after the pack-specific text has been elided.
    prose = prose
        .replace("`build_workflow`", "")
        .replace("`discover_workflows`", "");
    not_callable
        .into_iter()
        .filter(|name| prose.contains(&format!("`{name}`")))
        .collect()
}

/// A **Deferred** row is only honest while its tool really is deferred and the
/// prompt really gives the route. Pin both, so a tool promoted onto the belt,
/// or a prompt that drops the `tool_search` hint, fails here instead of
/// leaving a stale excuse in [`KNOWN_UNCALLABLE`].
#[test]
fn deferred_rows_name_deferred_tools_the_prompt_routes_through_tool_search() {
    let tools = registered_tools();
    let defs = load_builtins().expect("built-ins load");
    let deferred = KNOWN_UNCALLABLE
        .iter()
        .filter(|(_, _, why)| why.starts_with("`ToolExposure::Deferred`"));
    for (agent, tool, _) in deferred {
        // Feature-gated out of this build: the row cannot fire either.
        let Some(registered) = tools.iter().find(|t| t.name() == *tool) else {
            continue;
        };
        assert_eq!(
            registered.exposure(),
            tinytools::ToolExposure::Deferred,
            "`{tool}` is listed as Deferred for `{agent}` but is not deferred"
        );
        let def = defs
            .iter()
            .find(|d| d.id == *agent)
            .unwrap_or_else(|| panic!("Deferred row names unknown agent `{agent}`"));
        let prompt = render(def, &defs);
        assert!(
            prompt.contains("`tool_search`") && prompt.contains(&format!("`{tool}`")),
            "`{agent}`'s prompt must name `{tool}` with its route, `tool_search`"
        );
    }
}

/// Hits [`every_prompt_names_only_tools_its_agent_can_call`] tolerates, as
/// `(agent, tool, why)`; agent `*` matches any agent.
///
/// A ratchet, not an allowlist: an entry that no longer fires fails the test
/// too, so fixing a prompt forces deleting its row. Two kinds of row:
///
/// * **Real** — the prompt teaches a call the agent cannot make. Each is a
///   defect waiting on a prompt or belt fix; none may be added.
/// * **Collision** — the backticked word is also a tool name but is used as
///   something else (a node kind, an argument, an example). No fix is owed.
/// * **Deferred** — a `ToolExposure::Deferred` tool the prompt names together
///   with its route, `tool_search`, which makes it callable by name afterwards.
///   It is off the belt by design, so no fix is owed.
const KNOWN_UNCALLABLE: &[(&str, &str, &str)] = &[
    // Real.
    (
        "morning_briefing",
        "composio_list_connections",
        "withheld by the `composio` pack",
    ),
    (
        "morning_briefing",
        "composio_list_tools",
        "withheld by the `composio` pack",
    ),
    (
        "morning_briefing",
        "composio_execute",
        "withheld by the `composio` pack",
    ),
    (
        "context_scout",
        "list_workflows",
        "on its belt but withheld by the `workflows` pack",
    ),
    (
        "skill_executor",
        "describe_workflow",
        "step 1 of its procedure; on its belt but withheld by the `workflows` pack",
    ),
    // Collision.
    (
        "context_scout",
        "run_workflow",
        "names the orchestrator's call, not its own",
    ),
    (
        "scheduler_agent",
        "schedule",
        "the `schedule` argument of `cron_add`",
    ),
    (
        "summarizer",
        "file_read",
        "an example of a payload's source tool",
    ),
    ("workflow_builder", "http_request", "a flow node kind"),
    ("workflow_builder", "memory", "a flow node kind"),
    ("workflow_builder", "schedule", "a flow trigger field"),
    ("workflow_builder", "shell", "a flow node kind"),
    (
        "workflow_builder",
        "flow_memory_recall",
        "node-contract note on what a memory node shares",
    ),
    (
        "workflow_builder",
        "flow_memory_remember",
        "node-contract note on what a memory node shares",
    ),
    ("flow_discovery", "http_request", "a flow node kind"),
    ("flow_discovery", "schedule", "a flow trigger field"),
    // Deferred.
    (
        "orchestrator",
        "desktop_goal",
        "`ToolExposure::Deferred`; the prompt routes it through `tool_search`",
    ),
];

/// A prompt must never teach a call the agent cannot make.
///
/// The static markdown once named fifteen tools and ten of them were packed,
/// so the prompt taught a call the model could not make: it emits the name,
/// the harness answers "unknown tool", and the iteration is spent. This is
/// that rule over every built-in, against each one's own callable surface.
#[test]
fn every_prompt_names_only_tools_its_agent_can_call() {
    let universe = tool_universe();
    let known = |agent: &str, tool: &str| {
        KNOWN_UNCALLABLE
            .iter()
            .any(|(a, t, _)| (*a == "*" || *a == agent) && *t == tool)
    };
    let defs = load_builtins().expect("built-ins load");
    // A feature-gated agent absent from this build cannot fire its rows.
    let loaded: HashSet<&str> = defs.iter().map(|d| d.id.as_str()).collect();
    let mut fired = HashSet::new();
    let mut violations = Vec::new();
    for def in &defs {
        let not_callable = universe
            .iter()
            .map(String::as_str)
            .filter(|name| !can_call(def, name, &universe));
        for tool in names_presented_as_callable(&render(def, &defs), not_callable) {
            if known(&def.id, tool) {
                fired.insert((def.id.clone(), tool));
            } else {
                violations.push(format!("{}: `{tool}`", def.id));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "prompts name tools their agent cannot call — take them off the prompt, put \
         them on the belt, or route them through `use_skill`:\n{}",
        violations.join("\n")
    );
    let stale: Vec<_> = KNOWN_UNCALLABLE
        .iter()
        .filter(|(a, _, _)| *a == "*" || loaded.contains(a))
        .filter(|(a, t, _)| {
            !fired
                .iter()
                .any(|(fa, ft)| (*a == "*" || a == fa) && t == ft)
        })
        .collect();
    assert!(
        stale.is_empty(),
        "fixed — delete these rows from KNOWN_UNCALLABLE: {stale:?}"
    );
}

/// Agents whose prompt defers to the rendered tool list instead of naming a
/// tool; [`every_prompt_names_at_least_one_tool_it_can_call`] skips them.
const NAMES_NO_TOOL: &[&str] = &["tools_agent", "tool_maker", "critic", "archivist"];

const SKILL_SETUP_NAME: Option<&str> = if cfg!(feature = "skills") {
    Some("skill_setup")
} else {
    None
};

/// An agent with a belt must be told about at least one tool on it.
///
/// Catches a belt narrowed out from under its prompt, which otherwise reads
/// as an improvement: the tool bytes fall and nothing else moves.
/// Tools a prompt names and the belt carries, but which **do not exist in this
/// build at all** — compiled out by a Cargo feature rather than withheld by
/// policy.
///
/// The guard below iterates `universe`, so a tool absent from it is never
/// examined: the prompt can name it, the belt can carry it, and the agent still
/// reads as naming nothing it can call. That is indistinguishable in the
/// assertion output from the defect this test exists to catch — a belt narrowed
/// out from under its prompt — and the confusion is not hypothetical. It
/// produced openhuman#6507: a product defect filed against `skill_creator`'s
/// prompt and attributed to a PR, retracted once the tools turned out simply
/// not to be compiled.
///
/// `is_withheld_from` is real code that produces this exact symptom under
/// different circumstances, which is why the wrong explanation survived
/// scrutiny — it was correct about a situation that did not apply.
fn belt_tools_absent_from_build(
    def: &AgentDefinition,
    prompt: &str,
    universe: &BTreeSet<String>,
) -> Vec<String> {
    let ToolScope::Named(names) = &def.tools else {
        // A wildcard belt carries whatever exists, so nothing it names can be
        // "absent from the belt's perspective".
        return Vec::new();
    };
    names
        .iter()
        .chain(&def.extra_tools)
        .filter(|name| !universe.contains(name.as_str()))
        .filter(|name| prompt.contains(&format!("`{name}`")))
        .cloned()
        .collect()
}

/// The profile note appended to the guard's failure when any agent it flagged
/// names a tool this build does not contain.
///
/// Deliberately a **decision procedure, not a diagnosis**: it reports the tools
/// it found missing (a fact, derived here) and the command that settles the
/// question (authoritative by construction). It does not try to name which
/// feature gates which tool — that mapping lives in `#[cfg]` attributes across
/// `tools/ops.rs`, and a copy of it here would be a second authority free to
/// rot into a confident wrong answer. Running the command cannot be wrong.
fn profile_note(missing: &BTreeMap<String, Vec<String>>) -> String {
    if missing.is_empty() {
        return String::new();
    }
    let detail = missing
        .iter()
        .map(|(agent, tools)| format!("  {agent}: {}", tools.join(", ")))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\n\nNOTE — this may be a feature-profile artefact, not a prompt defect.\n\
         These agents name tools that do not exist in THIS build's tool universe:\n\
         {detail}\n\
         Tools are registered behind Cargo features, and `default` is a smaller \
         product than CI builds. Settle it by reproducing CI exactly:\n\
         \n    cargo test -p openhuman --lib --features \"$(bash scripts/ci/product-features.sh)\"\n\
         \nIf it passes there, the prompt is fine and this profile simply lacks the \
         tools. If it still fails, the failure is real. See openhuman#6512."
    )
}

#[test]
fn every_prompt_names_at_least_one_tool_it_can_call() {
    let universe = tool_universe();
    let defs = load_builtins().expect("built-ins load");
    let expected: Vec<&str> = NAMES_NO_TOOL
        .iter()
        .copied()
        .chain(SKILL_SETUP_NAME)
        .filter(|id| defs.iter().any(|d| d.id == *id))
        .collect();
    let silent: Vec<String> = defs
        .iter()
        .filter(|def| !matches!(&def.tools, ToolScope::Named(n) if n.is_empty()))
        .filter(|def| {
            let prompt = render(def, &defs);
            !universe
                .iter()
                .any(|name| can_call(def, name, &universe) && prompt.contains(&format!("`{name}`")))
        })
        .map(|def| def.id.clone())
        .collect();
    // Built only for the agents the guard actually flagged, so a passing run
    // does no extra work and the note can never appear on a green result.
    let unexpected: BTreeMap<String, Vec<String>> = silent
        .iter()
        .filter(|id| !expected.contains(&id.as_str()))
        .filter_map(|id| {
            let def = defs.iter().find(|d| &d.id == id)?;
            let absent = belt_tools_absent_from_build(def, &render(def, &defs), &universe);
            (!absent.is_empty()).then(|| (id.clone(), absent))
        })
        .collect();
    assert_eq!(
        silent,
        expected,
        "agents that carry tools but whose prompt names none of them{}",
        profile_note(&unexpected)
    );
}

/// The close-verification rubric is also the tier-2 eval's judge; if its rules
/// are reworded, that judge silently grades against a different standard.
#[test]
fn close_verification_rubric_keeps_its_three_rules() {
    let rubric = crate::agent::session_host::turn_checkpoint::close_verification_prompt("", "", "");
    for stem in [
        "1. The reply only says what the assistant will do",
        "2. The reply states something the tool records contradict",
        "3. The request was not completed, a failed record gives the reason",
    ] {
        assert!(rubric.contains(stem), "rubric lost rule `{stem}`");
    }
}
