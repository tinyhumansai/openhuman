//! Tool-pack advertisement: the index the model reads, and the disclosure /
//! execution distinction the whole pack mechanism rests on.

use super::*;
use tinyagents_harness_tools::skills::{route_sentence, USE_SKILL};

// ── the index must agree with the gate too ──────────────────────────────────

/// A spec shaped like the one `UseSkillTool` publishes.
fn use_skill_spec() -> tinytools::ToolSpec {
    let (tools, _handle) = registry_with_all(&["build_workflow"]);
    let tool = find(&tools, USE_SKILL);
    tinytools::ToolSpec {
        name: tool.name().to_string(),
        description: tool.description().to_string(),
        parameters: tool.parameters_schema(),
    }
}

/// The landing was fixed first; this is the invitation. A pack the session can
/// call nothing in must not be advertised as loadable — the model would go,
/// find out, and come back, which is a wasted round trip on every turn it is
/// tempted.
#[test]
fn the_index_drops_a_pack_this_session_can_call_nothing_in() {
    let mut spec = use_skill_spec();
    assert!(
        spec.description.contains("`system`"),
        "precondition: the unscoped index advertises every pack"
    );

    // Only the workflows pack is reachable.
    let workflows = pack("workflows").expect("workflows pack");
    let kept =
        tinyagents_harness_tools::skills::scope_use_skill_spec(&CATALOG, &mut spec, &|name| {
            workflows.tools.contains(&name)
        });
    assert!(
        kept,
        "workflows is callable, so use_skill stays on the wire"
    );

    assert!(
        spec.description.contains("`workflows`"),
        "a reachable pack must still be offered: {}",
        spec.description
    );
    for dead in ["`system`", "`crypto`", "`audio`", "`documents`"] {
        assert!(
            !spec.description.contains(dead),
            "{dead} has no callable tool here and must not be advertised: {}",
            spec.description
        );
    }
}

/// The schema is the stronger half: an unusable pack becomes unrepresentable,
/// not merely discouraged in prose.
#[test]
fn the_skill_enum_offers_only_reachable_packs() {
    let mut spec = use_skill_spec();
    let workflows = pack("workflows").expect("workflows pack");
    tinyagents_harness_tools::skills::scope_use_skill_spec(&CATALOG, &mut spec, &|name| {
        workflows.tools.contains(&name)
    });

    let values = spec
        .parameters
        .pointer("/properties/skill/enum")
        .and_then(Value::as_array)
        .expect("the skill enum survives scoping")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        vec!["workflows"],
        "the enum must name exactly the reachable packs"
    );
}

/// An empty index and an empty enum are not a tool. The caller is told to drop
/// `use_skill` rather than ship one that can do nothing.
#[test]
fn a_session_that_can_reach_no_pack_loses_use_skill() {
    let mut spec = use_skill_spec();
    assert!(
        !tinyagents_harness_tools::skills::scope_use_skill_spec(&CATALOG, &mut spec, &|_| false),
        "with nothing reachable, use_skill must be dropped, not emptied"
    );
}

/// Scoping must not quietly cost more context than it saves — the index is
/// charged to every turn.
#[test]
fn scoping_the_index_only_ever_shrinks_it() {
    let mut spec = use_skill_spec();
    let before = spec.description.len();
    let workflows = pack("workflows").expect("workflows pack");
    tinyagents_harness_tools::skills::scope_use_skill_spec(&CATALOG, &mut spec, &|name| {
        workflows.tools.contains(&name)
    });
    assert!(
        spec.description.len() < before,
        "scoped index ({}) must be smaller than the full one ({before})",
        spec.description.len()
    );
}

/// **The contract this fix must not break** (AGENTS.md: `Withheld` = "Registered
/// and callable: yes"). A withheld packed tool is hidden from the prompt and
/// still callable through `use_skill` — that is the entire point of a pack.
///
/// `ToolPolicySession` records that state as `HideFromPrompt`, and
/// `ToolPolicyDecision::is_denied()` is `!matches!(action, Allow)` — so it
/// answers **true** for a perfectly callable packed tool. Any "can this session
/// call it" predicate built on `is_denied()` therefore eats the whole pack.
#[test]
fn a_withheld_packed_tool_is_hidden_but_not_denied() {
    use tinytools::{ToolPolicyAction, ToolPolicyEngine};

    let (tools, _handle) = registry_with_all(&["goal_set", "build_workflow"]);
    // The real shape: the harness seeds `visible` with everything, then
    // `strip_packed_from_visible` removes the packed names for a non-owner.
    let mut visible: HashSet<String> = tools.iter().map(|t| t.name().to_string()).collect();
    strip_packed_from_visible(&mut visible, "orchestrator");
    assert!(
        !visible.contains("goal_set"),
        "precondition: the packed tool is withheld from the prompt"
    );

    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web_chat",
        "chat",
        &Default::default(),
        &tools,
        &visible,
    );

    let decision = session.decision_for("goal_set");
    assert_eq!(
        decision.action,
        ToolPolicyAction::HideFromPrompt,
        "a withheld packed tool is classified as prompt-hidden, not denied"
    );
    assert!(
        decision.is_denied(),
        "…and `is_denied()` nevertheless answers true for it — this is the trap"
    );
}

