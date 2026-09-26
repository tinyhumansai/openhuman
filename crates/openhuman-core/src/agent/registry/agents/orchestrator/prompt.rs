//! System prompt builder for the `orchestrator` built-in agent.
//!
//! The orchestrator follows a direct-first policy: respond directly or use
//! cheap direct tools whenever possible, and delegate only for specialised
//! execution. Connected Composio integrations are part of that direct
//! surface: every connected toolkit's actions are registered as `Deferred`
//! tools (`orchestrator_tools::collect_deferred_integration_actions`), so
//! the `## Connected Integrations` block tells the model to `tool_search`
//! for the action and call it — there is no integrations sub-agent to
//! delegate to any more. That prose lives here (not in the shared prompts
//! module) so nobody has to branch on `agent_id` in a shared section impl.

use crate::agent::harness::definition::SubagentEntry;
use crate::agent::harness::AgentDefinitionRegistry;
use crate::agent::prompts::{
    render_datetime, render_identity, render_tools, render_user_files, render_workspace,
    ConnectedIntegration, PromptContext,
};
use crate::skills::ops_types::Workflow;
use crate::tools::orchestrator_tools::sanitise_slug;
use crate::tools::toolpacks;
use anyhow::Result;
use std::fmt::Write;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    use crate::agent::prompts::{PROMPT_TIER_CONTEXT_MARKER, PROMPT_TIER_VOLATILE_MARKER};

    let mut out = String::with_capacity(8192);
    let push = |out: &mut String, part: &str| {
        if !part.trim().is_empty() {
            out.push_str(part.trim_end());
            out.push_str("\n\n");
        }
    };

    // Resolved once: the skill routes decide both the static rows below
    // and the generated sections further down, and they must agree (#6302).
    let skill_run = hand_off_route(ctx, "skill_executor");
    let skill_install = hand_off_route(ctx, "skill_setup");
    // An empty visibility set is the builder's unfiltered sentinel. Preserve
    // the MCP route for those sessions while suppressing it in gated-off builds.
    let mcp_available = cfg!(feature = "mcp")
        && (ctx.visible_tool_names.is_empty()
            || ctx.visible_tool_names.contains("mcp_registry_tool_call"));

    // ── Stable tier: identical across sessions for a given build ─────────
    //
    // Identity leads the prompt (#5701): SOUL.md is the product persona every
    // opted-in agent shares, ROLE.md is this agent's own role brief. Rendered
    // here rather than via `IdentitySection` because the orchestrator is a
    // `PromptSource::Dynamic` agent: `SystemPromptBuilder::from_dynamic`
    // installs only this builder, so the section chain never runs for us.
    push(&mut out, &render_identity(ctx)?);
    push(
        &mut out,
        &strip_route_lines(
            ARCHETYPE,
            skill_run.is_some() || skill_install.is_some(),
            mcp_available,
        ),
    );
    push(&mut out, &render_tools(ctx)?);
    push(&mut out, &render_datetime(ctx)?);

    // ── Context tier: stable for the session, not across installs ────────
    out.push_str(PROMPT_TIER_CONTEXT_MARKER);
    out.push('\n');
    push(&mut out, &render_workspace(ctx)?);
    // Model families that stop after announcing a plan get one short block of
    // execution discipline; the rest (Claude, Gemini) pay nothing. The text
    // and the gate are tinyagents', so every host renders the same words.
    if let Some(guidance) = tinyagents_harness::prompt::execution_discipline_for(ctx.model_name) {
        tracing::debug!(
            model = ctx.model_name,
            "[orchestrator-prompt] rendering model-gated execution discipline"
        );
        push(&mut out, guidance);
    }

    // ── Volatile tier: the user's state, changes between sessions ────────
    out.push_str(PROMPT_TIER_VOLATILE_MARKER);
    out.push('\n');
    push(&mut out, &render_user_files(ctx)?);
    push(&mut out, ctx.connected_identities_md.as_str());
    push(
        &mut out,
        &render_installed_skills(
            ctx.workflows,
            skill_run.as_deref(),
            skill_install.as_deref(),
        ),
    );
    push(&mut out, &render_withheld_specialists(ctx));
    push(
        &mut out,
        &render_connected_integrations(ctx.connected_integrations),
    );
    if mcp_available {
        push(&mut out, &render_connected_mcp_servers());
    }

    // NOTE: the grounding contract lives in `prompt.md` under the shared
    // heading, so `SystemPromptBuilder::build` skips the global copy.
    Ok(out)
}

