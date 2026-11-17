use super::*;

use std::sync::Arc;

use crate::agent_registry::host_definition::AgentDefinition;
use crate::config::Config;
use tinytools::ToolRuleSet;

use tinytools::{PermissionLevel, Surface, ToolSubject};

fn definition(id: &str) -> AgentDefinition {
    crate::agent_registry::agents::load_builtins()
        .expect("built-ins load")
        .into_iter()
        .next()
        .map(|mut def| {
            def.id = id.to_string();
            def.disallowed_tools.clear();
            def.tool_rules = None;
            def
        })
        .expect("a built-in definition")
}

fn visible(set: &ToolRuleSet, name: &str, context: &tinytools::RuleContext) -> bool {
    set.visible(&ToolSubject::named(name), context, Surface::Catalog)
}

#[test]
fn glob_list_keeps_the_legacy_exact_and_prefix_forms() {
    let patterns = vec!["shell".to_string(), "spawn_*".to_string()];
    assert!(glob_list_matches(&patterns, "shell"));
    assert!(glob_list_matches(&patterns, "spawn_subagent"));
    assert!(!glob_list_matches(&patterns, "shell_exec"));
    assert!(!glob_list_matches(&patterns, "file_read"));
    assert!(!glob_list_matches(&[], "shell"));
}

#[test]
fn glob_list_now_also_takes_inner_wildcards_and_any_case() {
    let patterns = vec!["mcp_*_delete_*".to_string(), "gmail_*".to_string()];
    assert!(glob_list_matches(
        &patterns,
        "mcp_github_delete_repo_abc123"
    ));
    assert!(glob_list_matches(&patterns, "GMAIL_SEND_EMAIL"));
}

#[test]
fn an_unrestricted_definition_contributes_no_layer() {
    assert!(agent_rule_layer(&definition("plain")).is_none());
}

#[test]
fn disallowed_tools_become_a_deny_rule_on_every_surface() {
    let mut def = definition("researcher");
    def.disallowed_tools = vec!["shell".into(), "web_*".into()];
    let layer = agent_rule_layer(&def).expect("a layer");
    assert_eq!(layer.name.as_deref(), Some("agent:researcher"));
    let set = ToolRuleSet::single(layer);
    let context = rule_context(None, None, None);
    assert!(!visible(&set, "shell", &context));
    assert!(!set.visible(&ToolSubject::named("web_fetch"), &context, Surface::Search));
    let call = set.evaluate(
        &ToolSubject::named("web_fetch"),
        &context,
        Surface::Call,
        None,
    );
    assert!(!call.callable);
    assert!(call.refusal("web_fetch").contains("disallowed_tools"));
    assert!(visible(&set, "file_read", &context));
}

#[test]
fn a_definition_rule_layer_keeps_its_own_name_and_adds_the_denylist() {
    let mut def = definition("writer");
    def.tool_rules = Some(
        tinytools::ToolRules::from_allow_deny(["file_*"], Vec::<String>::new()).named("custom"),
    );
    def.disallowed_tools = vec!["file_delete".into()];
    let layer = agent_rule_layer(&def).expect("a layer");
    assert_eq!(layer.name.as_deref(), Some("agent:writer/custom"));
    let set = ToolRuleSet::single(layer);
    let context = rule_context(None, None, None);
    assert!(visible(&set, "file_read", &context));
    assert!(!visible(&set, "file_delete", &context));
    assert!(!visible(&set, "shell", &context));
}

#[test]
fn the_session_set_stacks_config_then_agent() {
    let mut config = Config::default();
    config.tool_rules = tinytools::ToolRules::from_allow_deny(Vec::<String>::new(), ["shell"]);
    let mut def = definition("helper");
    def.disallowed_tools = vec!["web_*".into()];
    let set = session_rule_set(Some(&config), Some(&def));
    assert_eq!(set.layers.len(), 2);
    assert_eq!(set.layers[0].name.as_deref(), Some("config"));
    assert_eq!(set.layers[1].name.as_deref(), Some("agent:helper"));
    let context = rule_context(None, None, None);
    assert!(!visible(&set, "shell", &context));
    assert!(!visible(&set, "web_fetch", &context));
    assert!(visible(&set, "file_read", &context));
}

#[test]
fn an_empty_config_and_definition_yield_a_permissive_set() {
    let set = session_rule_set(Some(&Config::default()), Some(&definition("x")));
    assert!(set.is_permissive());
    assert!(session_rule_set(None, None).is_permissive());
}

#[test]
fn the_rule_context_drops_blank_values() {
    let context = rule_context(Some("telegram"), Some(" "), None);
    assert_eq!(context.get("channel"), Some("telegram"));
    assert_eq!(context.get("agent"), None);
    assert_eq!(context.get("origin"), None);
}

