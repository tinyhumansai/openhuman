//! An embedded agent on the harness domains gets the scheduler and clock tools
//! it names, although their domain family is off, and nothing else from that
//! family: not the shell or file tools, even when it names them too. `cron` is
//! deferred (found with `tool_search`), so it is called rather than listed.
//! The default definition's sub-agents stay out too: the scope names no
//! spawn tool.
//!
//! One runtime per process, so one test in this binary.

mod common;

use std::sync::Arc;

use common::{
    chat_requests, offline_config, runtime, stub_backend, tool_call_completion, tool_names,
};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, DomainSet, GroupMode, HostTurnTools, InMemorySessionStores,
    Runtime, ToolGroups, ToolScopeSpec, Workspace,
};

#[test]
fn a_named_scope_admits_the_clock_and_scheduler_but_never_the_shell() {
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            // The turn calls `cron` (a deferred tool, so not listed up front)
            // and then answers.
            let model = common::scripted_provider(
                vec![tool_call_completion("cron", r#"{"action":"list"}"#)],
                "ok",
            )
            .await;
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Stateless)
                .session_store(Arc::new(InMemorySessionStores::new()))
                .backend_url(backend.uri())
                .domains(DomainSet {
                    inference: true,
                    ..DomainSet::harness()
                })
                // Packs off except scheduling, so `cron` is not packed away.
                .tool_groups(ToolGroups::none().with("scheduling", GroupMode::Advertised))
                .build()
                .await
                .expect("runtime builds");
            let named = ["cron", "current_time", "resolve_time", "shell", "file_read"];
            let agent = runtime
                .agent(
                    AgentSpec::new("teeny")
                        .provider(common::route(&model, "fixture"))
                        .definition(AgentDefinitionSpec::new().system_prompt("TEENY").tools(
                            ToolScopeSpec::Named(named.iter().map(|n| (*n).to_string()).collect()),
                        ))
                        .tools(|_| HostTurnTools::advertised(Vec::new())),
                )
                .expect("agent instantiates");
            agent.run("hello").await.expect("the turn runs");

            let requests = chat_requests(&model).await;
            let offered = tool_names(&requests[0]);
            for admitted in ["current_time", "resolve_time", "tool_search"] {
                assert!(
                    offered.iter().any(|name| name == admitted),
                    "{admitted} missing: {offered:?}"
                );
            }
            // Neither the shell and files, nor the default definition's
            // sub-agents: a named scope that names no spawn tool delegates
            // nothing, on the first turn or any later one.
            for refused in [
                "shell",
                "file_read",
                "manage_tasks",
                "analyze_image",
                "create_image",
                "create_video",
            ] {
                assert!(
                    !offered.iter().any(|name| name == refused),
                    "{refused} offered: {offered:?}"
                );
            }
            let result = common::tool_results(&requests[1]);
            assert!(
                !result.to_lowercase().contains("unknown") && !result.contains("not available"),
                "cron did not run: {result}"
            );
            drop(agent);
            drop(runtime);
        })
        .await
        .expect("library host task did not panic");
    });
}
