//! Tool-pack advertisement: the index the model reads, and the disclosure /
//! execution distinction the whole pack mechanism rests on.

use super::*;
use tinyagents_harness::tool::packs::{route_sentence, USE_SKILL};

// ── the index must agree with the gate too ──────────────────────────────────

/// A spec shaped like the one `UseSkillTool` publishes.
fn use_skill_spec() -> tinytools::ToolSpec {
    let tools = registry_with_all(&["build_workflow"]);
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
        tinyagents_harness::tool::packs::scope_use_skill_spec(&CATALOG, &mut spec, &|name| {
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
    tinyagents_harness::tool::packs::scope_use_skill_spec(&CATALOG, &mut spec, &|name| {
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
        !tinyagents_harness::tool::packs::scope_use_skill_spec(&CATALOG, &mut spec, &|_| false),
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
    tinyagents_harness::tool::packs::scope_use_skill_spec(&CATALOG, &mut spec, &|name| {
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

    let tools = registry_with_all(&["goal_set", "build_workflow"]);
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
    let first = registry_with(name, PermissionLevel::Dangerous);
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
    append_pack_tools(&mut rebuilt);
    let rebuilt = Arc::new(rebuilt);

    // Re-point the ORIGINAL handle at it, which is what a rebuild does.
    crate::tools::host_extensions::pack_registry_handle(use_skill)
        .expect("use_skill exposes a pack registry handle")
        .bind(Arc::downgrade(&rebuilt));

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
    let tools = registry_with_all(&["build_workflow", "propose_workflow"]);
    let handle = crate::tools::host_extensions::pack_registry_handle(find(&tools, USE_SKILL))
        .expect("use_skill carries the pack handle");

    let rendered = tinyagents_harness::tool::packs::render_pack_filtered(
        &CATALOG,
        "workflows",
        handle,
        &|name: &str| name != "propose_workflow",
        "",
    )
    .expect("the pack still has a callable tool");

    assert!(
        rendered.contains("build_workflow"),
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
    let tools = registry_with_all(&["build_workflow", "propose_workflow"]);
    let handle = crate::tools::host_extensions::pack_registry_handle(find(&tools, USE_SKILL))
        .expect("use_skill carries the pack handle");

    let route = route_sentence(&["build_workflow".to_string()], &["workflow_builder"]);
    let err = tinyagents_harness::tool::packs::render_pack_filtered(
        &CATALOG,
        "workflows",
        handle,
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

/// `setup_skills` is a packed hand-off like `build_workflow` and
/// `manage_tasks`: it rides in the `skills` pack it hands off into, so the
/// orchestrator reaches it through `use_skill` instead of paying its schema on
/// every request.
#[test]
fn the_skill_install_hand_off_is_packed_in_the_skills_pack() {
    assert_eq!(
        registry::pack_for_tool("setup_skills").map(|pack| pack.id),
        Some("skills")
    );
}

/// A pack whose owner this agent can hand off to directly is closed to it
/// through `use_skill`; a hand-off inside that pack, a pack the agent owns, and
/// a pack whose hand-off is itself withheld all stay open.
#[test]
fn a_direct_hand_off_closes_its_owners_pack_and_nothing_else() {
    use crate::agent::orchestration::tools::{ArchetypeDelegationTool, DelegationTarget};

    let delegate = |name: &str, target: &str| -> Box<dyn tinytools::Tool> {
        Box::new(ArchetypeDelegationTool {
            tool_name: name.to_string(),
            agent_id: DelegationTarget(target.to_string()),
            tool_description: String::new(),
        })
    };
    // `hand_off_to_skill_setup` is a hypothetical UNPACKED hand-off to the
    // skills pack's owner (the real one, `setup_skills`, is packed now; see
    // `a_packed_hand_off_leaves_its_owners_pack_open`).
    let delegates = [
        delegate("hand_off_to_skill_setup", "skill_setup"),
        delegate("create_image", "image_agent"),
    ];
    let raw = registry_with_all(&[
        "skill_registry_install",
        "media_generate_image",
        "wallet_status",
        "mcp_registry_tool_call",
    ]);
    let tools: Vec<&dyn tinytools::Tool> = raw
        .iter()
        .map(|t| t.as_ref())
        .chain(delegates.iter().map(|t| t.as_ref()))
        .collect();
    // `hand_off_to_skill_setup` is unpacked, so the orchestrator advertises it
    // by construction; `create_image` is packed.
    let closed = closed_by_direct_handoff("orchestrator", &tools);
    assert!(
        closed.contains(&"skill_registry_install"),
        "a raw tool of the pack an unpacked hand-off leads to must close: {closed:?}"
    );
    assert!(
        !closed.contains(&"media_generate_image"),
        "`create_image` is withheld, so the media pack stays open: {closed:?}"
    );
    assert!(
        !closed.contains(&"wallet_status"),
        "no hand-off owns the web3 skill, so it stays open: {closed:?}"
    );
    assert!(
        !closed.contains(&"mcp_registry_tool_call"),
        "no MCP hand-off is on the belt, so integrations stays open: {closed:?}"
    );

    assert!(
        closed_by_direct_handoff("skill_setup", &tools).is_empty(),
        "a pack's owner keeps its own belt"
    );

    // The live regression (#6302): a `ToolScope::Named` agent is built with
    // `visible` = its named list, which never contains a synthesised delegate.
    // Keyed on the visible set, this closed nothing on a real session while
    // every test passed. Pack membership is knowable as soon as the tools are,
    // so the raw tools close with no visible set in the picture at all.
    let named_only: Vec<&dyn tinytools::Tool> = raw.iter().map(|t| t.as_ref()).collect();
    assert!(
        closed_by_direct_handoff("orchestrator", &named_only).is_empty(),
        "with no hand-off among the tools there is nothing to close: the rule \
         must key on a hand-off existing, not on a pack existing"
    );
}

/// The real skill-install hand-off is packed, so it no longer closes the pack:
/// the `skills` listing offers `setup_skills` beside the raw registry tools,
/// the shape every other packed hand-off (`build_workflow`, `manage_tasks`)
/// already has.
#[test]
fn a_packed_hand_off_leaves_its_owners_pack_open() {
    use crate::agent::orchestration::tools::{ArchetypeDelegationTool, DelegationTarget};

    let delegate: Box<dyn tinytools::Tool> = Box::new(ArchetypeDelegationTool {
        tool_name: "setup_skills".to_string(),
        agent_id: DelegationTarget("skill_setup".to_string()),
        tool_description: String::new(),
    });
    let raw = registry_with_all(&["skill_registry_install"]);
    let tools: Vec<&dyn tinytools::Tool> = raw
        .iter()
        .map(|t| t.as_ref())
        .chain(std::iter::once(delegate.as_ref()))
        .collect();
    let closed = closed_by_direct_handoff("orchestrator", &tools);
    assert!(
        !closed.contains(&"skill_registry_install"),
        "a packed hand-off must not close its own pack: {closed:?}"
    );
}

/// The live session shape, which the rule test above cannot catch.
///
/// This is the regression the first cut shipped (#6302): a `ToolScope::Named`
/// agent is built with `visible` = the list its `agent.toml` names, and a
/// *synthesised* delegate like `setup_skills` is by definition not in it — those
/// names arrive later, when `refresh_delegation_tools` inserts them. Keyed on
/// `visible`, the closing rule did nothing on a real chat session: the live
/// orchestrator went on calling `use_skill skills/skill_registry_search` and
/// `skill_registry_install` for real, while every harness test passed, because a
/// harness agent's empty visible set is seeded from every tool, synthesised ones
/// included.
///
/// Asserts `blocks_execution`, not `is_denied`: a merely withheld packed tool is
/// `HideFromPrompt`, for which `is_denied()` answers true, so an `is_denied`
/// assertion would pass without the fix.
#[test]
fn a_named_scope_session_closes_the_pack_its_visible_list_never_mentions() {
    use crate::agent::orchestration::tools::{ArchetypeDelegationTool, DelegationTarget};
    use tinytools::ToolPolicyEngine;

    let delegate: Box<dyn tinytools::Tool> = Box::new(ArchetypeDelegationTool {
        tool_name: "hand_off_to_skill_setup".to_string(),
        agent_id: DelegationTarget("skill_setup".to_string()),
        tool_description: String::new(),
    });
    let raw = registry_with_all(&["skill_registry_install"]);
    let tools: Vec<&dyn tinytools::Tool> = raw
        .iter()
        .map(|t| t.as_ref())
        .chain(std::iter::once(delegate.as_ref()))
        .collect();

    // Exactly what the builder hands a Named agent: its own list. No delegate.
    let visible: HashSet<String> = ["shell", USE_SKILL].iter().map(|s| s.to_string()).collect();
    let build = || {
        ToolPolicyEngine::build_session_from_refs(
            "orchestrator",
            "web_chat",
            "session",
            &Default::default(),
            &tools,
            &visible,
        )
    };

    assert!(
        !build()
            .decision_for("skill_registry_install")
            .blocks_execution(),
        "precondition: withheld-but-callable is the pack default, so this test \
         can only fail for the right reason if closing is what blocks it"
    );

    let mut session = build();
    close_handed_off_packs(&mut session, "orchestrator", &tools);
    assert!(
        session
            .decision_for("skill_registry_install")
            .blocks_execution(),
        "an unpacked hand-off is advertised by construction, so the raw \
         registry tool must close — even though `visible` never named it"
    );
}

/// `file_write` must survive on the orchestrator's belt.
///
/// It is the only create-capable file tool anywhere (`apply_patch` and `edit`
/// both canonicalize an existing target; `shell` means a heredoc). While it was
/// a member of the `files` pack (now `coding`), that pack's owners were one
/// synthesised delegate away on the orchestrator's belt, so
/// `closed_by_direct_handoff` DENIED the pack and `use_skill` answered "has no
/// tools available in this session". The agent's own `agent.toml` names `file_write` and says it
/// "creates new files"; this pins that the pack table no longer contradicts it.
///
/// `file_read` is held to the same rule for a different reason: every
/// `[tool_result_preview]` the harness emits says `read_with: file_read`, so a
/// closed `file_read` turns each oversized result into an `unknown tool` loop.
#[test]
fn the_orchestrators_only_file_creator_is_never_closed() {
    use crate::agent::orchestration::tools::{ArchetypeDelegationTool, DelegationTarget};

    // A hypothetical unpacked hand-off to a `coding` owner closes that pack.
    let delegates: Vec<Box<dyn tinytools::Tool>> = ["review_code"]
        .iter()
        .zip(["critic"])
        .map(|(name, target)| {
            Box::new(ArchetypeDelegationTool {
                tool_name: (*name).to_string(),
                agent_id: DelegationTarget(target.to_string()),
                tool_description: String::new(),
            }) as Box<dyn tinytools::Tool>
        })
        .collect();
    let raw = registry_with_all(&["file_write", "file_read"]);
    let tools: Vec<&dyn tinytools::Tool> = raw
        .iter()
        .map(|t| t.as_ref())
        .chain(delegates.iter().map(|t| t.as_ref()))
        .collect();

    let closed = closed_by_direct_handoff("orchestrator", &tools);
    assert!(
        !closed.contains(&"file_write"),
        "`file_write` must stay reachable to the orchestrator: {closed:?}"
    );
    assert!(
        !closed.contains(&"file_read"),
        "`file_read` must stay reachable: every result preview names it: {closed:?}"
    );
    assert!(
        closed.contains(&"grep"),
        "the rest of the `coding` pack is still the owners' belt: {closed:?}"
    );
    for tool in ["file_write", "file_read"] {
        assert!(
            registry::pack_for_tool(tool).is_none(),
            "`{tool}` must belong to no pack — that is what keeps it open"
        );
    }
}