/// **The permission ceiling survives the disclosure exemption.**
///
/// `build_session_from_refs` tests `explicitly_hidden` before
/// `exceeds_permission`, so a tool that is both hidden and over the ceiling is
/// recorded as `HideFromPrompt` and never gets its permission verdict.
/// `is_denied()` masked that by blocking every hidden tool. A predicate that
/// deliberately admits hidden tools must carry the ceiling itself, or `use_skill`
/// becomes a laundering route into a tool the channel would refuse.
#[test]
fn a_hidden_tool_over_the_permission_ceiling_still_blocks_execution() {
    use tinytools::{ToolPolicyAction, ToolPolicyDecision};

    let over = ToolPolicyDecision {
        tool_name: "dangerous_packed_tool".to_string(),
        action: ToolPolicyAction::HideFromPrompt,
        required_permission: Some(PermissionLevel::Dangerous),
        allowed_permission: PermissionLevel::ReadOnly,
    };
    assert!(
        over.blocks_execution(),
        "a hidden tool above the session's ceiling must not be executable"
    );

    let within = ToolPolicyDecision {
        tool_name: "ordinary_packed_tool".to_string(),
        action: ToolPolicyAction::HideFromPrompt,
        required_permission: Some(PermissionLevel::ReadOnly),
        allowed_permission: PermissionLevel::Dangerous,
    };
    assert!(
        !within.blocks_execution(),
        "a hidden tool within the ceiling is the normal packed case and must stay callable"
    );
}

#[test]
fn rebinding_a_pack_handle_repoints_it_at_the_new_registry() {
    // `bind_pack_registry`'s own docs say to "call this after **every**
    // rebinding of the agent's tool `Arc`", but the handle used to hold a
    // `OnceLock`, so the second write was dropped on the floor. An agent that
    // rebuilt its tool vector kept a `Weak` into the old allocation; once that
    // allocation went away the upgrade failed and every `use_skill` call
    // reported the registry as unavailable for the rest of the session.
    // Last write must win.
    let name = pack("web3").unwrap().tools[0];

    // The agent's first tool `Arc`, with the packed tool marked Dangerous.
    let (first, handle) = registry_with(name, PermissionLevel::Dangerous);
    let use_skill = find(&first, USE_SKILL);
    let args = json!({"skill": "web3", "tool": name});
    assert_eq!(
        use_skill.permission_level_with_args(&args),
        PermissionLevel::Dangerous,
        "sanity: the first binding resolves"
    );

    // The agent rebuilds its registry into a *different* allocation, where the
    // same packed tool is only ReadOnly.
    let mut rebuilt: Vec<Box<dyn Tool>> = vec![Box::new(FakeTool {
        name,
        level: PermissionLevel::ReadOnly,
        external: false,
        timeout: ToolTimeout::Inherit,
    })];
    let _rebuilt_handle = append_pack_tools(&mut rebuilt);
    let rebuilt = Arc::new(rebuilt);

    // Re-point the ORIGINAL handle at it, which is what a rebuild does.
    handle.bind(Arc::downgrade(&rebuilt));

    assert_eq!(
        use_skill.permission_level_with_args(&args),
        PermissionLevel::ReadOnly,
        "the rebound handle must resolve against the new registry, not the old one"
    );
}

// ── the listing must agree with the gate ────────────────────────────────────

/// The bug, at the layer it lives on: a non-owner was shown `propose_workflow`
/// and then refused when it called it.
///
/// `is_callable` here stands in for the session allowlist the middleware
/// applies (`channel_permission_block` denies `propose_workflow` for every
/// agent that is not `workflow_builder` / `flow_discovery`). The listing must
/// not mention a tool that gate will refuse.
#[test]
fn a_non_owner_listing_omits_the_tools_the_gate_will_refuse() {
    let (_tools, handle) = registry_with_all(&["run_workflow", "propose_workflow"]);

    let rendered = tinyagents_harness_tools::skills::render_pack_filtered(
        &CATALOG,
        "workflows",
        &handle,
        &|name: &str| name != "propose_workflow",
        "",
    )
    .expect("the pack still has a callable tool");

    assert!(
        rendered.contains("run_workflow"),
        "a tool the session CAN call must still be listed: {rendered}"
    );
    assert!(
        !rendered.contains("propose_workflow"),
        "a tool the session will refuse must not be advertised: {rendered}"
    );
}

