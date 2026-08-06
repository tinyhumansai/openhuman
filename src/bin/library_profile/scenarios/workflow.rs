//! `workflow`: run a real flows-domain workflow end to end. A
//! trigger -> transform -> agent graph is created OUTSIDE the measured region
//! (recorded as a checkpoint), then `flows_run` is measured as the workload.
//! The agent node's LLM routes through the plain-text mock provider.

use anyhow::Result;
use openhuman_core::core::event_bus::init_global;
use openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry;
use openhuman_core::openhuman::flows::ops::{flows_create, flows_run};
use openhuman_core::openhuman::flows::FlowRunTrigger;
use openhuman_core::openhuman::inference::provider::factory::test_provider_override;
use serde_json::json;

use crate::harness::{fixture, measure, ProfileResult};
use crate::mock::PlainTextMock;

pub async fn run() -> Result<ProfileResult> {
    let fixture = fixture()?;
    let _ = init_global(256);
    openhuman_core::openhuman::agent::bus::register_agent_handlers();
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let mock = PlainTextMock::new("Phoenix migration status: healthy, ramp on Friday.");
    let _provider = test_provider_override::install_model(mock.clone());

    let graph = json!({
        "name": "profile-workflow",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "prep", "kind": "transform", "name": "Prep",
              "config": { "set": { "topic": "Phoenix migration" } } },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher",
                          "prompt": "Summarise the Phoenix migration status in one line." } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "prep" },
            { "from_node": "prep", "to_node": "summarize" }
        ]
    });

    eprintln!("[library-profile] workflow: creating flow (outside measured region)");
    let flow = flows_create(&fixture.config, "profile-workflow".into(), graph, false)
        .await
        .map_err(anyhow::Error::msg)?
        .value;
    let flow_id = flow.id.clone();

    measure("workflow", 1, None, move |_rec| async move {
        let outcome = flows_run(
            &fixture.config,
            &flow_id,
            json!({ "topic": "Phoenix migration" }),
            serde_json::Map::new(),
            FlowRunTrigger::Rpc,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let output = outcome.value.get("output");
        anyhow::ensure!(
            output.is_some() && !output.unwrap().is_null(),
            "workflow run produced no output: {}",
            outcome.value
        );
        anyhow::ensure!(
            outcome.value.get("note").is_none(),
            "workflow with an actionable agent node unexpectedly reported nothing-to-run: {}",
            outcome.value
        );
        Ok(())
    })
    .await
}
