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

// ── Enforcement: the rule layer is the only `disallowed_tools` gate ─────────
//
// The session builder, sub-agent tool preparation and the hosted definition
// projection no longer drop a definition's `disallowed_tools` from their
// belts. These drive a real harness turn with no allowlist at all, so the
// only thing between the model and a denied tool is the rule layer composed
// here — exactly as a session or sub-agent turn installs it.

use std::sync::atomic::{AtomicUsize, Ordering};

use tinyagents_harness::runtime::{AgentHarness, RunPolicy};
use tinyagents_harness::testkit::ScriptedModel;
use tinyagents_harness::tool::ToolRulePolicy;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::ModelResponse;
use tinyinference_llm::tool::ToolCall;
use tinytools::{Tool, ToolExposure, ToolResult};

struct CountingTool {
    name: &'static str,
    exposure: ToolExposure,
    calls: AtomicUsize,
}

impl CountingTool {
    fn direct(name: &'static str) -> Arc<Self> {
        Self::new(name, ToolExposure::Direct)
    }

    fn deferred(name: &'static str) -> Arc<Self> {
        Self::new(name, ToolExposure::Deferred)
    }

    fn new(name: &'static str, exposure: ToolExposure) -> Arc<Self> {
        Arc::new(Self {
            name,
            exposure,
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "a lookup tool for the disallowed_tools enforcement tests"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    fn exposure(&self) -> ToolExposure {
        self.exposure
    }
    async fn execute(&self, _arguments: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success(format!("{} ran", self.name)))
    }
}

fn calling(calls: &[(&str, &str, serde_json::Value)]) -> ModelResponse {
    let mut response = ModelResponse::assistant("");
    for (id, name, args) in calls {
        response
            .message
            .tool_calls
            .push(ToolCall::new(*id, *name, args.clone()));
    }
    response
}

/// Runs `script` on a harness carrying `rules` the way a turn installs them
/// (`install_turn_rules`), with `tools` registered and no allowlist.
async fn run_under(
    rules: Arc<ToolRulePolicy>,
    script: Vec<ModelResponse>,
    tools: &[Arc<CountingTool>],
) -> (Arc<ScriptedModel>, Vec<Message>) {
    let model = Arc::new(ScriptedModel::new(script));
    let mut harness: AgentHarness<()> = AgentHarness::new();
    harness.with_model(model.clone());
    let mut policy = RunPolicy::default();
    install_turn_rules(&mut policy, Some(rules));
    harness.with_policy(policy);
    for tool in tools {
        harness.register_tool(tool.clone());
    }
    let run = harness
        .invoke_default(&(), vec![Message::user("go")])
        .await
        .expect("run completes");
    (model, run.messages)
}

fn tool_text(messages: &[Message], call_id: &str) -> String {
    messages
        .iter()
        .find_map(|message| match message {
            Message::Tool(tool) if tool.tool_call_id == call_id => Some(message.text()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no result for {call_id}"))
}

/// A definition that disallows an exact name and a prefix glob.
fn restricted() -> AgentDefinition {
    let mut def = definition("researcher");
    def.disallowed_tools = vec!["shell".into(), "vendor_*".into()];
    def
}

fn session_rules(def: &AgentDefinition) -> Arc<ToolRulePolicy> {
    Arc::new(turn_rule_policy(
        Arc::new(session_rule_set(None, Some(def))),
        rule_context(Some("web"), Some(&def.id), None),
    ))
}

/// Asserts the rules refuse a disallowed tool called by name — direct and
/// deferred, exact and globbed — keep it off the catalogue, and keep it out
/// of `tool_search` results, while the allowed tools still work.
async fn assert_disallowed_tools_are_refused(rules: Arc<ToolRulePolicy>) {
    let shell = CountingTool::direct("shell");
    let file_read = CountingTool::direct("file_read");
    let secret = CountingTool::deferred("vendor_secret_lookup");
    let weather = CountingTool::deferred("weather_lookup");
    let tools = [
        shell.clone(),
        file_read.clone(),
        secret.clone(),
        weather.clone(),
    ];
    let script = vec![
        calling(&[(
            "search",
            "tool_search",
            serde_json::json!({ "query": "lookup" }),
        )]),
        calling(&[
            ("by-name", "shell", serde_json::json!({})),
            ("guessed", "vendor_secret_lookup", serde_json::json!({})),
            ("allowed", "file_read", serde_json::json!({})),
        ]),
        ModelResponse::assistant("done"),
    ];

    let (model, messages) = run_under(rules, script, &tools).await;

    let catalogue: Vec<String> = model.requests()[0]
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert!(
        catalogue.contains(&"file_read".to_string()),
        "{catalogue:?}"
    );
    assert!(!catalogue.contains(&"shell".to_string()), "{catalogue:?}");

    let found = tool_text(&messages, "search");
    assert!(found.contains("weather_lookup"), "{found}");
    assert!(
        !found.contains("vendor_secret_lookup"),
        "tool_search must not surface a disallowed tool: {found}"
    );

    for call in ["by-name", "guessed"] {
        let refusal = tool_text(&messages, call);
        assert!(
            refusal.contains("disallowed_tools"),
            "{call} must be refused by the disallowed_tools rule: {refusal}"
        );
    }
    assert_eq!(shell.calls(), 0, "a disallowed tool never runs");
    assert_eq!(secret.calls(), 0, "a disallowed deferred tool never runs");
    assert_eq!(file_read.calls(), 1, "an allowed tool still runs");
}

#[tokio::test]
async fn a_session_refuses_disallowed_tools_by_name_and_in_tool_search() {
    assert_disallowed_tools_are_refused(session_rules(&restricted())).await;
}

#[tokio::test]
async fn a_sub_agent_refuses_its_disallowed_tools_by_name_and_in_tool_search() {
    let child = child_rule_policy(None, None, &restricted()).expect("child rules");
    assert_disallowed_tools_are_refused(child).await;
}

#[tokio::test]
async fn a_sub_agent_refuses_its_own_disallowed_tools_under_a_restricted_parent() {
    let mut parent = definition("orchestrator");
    parent.disallowed_tools = vec!["file_write".into()];
    let parent = turn_rule_policy(
        Arc::new(session_rule_set(None, Some(&parent))),
        rule_context(Some("web"), Some("orchestrator"), None),
    );
    let child = child_rule_policy(Some(&parent), None, &restricted()).expect("child rules");
    assert_disallowed_tools_are_refused(child).await;
}
