use super::*;

#[test]
fn env_overlay_context_tool_result_budget_env_suppresses_legacy_migration() {
    // If the env var is *present*, the `agent.tool_result_budget_bytes`
    // migration must NOT run — even when the explicit env value equals
    // the default. This protects users who explicitly set the env to
    // the default.
    let default_budget = tinyagents_harness::config::TurnConfig::default().tool_result_budget_bytes;
    let mut cfg = Config::default();
    cfg.context.tool_result_budget_bytes = default_budget;
    cfg.agent.tool_result_budget_bytes = 999_999;

    cfg.apply_env_overlay_with(&HashMapEnv::new().with(
        "OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES",
        &default_budget.to_string(),
    ));
    assert_eq!(
        cfg.context.tool_result_budget_bytes, default_budget,
        "env presence must suppress the legacy agent→context copy"
    );
}

#[test]
fn env_overlay_compaction_default_on_and_switch() {
    // Default is on: large results become a stats line, a head and a handle
    // the REPL tools query.
    assert!(Config::default().context.compaction_enabled);
    assert!(Config::default().tokenjuice.router_enabled);
    assert!(Config::default().tokenjuice.repl_handle_enabled);

    // `OPENHUMAN_COMPACTION=0` opts out; `=1` keeps it on.
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_COMPACTION", "0"));
    assert!(!cfg.context.compaction_enabled);
    assert!(!crate::inference::tokenjuice::repl_handle_active(&cfg));
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_COMPACTION", "1"));
    assert!(cfg.context.compaction_enabled);

    // Truthy re-enables; the namespaced alias works too.
    let mut cfg = Config::default();
    cfg.context.compaction_enabled = false;
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CONTEXT_COMPACTION_ENABLED", "on"),
    );
    assert!(cfg.context.compaction_enabled);

    // Garbage is ignored (leaves the prior value untouched).
    let mut cfg = Config::default();
    cfg.context.compaction_enabled = true;
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_COMPACTION", "maybe"));
    assert!(cfg.context.compaction_enabled);
}

#[test]
fn env_overlay_context_tool_result_budget_legacy_migration_when_env_absent() {
    // Env absent, context at default, agent customised → agent value copies forward.
    let default_budget = tinyagents_harness::config::TurnConfig::default().tool_result_budget_bytes;
    let mut cfg = Config::default();
    cfg.context.tool_result_budget_bytes = default_budget;
    cfg.agent.tool_result_budget_bytes = 777_777;

    cfg.apply_env_overlay_with(&HashMapEnv::new());
    assert_eq!(cfg.context.tool_result_budget_bytes, 777_777);
}

#[test]
fn env_overlay_context_tool_result_budget_env_wins_over_legacy_migration() {
    // Env present with a non-default value, and agent also customised.
    // The env value must apply; the legacy agent→context copy must NOT
    // overwrite it.
    let mut cfg = Config::default();
    cfg.agent.tool_result_budget_bytes = 111_111;

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES", "222222"),
    );
    assert_eq!(
        cfg.context.tool_result_budget_bytes, 222_222,
        "env value wins; legacy migration suppressed"
    );
}

#[test]
fn env_overlay_compaction_trigger_tokens_sets_and_clears_the_override() {
    assert_eq!(Config::default().context.compaction_trigger_tokens, None);

    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_COMPACTION_TRIGGER_TOKENS", " 64000 "),
    );
    assert_eq!(cfg.context.compaction_trigger_tokens, Some(64_000));

    // `0` clears an override set in config.toml.
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_COMPACTION_TRIGGER_TOKENS", "0"));
    assert_eq!(cfg.context.compaction_trigger_tokens, None);

    // Garbage is ignored (leaves the prior value untouched).
    let mut cfg = Config::default();
    cfg.context.compaction_trigger_tokens = Some(10);
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_COMPACTION_TRIGGER_TOKENS", "lots"),
    );
    assert_eq!(cfg.context.compaction_trigger_tokens, Some(10));
}

#[test]
fn compaction_strategy_defaults_to_task_state_and_env_selects_it() {
    use crate::config::CompactionStrategy;
    assert_eq!(
        Config::default().context.compaction_strategy,
        CompactionStrategy::TaskState
    );

    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_COMPACTION_STRATEGY", " Summary "),
    );
    assert_eq!(cfg.context.compaction_strategy, CompactionStrategy::Summary);
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_COMPACTION_STRATEGY", "task-state"),
    );
    assert_eq!(
        cfg.context.compaction_strategy,
        CompactionStrategy::TaskState
    );

    // Unknown and empty values leave the current strategy alone.
    cfg.context.compaction_strategy = CompactionStrategy::Summary;
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_COMPACTION_STRATEGY", "magic"));
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_COMPACTION_STRATEGY", ""));
    assert_eq!(cfg.context.compaction_strategy, CompactionStrategy::Summary);
}

#[test]
fn compaction_strategy_reads_from_toml() {
    use crate::config::CompactionStrategy;
    let cfg: crate::config::ContextConfig =
        toml::from_str("compaction_strategy = \"summary\"").unwrap();
    assert_eq!(cfg.compaction_strategy, CompactionStrategy::Summary);
    let cfg: crate::config::ContextConfig = toml::from_str("").unwrap();
    assert_eq!(cfg.compaction_strategy, CompactionStrategy::TaskState);
}