/// Render `## Capabilities not in your tool list` — the specialists whose
/// delegate tool a tool pack is currently withholding.
///
/// This block is **generated, not written**, and that is the whole point. The
/// routing table it replaces was prose in `prompt.md` naming fifteen tools,
/// none of it conditioned on the live tool set, and ten of those names were
/// tools a pack had withheld: the prompt taught the model to call something it
/// could not see, and nothing in the build compared the two. Deriving the rows
/// from the same registry `collect_orchestrator_tools` synthesises the
/// delegates from means a pack change moves both halves at once.
///
/// **Advertised specialists are deliberately absent.** Their `when_to_use` is
/// already their tool description on the wire, and restating it here would be
/// the duplication `orchestrator/agent.toml` warns about, charged twice per
/// turn. Only a withheld specialist needs prose, because its description is
/// the thing the model cannot see.
fn render_withheld_specialists(ctx: &PromptContext<'_>) -> String {
    // Empty is the harness's "everything is visible" sentinel, not "nothing
    // visible" — with no filter, nothing is withheld and the section is void.
    if ctx.visible_tool_names.is_empty() {
        tracing::debug!(
            agent = ctx.agent_id,
            "[orchestrator-prompt] no visible-tool filter; nothing can be withheld"
        );
        return String::new();
    }
    let Some(registry) = AgentDefinitionRegistry::global() else {
        tracing::debug!(
            "[orchestrator-prompt] no agent registry; withheld-specialist section omitted"
        );
        return String::new();
    };
    let Some(definition) = resolve_definition(registry, ctx.agent_id) else {
        tracing::debug!(
            agent = ctx.agent_id,
            "[orchestrator-prompt] agent id does not resolve to a registry entry"
        );
        return String::new();
    };

    let mut rows: Vec<(String, &'static str)> = Vec::new();
    for entry in &definition.subagents {
        // `Skills(_)` expands to searchable integration actions, not a
        // delegate tool; the `## Connected Integrations` block covers them.
        let SubagentEntry::AgentId(agent_id) = entry else {
            continue;
        };
        // Runtime-only, never given a delegate tool — see the same skip in
        // `collect_orchestrator_tools`.
        if agent_id == "summarizer" {
            continue;
        }
        let Some(target) = registry.get(agent_id) else {
            continue;
        };
        let tool_name = target
            .delegate_name
            .clone()
            .unwrap_or_else(|| format!("delegate_{}", target.id));
        if ctx.visible_tool_names.contains(&tool_name) {
            continue;
        }
        let Some(pack) = toolpacks::pack_for_tool(&tool_name) else {
            // Not advertised and not packed: the agent is compiled out or the
            // belt never listed it, so there is no route to describe.
            continue;
        };
        rows.push((tool_name, pack.id));
    }

    if rows.is_empty() {
        tracing::debug!(
            agent = ctx.agent_id,
            subagents = definition.subagents.len(),
            visible = ctx.visible_tool_names.len(),
            "[orchestrator-prompt] no withheld specialists to render"
        );
        return String::new();
    }
    tracing::debug!(
        count = rows.len(),
        "[orchestrator-prompt] rendering withheld-specialist routing"
    );

    // One line per pack, tools named without their blurbs: `use_skill`'s own
    // description already carries a one-line summary of every pack, and the
    // full `when_to_use` arrives with the schema once the pack is loaded.
    let mut by_pack: std::collections::BTreeMap<&'static str, Vec<String>> =
        std::collections::BTreeMap::new();
    for (tool, pack) in rows {
        by_pack.entry(pack).or_default().push(format!("`{tool}`"));
    }
    let mut out = String::from(
        "## Capabilities not in your tool list\n\nAvailable through `use_skill` (`skill` \
         alone lists arguments; add `tool` + `args` to run):\n\n",
    );
    for (pack, tools) in by_pack {
        let _ = writeln!(out, "- skill `{pack}`: {}", tools.join(", "));
    }
    out
}

/// How this session can reach `specialist` right now, as the call to name.
///
/// The hand-off tool in backticks when it is on the belt; the `use_skill` form
/// when a pack holds it; `None` when this agent has no route to the specialist,
/// in which case the prompt must name none. Derived from the registry and the
/// visible set exactly like [`render_withheld_specialists`], never hand-written,
/// so a pack or allowlist change moves the prose with it (#6302).
fn hand_off_route(ctx: &PromptContext<'_>, specialist: &str) -> Option<String> {
    let registry = AgentDefinitionRegistry::global()?;
    let definition = resolve_definition(registry, ctx.agent_id)?;
    let listed = definition
        .subagents
        .iter()
        .any(|entry| matches!(entry, SubagentEntry::AgentId(id) if id == specialist));
    if !listed {
        return None;
    }
    let target = registry.get(specialist)?;
    let tool = target
        .delegate_name
        .clone()
        .unwrap_or_else(|| format!("delegate_{}", target.id));
    if ctx.visible_tool_names.is_empty() || ctx.visible_tool_names.contains(&tool) {
        return Some(format!("`{tool}`"));
    }
    // A packed route is only a route if this session can call `use_skill`
    // itself. A filtered belt holding neither the delegate nor `use_skill` has
    // no way to reach the specialist, and naming one anyway is the same "call a
    // tool you do not have" failure this whole block exists to end (#6302).
    if !ctx.visible_tool_names.contains(toolpacks::USE_SKILL) {
        return None;
    }
    toolpacks::pack_for_tool(&tool).map(|pack| {
        format!(
            "`use_skill {{ \"skill\": \"{}\", \"tool\": \"{tool}\" }}`",
            pack.id
        )
    })
}

/// `prompt.md` with the route-tagged rows this build cannot honour removed.
///
/// A row tagged `<!--route:skills-->` or `<!--route:mcp-->` names a hand-off
/// that exists only while that family is compiled in: with `skills` off the
/// loader drops `skill_setup` and `skill_executor` from the builtins, so no
/// delegate is synthesised and the static row would order the model to call a
/// tool nobody has — the very failure this issue is about (#6302). The tag is
/// stripped from every row that stays, so it never reaches the model.
fn strip_route_lines(archetype: &str, skills: bool, mcp: bool) -> String {
    const SKILLS_TAG: &str = "<!--route:skills-->";
    const MCP_TAG: &str = "<!--route:mcp-->";
    archetype
        .lines()
        .filter(|line| {
            if line.contains(SKILLS_TAG) {
                skills
            } else if line.contains(MCP_TAG) {
                mcp
            } else {
                true
            }
        })
        .map(|line| line.replace(SKILLS_TAG, "").replace(MCP_TAG, ""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The registry entry behind `agent_id`, tolerating the web channel's rename.
///
/// `PromptContext::agent_id` carries `OpenHumanSessionHost::agent_definition_name`, which the
/// web channel rewrites to `"orchestrator_<short_thread>"` so each thread gets
/// its own transcript namespace. The canonical id lives in a different field
/// (`agent_definition_id`, whose docs say to use it for exactly this), but that
/// one is not on `PromptContext` and adding it would mean editing all 62
/// construction sites of a struct with no `Default`.
///
/// So: exact match first, then the longest registry id that `agent_id` extends
/// at an `_` boundary. Longest wins because ids are not prefix-free —
/// `integrations_agent` starts with no other id today, but `skill_setup` and
/// `skill_executor` share a stem, and a shorter accidental match would resolve
/// a renamed session onto the wrong agent's subagent list.
fn resolve_definition<'r>(
    registry: &'r AgentDefinitionRegistry,
    agent_id: &str,
) -> Option<&'r crate::agent::harness::definition::AgentDefinition> {
    if let Some(found) = registry.get(agent_id) {
        return Some(found);
    }
    let best = registry
        .list()
        .iter()
        .filter(|d| {
            agent_id
                .strip_prefix(d.id.as_str())
                .is_some_and(|rest| rest.starts_with('_'))
        })
        .max_by_key(|d| d.id.len())?
        .id
        .clone();
    registry.get(&best)
}

fn render_installed_skills(
    skills: &[Workflow],
    run: Option<&str>,
    install: Option<&str>,
) -> String {
    if skills.is_empty() {
        tracing::debug!("[orchestrator-prompt] no installed skills, section omitted");
        return String::new();
    }
    tracing::debug!(
        count = skills.len(),
        run_route = run.is_some(),
        install_route = install.is_some(),
        "[orchestrator-prompt] rendering installed skills section"
    );
    let mut out = String::from("## Installed Skills\n\n");
    if let Some(run) = run {
        let _ = write!(out, "Run one with {run} (skill id + task). ");
    }
    if let Some(install) = install {
        let _ = write!(out, "Find or install others with {install}. ");
    }
    out.push_str(
        "A skill runs in an isolated worker and returns its result plus a `## Handoff Plan` \
         for anything it could not do itself.\n\n",
    );
    for skill in skills {
        let id = if skill.dir_name.is_empty() {
            &skill.name
        } else {
            &skill.dir_name
        };
        let desc = if skill.description.is_empty() {
            "(no description)".to_string()
        } else {
            // Skill descriptions are third-party metadata injected verbatim
            // into the system prompt on EVERY turn. Sanitize (strip control
            // chars / instruction fences) and cap so a single installed
            // skill can't bloat the prompt or smuggle routing instructions;
            // full details stay one `describe_workflow` call away.
            crate::util::sanitize::sanitize_for_llm(&skill.description, 120)
                .replace(['\n', '\t'], " ")
                .trim()
                .to_string()
        };
        let _ = writeln!(out, "- **{id}**: {desc}");
    }
    out
}

/// Render the `## Connected MCP Servers` block from the live connection
/// registry. The MCP analogue of [`render_connected_integrations`]: it lists each
/// connected MCP server and tells the orchestrator to discover and call its
/// tools directly. This is what lets the orchestrator pick up a connected server
/// *without the user naming it* (e.g. a connected "weather" server answering
/// "what's the weather in Tokyo?").
///
/// Reads the global connection map via a guarded `block_on` — the same
/// pattern `tool_registry::ops::registry_entries` uses. `block_in_place`
/// requires the multi-threaded runtime; single-threaded contexts (unit
/// tests) fall back to an empty list and the section is omitted.
fn render_connected_mcp_servers() -> String {
    use crate::mcp::registry::connections;
    let servers = match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(connections::connected_overview()))
        }
        _ => Vec::new(),
    };
    format_connected_mcp_block(&servers)
}