#[test]
fn turn_policy_evaluates_when_conditions_in_its_context() {
    let mut config = Config::default();
    config.tool_rules = serde_json::from_value(serde_json::json!({ "rules": [
        { "effect": "deny", "match": { "name": "shell" }, "when": { "channel": "telegram" } },
    ] }))
    .expect("rules");
    let set = Arc::new(session_rule_set(Some(&config), None));
    let telegram = turn_rule_policy(set.clone(), rule_context(Some("telegram"), None, None));
    let web = turn_rule_policy(set, rule_context(Some("web"), None, None));
    assert!(!visible(&telegram.rules, "shell", &telegram.context));
    assert!(visible(&web.rules, "shell", &web.context));
}

#[test]
fn config_parses_a_tool_rules_table_from_toml() {
    let config: Config = toml::from_str(
        r#"
[tool_rules]
default = "allow"

[[tool_rules.rules]]
id = "no-mcp"
effect = "deny"
match = { name = "mcp_*" }
except = { family = "github" }
"#,
    )
    .expect("config parses");
    assert_eq!(config.tool_rules.rules.len(), 1);
    assert_eq!(config.tool_rules.rules[0].id.as_deref(), Some("no-mcp"));
}

#[test]
fn a_child_inherits_operator_layers_but_not_its_parents_agent_layer() {
    let mut config = Config::default();
    config.tool_rules = tinytools::ToolRules::from_allow_deny(Vec::<String>::new(), ["shell"]);
    let mut orchestrator = definition("orchestrator");
    orchestrator.disallowed_tools = vec!["file_*".into()];
    let parent = turn_rule_policy(
        Arc::new(session_rule_set(Some(&config), Some(&orchestrator))),
        rule_context(
            Some("telegram"),
            Some("orchestrator"),
            Some("ExternalChannel(telegram)"),
        ),
    )
    .with_permission_ceiling(PermissionLevel::ReadOnly);
    let mut worker = definition("code_executor");
    worker.disallowed_tools = vec!["web_*".into()];

    let child = child_rule_policy(Some(&parent), None, &worker).expect("child rules");
    assert_eq!(child.permission_ceiling, Some(PermissionLevel::ReadOnly));
    assert_eq!(child.context.get("agent"), Some("code_executor"));
    assert_eq!(child.context.get("channel"), Some("telegram"));
    assert!(
        !visible(&child.rules, "shell", &child.context),
        "operator layer inherited"
    );
    assert!(
        visible(&child.rules, "file_write", &child.context),
        "parent agent layer dropped"
    );
    assert!(
        !visible(&child.rules, "web_fetch", &child.context),
        "own layer added"
    );
}

#[test]
fn a_child_without_parent_rules_takes_the_operator_layer() {
    let mut config = Config::default();
    config.tool_rules = tinytools::ToolRules::from_allow_deny(Vec::<String>::new(), ["shell"]);
    let child = child_rule_policy(None, Some(&config), &definition("worker")).expect("rules");
    assert!(!visible(&child.rules, "shell", &child.context));
    assert!(child_rule_policy(None, None, &definition("worker")).is_none());
}

#[test]
fn an_operator_layer_cannot_be_named_into_an_agent_layer() {
    let mut config = Config::default();
    config.tool_rules = tinytools::ToolRules::from_allow_deny(Vec::<String>::new(), ["shell"])
        .named("agent:baseline");
    let set = session_rule_set(Some(&config), None);
    assert_eq!(set.layers[0].name.as_deref(), Some("config:agent:baseline"));
    let parent = turn_rule_policy(
        Arc::new(set),
        rule_context(None, Some("orchestrator"), None),
    );

    let child = child_rule_policy(Some(&parent), None, &definition("worker")).expect("rules");
    assert!(
        !visible(&child.rules, "shell", &child.context),
        "the operator layer is still inherited whatever its author named it"
    );
}

/// A parent whose rules are permissive can still carry a permission ceiling
/// (the pack ceiling of a `use_skill` run). The child must keep it: dropping
/// the policy because the rules alone are permissive lifted the ceiling.
#[test]
fn a_child_keeps_the_parents_ceiling_when_no_rules_restrict_it() {
    let parent = turn_rule_policy(
        Arc::new(session_rule_set(None, None)),
        rule_context(None, Some("orchestrator"), None),
    )
    .with_permission_ceiling(PermissionLevel::ReadOnly);
    assert!(
        parent.rules.is_permissive(),
        "precondition: no rule restricts"
    );

    let child = child_rule_policy(Some(&parent), None, &definition("worker"))
        .expect("the parent's ceiling still restricts the child");
    assert_eq!(child.permission_ceiling, Some(PermissionLevel::ReadOnly));
    assert_eq!(child.context.get("agent"), Some("worker"));
}