/// When the session can reach nothing in the pack, the failure has to carry the
/// way out. A bare "no tools available" is the dead end the model retried into.
#[test]
fn a_listing_with_nothing_callable_names_the_route_out() {
    let (_tools, handle) = registry_with_all(&["build_workflow", "propose_workflow"]);

    let route = route_sentence(&["build_workflow".to_string()], &["workflow_builder"]);
    let err = tinyagents_harness_tools::skills::render_pack_filtered(
        &CATALOG,
        "workflows",
        &handle,
        &|_| false,
        &route,
    )
    .expect_err("nothing callable must not render a menu");

    assert!(
        err.contains("build_workflow"),
        "the denial must name the delegate to call instead: {err}"
    );
}

/// The route only exists because these owners do. If the `workflows` pack is
/// ever re-owned, the hint silently stops naming `workflow_builder` — this
/// pins the assumption the two tests above rest on.
#[test]
fn the_workflows_pack_is_still_owned_by_the_flow_agents() {
    let pack = pack("workflows").expect("the workflows pack exists");
    assert!(
        pack.owners.contains(&"workflow_builder"),
        "owners moved: {:?}",
        pack.owners
    );
    assert!(
        pack.tools.contains(&"propose_workflow"),
        "propose_workflow left the pack: {:?}",
        pack.tools
    );
}

// ── #6302: the MCP and skill hand-offs, and the packs they close ───────────

/// The former specialist aliases are no longer registered or packed; the
/// orchestrator uses the shared spawn tool with an agent id.
#[test]
fn retired_specialist_aliases_are_not_packed() {
    for alias in [
        "setup_skills",
        "manage_tasks",
        "build_workflow",
        "discover_workflows",
        "make_presentation",
        "analyze_image",
        "create_image",
        "create_video",
        "review_code",
        "plan",
    ] {
        assert!(registry::pack_for_tool(alias).is_none(), "{alias}");
        assert!(
            !registry::all_packed_tool_names().contains(&alias),
            "{alias}"
        );
    }
}

/// The spawn tool closes only packs owned by specialists in its allowed-id
/// schema, leaving unrelated packs available to direct calls.
#[test]
fn spawn_tool_schema_closes_only_reachable_specialist_packs() {
    let (raw, _handle) = registry_with_all(&[
        "skill_registry_install",
        "media_generate_image",
        "propose_workflow",
        "generate_presentation",
        "edit",
        "composio_execute",
        "wallet_status",
    ]);
    let spawn = crate::agent::orchestration::tools::SpawnSubagentTool::scoped(vec![
        "skill_setup".to_string(),
        "image_agent".to_string(),
        "video_agent".to_string(),
        "vision_agent".to_string(),
        "workflow_builder".to_string(),
        "flow_discovery".to_string(),
        "presentation_agent".to_string(),
    ]);
    let tools: Vec<&dyn tinytools::Tool> = raw
        .iter()
        .map(|tool| tool.as_ref())
        .chain(std::iter::once(&spawn as &dyn tinytools::Tool))
        .collect();

    let core =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None)
            .derive_with(crate::core::runtime::ContextOverlay::new(
                crate::config::Config::default(),
                crate::core::runtime::DomainSet::full(),
                super::super::ToolGroups::packed(),
            ));
    let closed = crate::core::runtime::CoreContext::sync_scope(core, || {
        closed_by_direct_handoff("orchestrator", &tools)
    });
    assert!(closed.contains(&"skill_registry_install"), "{closed:?}");
    assert!(closed.contains(&"media_generate_image"), "{closed:?}");
    assert!(closed.contains(&"propose_workflow"), "{closed:?}");
    assert!(closed.contains(&"generate_presentation"), "{closed:?}");
    assert!(
        !closed.contains(&"edit"),
        "auxiliary coding-pack owners must not close the Master Agent's coding family: {closed:?}"
    );
    assert!(
        !closed.contains(&"composio_execute"),
        "an incidental workflow_builder owner must not route connected-service actions away from the Master Agent: {closed:?}"
    );
    assert!(!closed.contains(&"wallet_status"), "{closed:?}");
}