/// Pure formatter for the connected-MCP block — split from
/// [`render_connected_mcp_servers`] so it is unit-testable without a live
/// connection registry. Empty input → empty string (section omitted).
fn format_connected_mcp_block(
    servers: &[crate::mcp::registry::connections::ConnectedServerOverview],
) -> String {
    if servers.is_empty() {
        return String::new();
    }
    // Keep the block compact — describe each server (the capability signal),
    // not its full toolset. Mirrors the Composio `## Connected Integrations`
    // block (`**Toolkit** (slug): description`). The orchestrator discovers
    // each server's actual tools on demand via `mcp_registry_list_tools`.
    let mut out = String::from(
        "## Connected MCP Servers\n\n\
         Their actions are searchable through `tool_search`. Search for the \
         action in plain words, then call the matching MCP tool with the \
         schema the search returns.\n\n",
    );
    for s in servers {
        let name = if s.display_name.trim().is_empty() {
            s.qualified_name.as_str()
        } else {
            s.display_name.as_str()
        };
        // The registry/install description and instructions are UNTRUSTED
        // free-form metadata.
        // It is interpolated into the orchestrator system prompt verbatim, so
        // run it through the same strip-control + strip-instruction-fence +
        // byte-bound pipeline used for remote tool metadata before trusting it
        // (malicious metadata could otherwise smuggle routing-overriding
        // instructions into the prompt). Flatten newlines/tabs so a single
        // list item can't be broken or hijacked across lines.
        let capability_raw = s
            .description
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                s.instructions
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or("")
            .trim();
        let capability = if capability_raw.is_empty() {
            String::new()
        } else {
            crate::util::sanitize::sanitize_for_llm(capability_raw, 120)
                .replace(['\n', '\t'], " ")
                .trim()
                .to_string()
        };
        if !capability.is_empty() {
            let _ = writeln!(
                out,
                "- **{name}** (`{}`, `server_id: \"{}\"`): {capability}",
                s.qualified_name, s.server_id
            );
        } else {
            // No registry capability metadata — fall back to a tool-count
            // hint so the line still conveys the server has callable capability.
            let _ = writeln!(
                out,
                "- **{name}** (`{}`, `server_id: \"{}\"`) — {} tool{} available",
                s.qualified_name,
                s.server_id,
                s.tools.len(),
                if s.tools.len() == 1 { "" } else { "s" }
            );
        }
    }
    out
}