#[test]
fn compaction_settings_bundle_the_trigger_and_strategy() {
    use crate::config::{CompactionSettings, CompactionStrategy, ContextConfig};
    let mut cfg = ContextConfig::default();
    assert_eq!(cfg.compaction_settings(), CompactionSettings::default());
    cfg.compaction_trigger_tokens = Some(0);
    cfg.compaction_strategy = CompactionStrategy::Summary;
    let settings = cfg.compaction_settings();
    assert_eq!(settings.trigger_tokens, None, "a 0 trigger is no override");
    assert_eq!(settings.strategy, CompactionStrategy::Summary);
    cfg.compaction_trigger_tokens = Some(64_000);
    assert_eq!(cfg.compaction_settings().trigger_tokens, Some(64_000));
}

#[test]
fn env_overlay_keeps_unattended_browser_actions_and_unknown_names_allow_nothing() {
    let mut cfg = Config::default();
    cfg.browser.unattended_actions = vec!["click".into(), "navigate".into()];
    cfg.apply_env_overlay_with(&HashMapEnv::new());
    assert_eq!(cfg.browser.unattended_actions, vec!["click", "navigate"]);
    assert!(cfg.browser.allows_unattended("click"));
    assert!(!cfg.browser.allows_unattended("navigate"));
}

#[test]
fn unknown_unattended_browser_actions_are_counted_not_echoed() {
    #[derive(Clone, Default)]
    struct Logs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Logs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();

    let mut cfg = Config::default();
    cfg.browser.unattended_actions = vec!["click".into(), "sk-live-not-an-action".into()];
    cfg.apply_env_overlay_with(&HashMapEnv::new());

    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    let line = text
        .lines()
        .find(|line| line.contains("unattended_actions"))
        .unwrap_or_else(|| panic!("no unattended_actions warning in {text:?}"));
    assert!(line.contains("count=1"), "{line}");
    assert!(!line.contains("sk-live-not-an-action"), "{line}");
}

#[test]
fn summarizer_tool_rendering_defaults_to_full_and_env_selects_markers() {
    use tinyagents_harness::summarization::ToolRendering;
    assert_eq!(
        Config::default().context.summarizer_tool_rendering,
        ToolRendering::Full
    );

    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CONTEXT_SUMMARIZER_TOOL_RENDERING", " Markers "),
    );
    assert_eq!(
        cfg.context.summarizer_tool_rendering,
        ToolRendering::Markers
    );

    // Unknown and empty values leave the current rendering alone.
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CONTEXT_SUMMARIZER_TOOL_RENDERING", "terse"),
    );
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CONTEXT_SUMMARIZER_TOOL_RENDERING", ""),
    );
    assert_eq!(
        cfg.context.summarizer_tool_rendering,
        ToolRendering::Markers
    );

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CONTEXT_SUMMARIZER_TOOL_RENDERING", "full"),
    );
    assert_eq!(cfg.context.summarizer_tool_rendering, ToolRendering::Full);
}

#[test]
fn summarizer_max_output_tokens_env_sets_clears_and_ignores_garbage() {
    assert_eq!(Config::default().context.summarizer_max_output_tokens, None);

    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CONTEXT_SUMMARIZER_MAX_OUTPUT_TOKENS", " 4000 "),
    );
    assert_eq!(cfg.context.summarizer_max_output_tokens, Some(4_000));

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CONTEXT_SUMMARIZER_MAX_OUTPUT_TOKENS", "lots"),
    );
    assert_eq!(cfg.context.summarizer_max_output_tokens, Some(4_000));

    // `0` clears a cap set in config.toml.
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CONTEXT_SUMMARIZER_MAX_OUTPUT_TOKENS", "0"),
    );
    assert_eq!(cfg.context.summarizer_max_output_tokens, None);
}

#[test]
fn summarizer_settings_read_from_the_context_table_beside_the_harness_settings() {
    use tinyagents_harness::summarization::ToolRendering;
    let cfg: Config = toml::from_str(
        "[context]\n\
         compaction_strategy = \"summary\"\n\
         summarizer_model = \"scaledown/summarize\"\n\
         summarizer_tool_rendering = \"markers\"\n\
         summarizer_max_output_tokens = 4000\n",
    )
    .unwrap();
    assert_eq!(
        cfg.context.compaction_strategy,
        crate::config::CompactionStrategy::Summary
    );
    assert_eq!(
        cfg.context.summarizer_model.as_deref(),
        Some("scaledown/summarize")
    );
    let summarizer = cfg.context.summarizer_settings();
    assert_eq!(summarizer.tool_rendering, ToolRendering::Markers);
    assert_eq!(summarizer.max_output_tokens, Some(4_000));

    // The written form keeps every key in the one `[context]` table.
    let written = toml::to_string(&cfg.context).unwrap();
    assert!(
        written.contains("summarizer_tool_rendering = \"markers\""),
        "{written}"
    );
    assert!(
        written.contains("compaction_strategy = \"summary\""),
        "{written}"
    );
}
