use super::super::channel::CHANNEL_OUTBOUND_EVENT;
use super::*;
use crate::agent::bus::{mock_agent_run_turn, AgentTurnRequest, AgentTurnResponse};
use crate::agent::turn_origin::AgentTurnOrigin;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
use std::sync::Mutex;

fn config(tmp: &tempfile::TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    config.local_ai.runtime_enabled = false;
    config.memory.conversations.enabled = false;
    config.agent.session_dual_write = false;
    config.agent.session_shadow_reads = false;
    config
}

fn params(message_id: &str, text: &str) -> RelayInboundParams {
    serde_json::from_value(json!({
        "channel": "telegram",
        "chat_id": "-100",
        "sender_id": "42",
        "sender_name": "Ada",
        "message_id": message_id,
        "text": text,
        "client_id": "gw-ops-test",
    }))
    .expect("params")
}

/// What the stubbed `agent.run_turn` saw.
#[derive(Default)]
struct Seen {
    histories: Vec<Vec<(String, String)>>,
    origins: Vec<AgentTurnOrigin>,
    /// Whether the relayed thread was busy (for background delivery) while
    /// the turn ran.
    busy: Vec<bool>,
}

#[tokio::test]
async fn a_relayed_turn_runs_as_an_external_channel_and_keeps_its_thread() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let stub_seen = Arc::clone(&seen);
    let relayed_thread = params("m1", "x").thread_id();
    let _bus = mock_agent_run_turn(move |req: AgentTurnRequest| {
        let mut seen = stub_seen.lock().unwrap();
        seen.busy
            .push(crate::agent::orchestration::busy_guard::is_busy(
                &relayed_thread,
            ));
        seen.histories.push(
            req.history
                .iter()
                .map(|m| (m.role.clone(), m.content.clone()))
                .collect(),
        );
        seen.origins.push(req.origin.clone());
        let n = seen.histories.len();
        async move { Ok(AgentTurnResponse::new(format!("answer {n}"))) }
    })
    .await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config(&tmp);
    let workspace = config.workspace_dir.clone();
    let mut events = crate::web_chat::subscribe_web_channel_events();

    for (id, text) in [("m1", "first question"), ("m2", "second question")] {
        let p = params(id, text);
        let thread = p.thread_id();
        assert_eq!(
            store::record_inbound(&workspace, &p, &thread).await,
            Ok(Recorded::New)
        );
        let replies = run_relay_turn(config.clone(), p, thread, format!("req-{id}")).await;
        assert_eq!(replies.len(), 1, "{replies:?}");
    }

    let seen = seen.lock().unwrap();
    assert_eq!(seen.histories.len(), 2, "one agent turn per message");
    assert_eq!(
        seen.busy,
        vec![true, true],
        "a relayed turn marks its thread busy, so a background result waits"
    );
    assert!(
        !crate::agent::orchestration::busy_guard::is_busy(&params("m1", "x").thread_id()),
        "the thread is idle once the turn ends"
    );
    for origin in &seen.origins {
        match origin {
            AgentTurnOrigin::ExternalChannel {
                channel, sender, ..
            } => {
                assert_eq!(channel, "telegram");
                assert_eq!(sender.as_deref(), Some("42"));
            }
            other => panic!("relayed turns run as an external channel, got {other:?}"),
        }
    }
    // The second turn is seeded from the thread: the first exchange, then
    // the new message.
    let second = &seen.histories[1];
    let contents: Vec<&str> = second.iter().map(|(_, c)| c.as_str()).collect();
    assert!(
        contents.iter().any(|c| c.contains("first question")),
        "{contents:?}"
    );
    assert!(contents.contains(&"answer 1"), "{contents:?}");
    assert!(
        contents
            .last()
            .is_some_and(|c| c.contains("second question")),
        "{contents:?}"
    );

    let thread = params("m1", "x").thread_id();
    let messages = crate::threads::store::get_messages(workspace.clone(), &thread).unwrap();
    let ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["user:m1", "assistant:m1", "user:m2", "assistant:m2"]
    );
    assert_eq!(messages[1].content, "answer 1");

    let mut outbound = Vec::new();
    while let Ok(event) = events.try_recv() {
        if event.client_id == "gw-ops-test" {
            outbound.push(event);
        }
    }
    assert_eq!(outbound.len(), 2, "one channel_outbound per reply");
    assert!(outbound.iter().all(|e| e.event == CHANNEL_OUTBOUND_EVENT));
    assert!(outbound.iter().all(|e| e.thread_id == thread));
    assert_eq!(outbound[0].request_id, "req-m1");
    assert_eq!(outbound[0].full_response.as_deref(), Some("answer 1"));
    assert_eq!(outbound[0].structured.as_ref().unwrap()["chat_id"], "-100");
}

#[tokio::test]
async fn invalid_or_repeated_messages_start_no_turn() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config(&tmp);
    let workspace = config.workspace_dir.clone();
    let ctx = CoreContext::for_test(DomainSet::full(), Some(workspace.clone())).derive_with(
        ContextOverlay::new(config, DomainSet::full(), Default::default()),
    );

    CoreContext::scope(ctx, async {
        let mut bad = params("m1", "hello");
        bad.chat_id = "a/b".into();
        let err = channel_relay_inbound(bad).await.expect_err("invalid");
        assert!(err.contains("chat_id"), "{err}");

        let p = params("m1", "hello");
        let thread = p.thread_id();
        store::record_inbound(&workspace, &p, &thread)
            .await
            .unwrap();
        let outcome = channel_relay_inbound(p)
            .await
            .expect("duplicate is not an error");
        let value = outcome.into_cli_compatible_json().unwrap();
        let text = value.to_string();
        assert!(text.contains("\"duplicate\":true"), "{text}");
        assert!(text.contains(&thread), "{text}");
    })
    .await;
}
