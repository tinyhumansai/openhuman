//! Frozen Juice declarations and executors survive configuration changes.

use super::*;

#[tokio::test]
async fn a_resumed_thread_keeps_recorded_juice_specs_and_executors_when_repl_is_off() {
    use crate::inference::tokenjuice::repl_tools_for;

    crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin definitions");
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut active_config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    std::fs::create_dir_all(&active_config.workspace_dir).expect("workspace");
    active_config.context.compaction_enabled = true;
    active_config.tokenjuice.router_enabled = true;
    active_config.tokenjuice.ccr_enabled = true;
    active_config.tokenjuice.repl_handle_enabled = true;
    let mut recorded_specs = repl_tools_for(&active_config)
        .iter()
        .map(|tool| tool.spec())
        .collect::<Vec<_>>();
    assert_eq!(
        recorded_specs
            .iter()
            .map(|spec| spec.name.as_str())
            .collect::<Vec<_>>(),
        tinyjuice_bus::tools::REPL_TOOLS
    );
    for spec in &mut recorded_specs {
        spec.description = format!("frozen declaration for {}", spec.name);
        spec.parameters = serde_json::json!({"type":"object","frozen":spec.name});
    }
    let recorded = ToolSnapshot::new(recorded_specs.clone()).expect("recorded tool snapshot");

    // A fresh process has the same workspace but current policy no longer
    // creates new handles. The resumed thread must still regain only the
    // REPL tools it was sent, with their original declarations.
    let mut current_config = crate::config::Config {
        workspace_dir: active_config.workspace_dir.clone(),
        action_dir: active_config.action_dir.clone(),
        config_path: active_config.config_path.clone(),
        modules: crate::config::schema::ModulesConfig {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    current_config.context.compaction_enabled = false;
    current_config.tokenjuice.router_enabled = false;
    current_config.tokenjuice.ccr_enabled = false;
    current_config.tokenjuice.repl_handle_enabled = false;
    assert!(repl_tools_for(&current_config).is_empty());
    let mut host = crate::agent::OpenHumanSessionHost::from_config_for_agent(
        &crate::core::runtime::DispatchCtx::ambient(),
        &current_config,
        "summarizer",
    )
    .expect("summarizer session");
    host.ensure_runtime_session().expect("runtime session");
    let prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("prelude");

    prelude.adopt_recorded_tools(Some(&recorded));
    assert_eq!(
        prelude
            .mutable
            .lock()
            .expect("prelude state")
            .recorded_repl_tools
            .len(),
        recorded_specs.len()
    );
    prelude.refresh_turn_boundary(false).await.expect("refresh");
    let surface_names = {
        let surface = prelude.tool_surface.lock().expect("tool surface");
        (
            surface
                .tools
                .iter()
                .map(|tool| tool.name().to_string())
                .collect::<Vec<_>>(),
            surface
                .synthesized_tools
                .iter()
                .map(|tool| tool.name().to_string())
                .collect::<Vec<_>>(),
            surface
                .visible_tool_specs
                .iter()
                .map(|spec| spec.name.clone())
                .collect::<Vec<_>>(),
            surface.visible_tool_names.clone(),
        )
    };
    let prepared = prelude.prepare(false).await.expect("prepared surface");
    let prepared = prepared.tools.expect("prepared tool snapshot");

    for recorded_spec in &recorded_specs {
        let prepared_spec = prepared
            .specs()
            .iter()
            .find(|spec| spec.name == recorded_spec.name)
            .unwrap_or_else(|| {
                panic!(
                    "the resumed request lost {}; surface={surface_names:?}",
                    recorded_spec.name
                )
            });
        assert_eq!(prepared_spec.description, recorded_spec.description);
        assert_eq!(prepared_spec.parameters, recorded_spec.parameters);
        let surface = prelude.tool_surface.lock().expect("tool surface");
        assert!(
            surface
                .tools
                .iter()
                .chain(surface.synthesized_tools.iter())
                .any(|tool| tool.name() == recorded_spec.name),
            "the recorded declaration {} needs an executor",
            recorded_spec.name
        );
    }
    let unavailable = {
        let (tools, synthesized_tools) = {
            let surface = prelude.tool_surface.lock().expect("tool surface");
            (
                Arc::clone(&surface.tools),
                Arc::clone(&surface.synthesized_tools),
            )
        };
        let tool = tools
            .iter()
            .chain(synthesized_tools.iter())
            .find(|tool| tool.name() == "juice_find")
            .expect("recorded executor remains registered");
        tool.execute(serde_json::json!({"handle":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}))
            .await
            .expect("tool reports disabled module as a result")
    };
    assert!(
        unavailable.is_error,
        "a recorded executor must fail closed when modules are disabled: {}",
        unavailable.output()
    );
}

#[tokio::test]
async fn resumed_juice_snapshot_does_not_add_live_repl_declarations() {
    use crate::inference::tokenjuice::repl_tools_for;

    crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin definitions");
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");
    config.context.compaction_enabled = true;
    config.tokenjuice.router_enabled = true;
    config.tokenjuice.ccr_enabled = true;
    config.tokenjuice.repl_handle_enabled = true;
    let mut recorded_spec = repl_tools_for(&config)
        .into_iter()
        .find(|tool| tool.name() == tinyjuice_bus::tools::REPL_TOOLS[0])
        .expect("juice_find")
        .spec();
    recorded_spec.description = "frozen juice_find declaration".into();
    recorded_spec.parameters = serde_json::json!({"type":"object","frozen":true});
    let recorded = ToolSnapshot::new(vec![recorded_spec.clone()]).expect("recorded snapshot");

    let mut host = crate::agent::OpenHumanSessionHost::from_config_for_agent(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "summarizer",
    )
    .expect("summarizer session");
    host.ensure_runtime_session().expect("runtime session");
    let prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("prelude");
    prelude.adopt_recorded_tools(Some(&recorded));
    prelude.refresh_turn_boundary(false).await.expect("refresh");
    let prepared = prelude
        .prepare(false)
        .await
        .expect("prepared surface")
        .tools
        .expect("tool snapshot");

    let juice_names = prepared
        .specs()
        .iter()
        .filter(|spec| crate::inference::tokenjuice::is_repl_tool(&spec.name))
        .map(|spec| spec.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(juice_names, [tinyjuice_bus::tools::REPL_TOOLS[0]]);
    let frozen = prepared
        .specs()
        .iter()
        .find(|spec| spec.name == recorded_spec.name)
        .expect("recorded spec");
    assert_eq!(frozen.description, recorded_spec.description);
    assert_eq!(frozen.parameters, recorded_spec.parameters);

    let empty = ToolSnapshot::new(Vec::new()).expect("empty recorded snapshot");
    prelude.adopt_recorded_tools(Some(&empty));
    prelude
        .refresh_turn_boundary(false)
        .await
        .expect("refresh empty");
    let prepared = prelude
        .prepare(false)
        .await
        .expect("prepared empty snapshot")
        .tools
        .expect("tool snapshot");
    assert!(
        prepared
            .specs()
            .iter()
            .all(|spec| !crate::inference::tokenjuice::is_repl_tool(&spec.name)),
        "an empty frozen transcript must not gain Juice declarations from the live catalog"
    );
}

/// `[components] juice = false` end to end through a turn's wiring: the
/// session builds its tool-output middleware with compaction and the summary
/// stage off, the tinyjuice module is never claimed, and a large compressible
/// tool result enters the transcript uncompressed.
#[tokio::test]
async fn juice_off_turn_loads_no_module_and_passes_tool_output_through() {
    use crate::agent::tinyagents::middleware::tool_output::ToolOutputMiddleware;
    use tinyagents_harness::context::{RunConfig, RunContext};
    use tinyagents_harness::middleware::{Middleware, ToolInvocationIdentity};

    crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin definitions");
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        modules: crate::config::schema::ModulesConfig {
            enabled: true,
            allow_download: false,
            ..Default::default()
        },
        ..Default::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");
    config.context.compaction_enabled = true;
    config.tokenjuice.router_enabled = true;
    assert!(config.components.set("juice", false));

    let mut host = crate::agent::OpenHumanSessionHost::from_config_for_agent(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "summarizer",
    )
    .expect("summarizer session");
    host.ensure_runtime_session().expect("runtime session");
    let context_mw = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .context_middleware
        .clone()
        .expect("context middleware");
    assert!(
        !context_mw.tokenjuice_compaction_enabled,
        "juice off must switch tool-output compaction off"
    );
    assert!(
        context_mw.payload_summarizer.is_none(),
        "juice off must drop the summary stage"
    );

    // Run the middleware exactly as the turn would, on a result that
    // compaction would otherwise rewrite (a large uniform JSON array).
    let rows: Vec<serde_json::Value> = (0..200)
        .map(|i| serde_json::json!({"id": i, "name": format!("row-{i}"), "status": "ok"}))
        .collect();
    let payload = serde_json::to_string(&rows).expect("payload");
    assert!(payload.len() >= config.tokenjuice.min_bytes_to_compress);
    let mw = ToolOutputMiddleware {
        budget_bytes: 1_000_000,
        payload_summarizer: context_mw.payload_summarizer.clone(),
        artifact_store: None,
        tokenjuice_compaction_enabled: context_mw.tokenjuice_compaction_enabled,
        tokenjuice_compression: context_mw.tokenjuice_compression,
        runtime_config: context_mw.runtime_config.clone(),
        tool_policies: Default::default(),
        artifact_reads: Default::default(),
        focus_by_call: Default::default(),
        summary_focus_tools: Default::default(),
        raw_fetches: Default::default(),
        file_reads: Default::default(),
    };
    let mut ctx = RunContext::new(
        RunConfig::new("juice-off-turn"),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );
    let mut result = tinytools::ToolResult::success(&payload);
    mw.after_tool(
        &mut ctx,
        &(),
        &ToolInvocationIdentity::new(
            tinyagents_harness::ids::CallId::new("juice-off-1"),
            "some_other_tool",
        ),
        &mut result,
    )
    .await
    .expect("after_tool");
    assert_eq!(result.output(), payload, "tool output must pass through");

    // The module was never asked for: the refusal is typed and nothing was
    // downloaded (downloads are off, so a claim would have ended in Failed).
    // The loader only exists with the `modules` feature.
    #[cfg(feature = "modules")]
    {
        let err = crate::modules::ensure_loaded_within(&config, "tinyjuice", None)
            .await
            .unwrap_err();
        assert_eq!(err, tinybus::module::LoadError::Disabled("juice"));
    }
}
