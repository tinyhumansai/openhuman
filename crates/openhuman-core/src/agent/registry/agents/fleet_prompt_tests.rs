//! Invariants every built-in agent's rendered prompt must hold.
//!
//! The per-agent `prompt_tests.rs` files pin what one prompt says. These pin
//! what every prompt must never get wrong, over every definition
//! [`load_builtins`] returns, so an agent added tomorrow is covered without
//! anyone writing a test for it.

use super::load_builtins;
use crate::agent::context::prompt::{LearnedContextData, PromptContext, ToolCallFormat};
use crate::agent::harness::definition::{AgentDefinition, PromptSource, ToolScope};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

/// Render `def`'s prompt through its own `PromptSource`, with no tools,
/// connections or user: what is left is what the agent was written to say.
///
/// Each render gets its own workspace: building a prompt seeds identity files
/// into it, and the tests here render in parallel.
///
/// With no tools, every section gated on them (the `shell` sentence, the
/// `resolve_time` rule) is absent here, and these invariants check static
/// prose only: a prompt must name a tool it can call in its own text, not
/// through a shared section, and a gated section is out of their reach.
fn render(def: &AgentDefinition) -> String {
    let PromptSource::Dynamic(build) = &def.system_prompt else {
        panic!("built-in `{}` must carry a dynamic prompt", def.id);
    };
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let visible = HashSet::new();
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
        personality_soul_md: None,
        personality_memory_md: None,
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
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::config::Config::default()
    };
    let tools = crate::tools::all_tools(
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
        .iter()
        .map(|t| t.name().to_string())
        .chain(
            crate::tools::toolpacks::all_packed_tool_names()
                .into_iter()
                .map(str::to_string),
        )
        .collect()
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
    let packed = crate::tools::toolpacks::all_packed_tool_names();
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
        for name in pack.tools {
            prose = prose.replace(&format!("skill `{}`, tool `{name}`", pack.id), "");
        }
    }
    for name in &packed {
        prose = prose.replace(&format!("skill `{name}`"), "");
    }
    not_callable
        .into_iter()
        .filter(|name| prose.contains(&format!("`{name}`")))
        .collect()
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
const KNOWN_UNCALLABLE: &[(&str, &str, &str)] = &[
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
    ("workflow_builder", "shell", "a flow node kind"),
    ("workflow_builder", "schedule", "a flow trigger field"),
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
        for tool in names_presented_as_callable(&render(def), not_callable) {
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
const NAMES_NO_TOOL: &[&str] = &["critic", "archivist", "skill_setup"];

/// An agent with a belt must be told about at least one tool on it.
///
/// Catches a belt narrowed out from under its prompt, which otherwise reads
/// as an improvement: the tool bytes fall and nothing else moves.
#[test]
fn every_prompt_names_at_least_one_tool_it_can_call() {
    let universe = tool_universe();
    let defs = load_builtins().expect("built-ins load");
    let expected: Vec<&str> = NAMES_NO_TOOL
        .iter()
        .copied()
        .filter(|id| defs.iter().any(|d| d.id == *id))
        .collect();
    let silent: Vec<String> = defs
        .iter()
        .filter(|def| !matches!(&def.tools, ToolScope::Named(n) if n.is_empty()))
        .filter(|def| {
            let prompt = render(def);
            !universe
                .iter()
                .any(|name| can_call(def, name, &universe) && prompt.contains(&format!("`{name}`")))
        })
        .map(|def| def.id.clone())
        .collect();
    assert_eq!(
        silent, expected,
        "agents that carry tools but whose prompt names none of them"
    );
}

/// The close-verification rubric is also the tier-2 eval's judge; if its rules
/// are reworded, that judge silently grades against a different standard.
#[test]
fn close_verification_rubric_keeps_its_three_rules() {
    let rubric =
        crate::agent::harness::session::turn_checkpoint::close_verification_prompt("", "", "");
    for stem in [
        "1. The reply only says what the assistant will do",
        "2. The reply states something the tool records contradict",
        "3. The request was not completed, a failed record gives the reason",
    ] {
        assert!(rubric.contains(stem), "rubric lost rule `{stem}`");
    }
}
