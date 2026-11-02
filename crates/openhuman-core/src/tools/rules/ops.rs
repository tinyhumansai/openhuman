//! Building a session's rule layers and a turn's rule policy.

use std::sync::Arc;

use tinyagents_harness::tool::ToolRulePolicy;
use tinytools::{glob_matches, RuleContext, RuleEffect, ToolRule, ToolRuleSet, ToolRules};

use crate::agent_registry::host_definition::AgentDefinition;
use crate::config::Config;

/// Whether any entry of a definition tool list matches `name`.
///
/// This is the grammar of every name list on an agent definition
/// (`disallowed_tools`) and the registry's denylist: `*` and `?` globs,
/// ASCII case-insensitive, so the historical exact names and trailing-`*`
/// prefixes keep matching what they did. It is the one implementation; the
/// session builder, sub-agent tool preparation, parallel staging and the
/// hosted definition projection all call it.
#[must_use]
pub fn glob_list_matches(patterns: &[String], name: &str) -> bool {
    patterns.iter().any(|pattern| glob_matches(pattern, name))
}

/// Name prefix of every layer an agent definition contributes.
const AGENT_LAYER_PREFIX: &str = "agent:";

/// Name of the operator's `[tool_rules]` layer; an author's own name is kept
/// as a `config:<own>` suffix. Composition sets both prefixes itself, so an
/// operator cannot name a layer into looking like an agent's (or back).
const OPERATOR_LAYER: &str = "config";

/// The rule layer an agent definition contributes: its `tool_rules`, plus a
/// `deny` for its `disallowed_tools`, so a denied tool is refused on every
/// surface — including `tool_search` and a call by a guessed name — not only
/// removed from the visible belt. `None` when the definition restricts
/// nothing.
#[must_use]
pub fn agent_rule_layer(def: &AgentDefinition) -> Option<ToolRules> {
    let mut layer = def.tool_rules.clone().unwrap_or_default();
    if !def.disallowed_tools.is_empty() {
        layer.rules.push(
            ToolRule::names(RuleEffect::Deny, def.disallowed_tools.iter().cloned())
                .with_id("disallowed_tools"),
        );
    }
    if layer.is_permissive() {
        return None;
    }
    // Always `agent:<id>`, keeping an author's own name as a suffix: the
    // prefix is how a sub-agent tells its parent's agent layer from the
    // operator's layers it inherits (see `child_rule_policy`).
    layer.name = Some(match layer.name.take() {
        Some(own) => format!("{AGENT_LAYER_PREFIX}{}/{own}", def.id),
        None => format!("{AGENT_LAYER_PREFIX}{}", def.id),
    });
    Some(layer)
}

/// The rule layers a session carries: the operator's `[tool_rules]`, then
/// the definition's layer. Either may be absent.
#[must_use]
pub fn session_rule_set(config: Option<&Config>, def: Option<&AgentDefinition>) -> ToolRuleSet {
    let mut set = ToolRuleSet::new();
    if let Some(config) = config {
        let mut layer = config.tool_rules.clone();
        layer.name = Some(match layer.name.take() {
            Some(own) => format!("{OPERATOR_LAYER}:{own}"),
            None => OPERATOR_LAYER.to_string(),
        });
        set.push(layer);
    }
    if let Some(layer) = def.and_then(agent_rule_layer) {
        set.push(layer);
    }
    tracing::debug!(
        agent = def.map(|d| d.id.as_str()),
        layers = set.layers.len(),
        "[tool_rules] session rule set composed"
    );
    set
}

/// The context `when` conditions match: `channel`, `agent`, `origin`. Absent
/// or blank values are left out, so a condition on them does not match.
#[must_use]
pub fn rule_context(
    channel: Option<&str>,
    agent: Option<&str>,
    origin: Option<&str>,
) -> RuleContext {
    let mut context = RuleContext::new();
    for (key, value) in [("channel", channel), ("agent", agent), ("origin", origin)] {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            context = context.with(key, value);
        }
    }
    context
}

/// The rule policy for a sub-agent run of `def`.
///
/// The child keeps every operator layer its parent turn carried — the
/// `[tool_rules]` an operator set bind every agent in the tree — and adds its
/// own definition's layer. It does **not** inherit the parent *agent's* layer:
/// an orchestrator commonly disallows exactly the tools it delegates to its
/// workers. A parent without rules (an entry point that built its context
/// from scratch) contributes the operator's `[tool_rules]` directly.
/// The context keeps the parent's `channel` and `origin`; `agent` becomes the
/// child's id. `None` when nothing restricts the child: no rule layer and no
/// inherited permission ceiling.
#[must_use]
pub fn child_rule_policy(
    parent: Option<&ToolRulePolicy>,
    config: Option<&Config>,
    def: &AgentDefinition,
) -> Option<Arc<ToolRulePolicy>> {
    let mut set = match parent {
        Some(parent) => parent.rules.inherit_operator_layers(),
        None => session_rule_set(config, None),
    };
    if let Some(layer) = agent_rule_layer(def) {
        set.push(layer);
    }
    // The parent's permission ceiling restricts the child even when no rule
    // layer does.
    let permission_ceiling = parent.and_then(|parent| parent.permission_ceiling);
    if set.is_permissive() && permission_ceiling.is_none() {
        return None;
    }
    let context = parent
        .map(|parent| parent.context.clone())
        .unwrap_or_default()
        .with("agent", def.id.clone());
    tracing::debug!(
        agent = %def.id,
        layers = set.layers.len(),
        inherited = parent.is_some(),
        "[tool_rules] sub-agent rule policy composed"
    );
    Some(Arc::new(ToolRulePolicy {
        rules: Arc::new(set),
        context,
        permission_ceiling,
    }))
}

/// Installs a turn's rules (`OpenHumanRunContext::tool_rules`) as the harness
/// `RunPolicy::tool_rules`, which applies them to the catalogue, `tool_search`
/// and every call. A permissive or absent policy leaves the default.
pub fn install_turn_rules(
    policy: &mut tinyagents_harness::runtime::RunPolicy,
    rules: Option<Arc<ToolRulePolicy>>,
) {
    let Some(rules) = rules.filter(|rules| !rules.is_permissive()) else {
        return;
    };
    tracing::debug!(
        layers = rules.rules.layers.len(),
        context = ?rules.context,
        "[tool_rules] turn harness carries tool rules"
    );
    policy.tool_rules = (*rules).clone();
}

impl crate::agent::tinyagents::host::OpenHumanRunContext {
    /// This context for a sub-agent run of `def`, carrying
    /// [`child_rule_policy`]'s rules in place of the parent's.
    pub(crate) fn for_subagent(&self, def: &AgentDefinition, config: Option<&Config>) -> Self {
        let mut child = self.clone();
        child.tool_rules = child_rule_policy(self.tool_rules.as_deref(), config, def);
        child.sandbox_mode = Some(def.sandbox_mode);
        child
    }
}

/// The harness policy for one turn: `rules` evaluated in `context`.
#[must_use]
pub fn turn_rule_policy(rules: Arc<ToolRuleSet>, context: RuleContext) -> ToolRulePolicy {
    ToolRulePolicy {
        rules,
        context,
        permission_ceiling: None,
    }
}