/// Render the `## Connected Integrations` block. Only toolkits the user has
/// actively connected are listed — unauthorised toolkits are hidden so the
/// orchestrator cannot claim access to a service it does not have. When
/// every toolkit is unconnected the whole section is omitted.
///
/// The connected toolkits' actions are `Deferred` tools on this agent's own
/// belt (`collect_deferred_integration_actions`), so the block teaches one
/// route: `tool_search` for the action, then call it. The old collapsed
/// `delegate_to_integrations_agent` spawn is gone; an integration action is
/// a search and a call, not a sub-agent run.
///
/// The slug printed beside each toolkit uses the same `sanitise_slug` as the
/// rest of the prompt surface so the model's `composio_connect` argument and
/// its searches name the toolkit consistently.
///
fn render_connected_integrations(integrations: &[ConnectedIntegration]) -> String {
    let connected: Vec<&ConnectedIntegration> =
        integrations.iter().filter(|ci| ci.connected).collect();
    tracing::debug!(
        total_integrations = integrations.len(),
        connected_count = connected.len(),
        "[connected-integrations] rendering integration section ({} connected / {} total)",
        connected.len(),
        integrations.len()
    );
    if connected.is_empty() {
        tracing::debug!("[connected-integrations] section omitted — no connected integrations");
        return String::new();
    }
    let mut out = String::from(
        "## Connected Integrations\n\n\
         Their actions are not in your listed tools: `tool_search` for the action in plain \
         words (\"send an email\", \"list calendar events\"), then call the tool it returns — \
         no sub-agent. Act on a service only when the request operates on that service's data \
         or actions (a connected service is not a reason to touch it for general-knowledge, \
         web/news, date/time or math questions). Never claim you cannot access one without \
         searching first.\n\n",
    );
    for ci in &connected {
        let slug = sanitise_slug(&ci.toolkit);
        if ci.connections.len() > 1 {
            let _ = writeln!(
                out,
                "- **{}** (`toolkit: \"{}\"`, {} accounts connected): {}",
                ci.toolkit,
                slug,
                ci.connections.len(),
                ci.description
            );
            for conn in &ci.connections {
                let label = conn.label.as_deref().unwrap_or("(unlabeled)");
                let default_marker = if conn.is_default { " [default]" } else { "" };
                let _ = writeln!(
                    out,
                    "  - `connection_id: \"{}\"` — {}{}",
                    conn.connection_id, label, default_marker
                );
            }
        } else {
            let _ = writeln!(
                out,
                "- **{}** (`toolkit: \"{}\"`): {}",
                ci.toolkit, slug, ci.description
            );
        }
    }
    // CRITICAL behavioural rule. Without this, the orchestrator answers
    // "can you do X with {toolkit}?" from its training-data priors about
    // "what gmail/notion/slack usually does", which is consistently a
    // SUBSET of the real per-toolkit catalogue (no bulk-delete, no
    // batch-modify, no admin/destructive actions, etc.). The result is a
    // confident wrong refusal ("nope, I can't delete emails") even when
    // the action is in the catalogue. `tool_search` is the ground truth for
    // callable actions.
    // The cross-chat bullet names the canonical header literal verbatim
    // so the model knows exactly which block to mistrust. Sourced from
    // CROSS_CHAT_HEADER (single source of truth) — drift would silently
    // detune the rule.
    let cross_chat_header_for_prompt =
        crate::memory::agent::memory_loader::CROSS_CHAT_HEADER.trim_end();
    let _ = write!(
        out,
        "\n### Capability questions about connected toolkits\n\n\
         Your prior knowledge of what a toolkit can do is unreliable: the live catalogue and \
         the user's scopes decide. For \"can you do X with {{toolkit}}?\" or any action on a \
         connected toolkit, `tool_search` first; an empty search means the action \
         is not currently available. A past \"I can / can't\" in the \
         `{cross_chat_header_for_prompt}` block is a stale snapshot, never an answer.\n\n",
    );

    tracing::debug!(
        section_len = out.len(),
        "[connected-integrations] section emitted ({} bytes)",
        out.len()
    );
    out
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
