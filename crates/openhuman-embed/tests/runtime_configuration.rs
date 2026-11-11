//! One process owns one runtime: verify defaults, templates and storage together.
mod common;
use common::{chat_requests, offline_config, provider, runtime};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentError, AgentSpec, ModelDefaults, RuntimeBuilder, ServiceSet,
    ToolScopeSpec,
};

#[test]
fn runtime_defaults_templates_sampling_and_storage_are_isolated() {
    runtime().block_on(async {
        tokio::spawn(assert_runtime_configuration()).await.unwrap();
    });
}

async fn assert_runtime_configuration() {
    let provider = provider("configured reply").await;
    let mut config = offline_config();
    config.default_model = Some("gpt-4o-mini".into());
    config.default_temperature = 0.1;
    config.storage.url = Some("memory".into());
    let rules: openhuman_embed::config::ToolRules = serde_json::from_value(
        serde_json::json!({"rules":[{"effect":"deny","match":{"name":"shell"}}]}),
    )
    .unwrap();
    let runtime = RuntimeBuilder::lean()
        .config(config)
        .services(ServiceSet::none())
        .provider(openhuman_embed::Provider::openai_compatible(
            format!("{}/v1", provider.uri()),
            "local-test",
        ))
        .model_defaults(ModelDefaults {
            temperature: Some(0.3),
            max_tokens: Some(90),
            top_p: Some(0.8),
            max_iterations: Some(5),
        })
        .autonomy(openhuman_embed::config::AutonomyConfig {
            max_actions_per_hour: 7,
            ..Default::default()
        })
        .privacy(openhuman_embed::config::PrivacyConfig {
            mode: openhuman_embed::config::PrivacyMode::Sensitive,
        })
        .cron(openhuman_embed::config::CronConfig {
            enabled: false,
            ..Default::default()
        })
        .secrets(openhuman_embed::config::SecretsConfig { encrypt: false })
        .learning(openhuman_embed::LearningSettings {
            build_beliefs_every: 21,
            learnings_limit: 3,
        })
        .tool_rules(rules.clone())
        .access(Access::readonly())
        .build()
        .await
        .unwrap();
    assert_eq!(runtime.defaults().model.temperature, Some(0.3));
    assert_eq!(
        runtime.capabilities().storage.driver.as_deref(),
        Some("memory")
    );
    assert!(runtime.storage().is_some());
    runtime
        .define_template(
            "shared",
            AgentDefinitionSpec::new()
                .bare_prompt("Shared analyst.")
                .tools(ToolScopeSpec::Named(Vec::new())),
        )
        .unwrap();
    assert_eq!(runtime.defaults().templates.len(), 1);
    assert!(
        matches!(runtime.agent(AgentSpec::new("missing").extends("unknown")),Err(AgentError::UnknownTemplate(name)) if name=="unknown")
    );
    let first = runtime
        .agent(
            AgentSpec::new("first")
                .model("gpt-4o-mini")
                .extends("shared")
                .model_defaults(ModelDefaults {
                    temperature: Some(0.6),
                    max_tokens: Some(120),
                    ..Default::default()
                }),
        )
        .unwrap();
    let second = runtime
        .agent(AgentSpec::new("second").extends("shared"))
        .unwrap();
    assert_eq!(first.config().default_model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(second.config().default_temperature, 0.3);
    assert_eq!(first.config().autonomy.max_actions_per_hour, 7);
    assert_eq!(first.config().tool_rules, rules);
    assert!(runtime.capabilities().configuration.has_tool_rules);
    assert_eq!(
        first.config().privacy.mode,
        openhuman_embed::config::PrivacyMode::Sensitive
    );
    assert!(!first.config().cron.enabled);
    assert!(!first.config().secrets.encrypt);
    assert_eq!(first.config().memory.recall.build_beliefs_every, 21);
    assert_eq!(first.config().memory.recall.learnings_limit, 3);
    first.turn("first question").send().await.unwrap();
    second.turn("second question").send().await.unwrap();
    first
        .turn("override question")
        .temperature(0.9)
        .max_tokens(150)
        .send()
        .await
        .unwrap();
    let requests = chat_requests(&provider).await;
    assert_eq!(requests.len(), 3);
    let bodies: Vec<serde_json::Value> = requests
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(bodies[0]["temperature"], 0.6);
    assert_eq!(bodies[0]["max_tokens"], 120);
    assert_eq!(bodies[1]["temperature"], 0.3);
    assert_eq!(bodies[1]["max_tokens"], 90);
    assert_eq!(bodies[2]["temperature"], 0.9);
    assert_eq!(bodies[2]["max_tokens"], 150);
    for body in &bodies {
        assert_eq!(body["top_p"], 0.8);
    }
    assert_eq!(runtime.defaults().model.temperature, Some(0.3));
    assert!(matches!(
        runtime.agent(
            AgentSpec::new("wide")
                .extends("shared")
                .definition(AgentDefinitionSpec::new().tools(ToolScopeSpec::Wildcard))
        ),
        Err(AgentError::WidensRuntime(_))
    ));
}
