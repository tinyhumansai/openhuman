use super::*;
use serde_json::json;

/// A turn that finished on its own keeps the historical envelope and adds
/// `hit_cap: false`; no `checkpoint` key appears.
#[test]
fn a_finished_turn_reports_hit_cap_false() {
    let json = AgentChatReply {
        text: "done".into(),
        hit_cap: false,
    }
    .into_rpc_json()
    .expect("serializes");

    assert_eq!(
        json,
        json!({ "result": "done", "logs": ["agent chat completed"], "hit_cap": false })
    );
}

/// #6958: a capped turn used to look like success to a headless caller. It
/// now says so, and carries the checkpoint text as its own field.
#[test]
fn a_capped_turn_reports_hit_cap_and_the_checkpoint() {
    let json = AgentChatReply {
        text: "Changed foo.rs. **Still to do**: bar.rs".into(),
        hit_cap: true,
    }
    .into_rpc_json()
    .expect("serializes");

    assert_eq!(json["hit_cap"], json!(true));
    assert_eq!(
        json["checkpoint"],
        json!("Changed foo.rs. **Still to do**: bar.rs")
    );
}

/// Backward compatibility: existing clients read the reply from `result` (or
/// peel the envelope with `unwrap_rpc`), and still get the plain string.
#[test]
fn the_reply_text_stays_where_existing_clients_read_it() {
    for hit_cap in [false, true] {
        let json = AgentChatReply {
            text: "the reply".into(),
            hit_cap,
        }
        .into_rpc_json()
        .expect("serializes");
        assert_eq!(json["result"], json!("the reply"));
        assert_eq!(json["logs"], json!(["agent chat completed"]));
        assert_eq!(
            crate::core::unwrap_rpc(&json),
            &json!("the reply"),
            "unwrap_rpc must still land on the reply text"
        );
    }
}

/// The library path (`agent_chat_for`) keeps returning `Outcome<String>`.
#[test]
fn the_outcome_keeps_its_historical_shape() {
    let outcome = AgentChatReply {
        text: "hi".into(),
        hit_cap: true,
    }
    .into_outcome();
    assert_eq!(outcome.value, "hi");
    assert_eq!(outcome.logs, vec!["agent chat completed".to_string()]);
}

#[test]
fn explicit_turn_temperature_overrides_the_definition_without_mutating_it() {
    let mut definition = crate::agent::harness::builtin_definitions::test_main_def();
    definition.temperature = 0.6;
    let effective = definition_for_turn(&definition, Some(0.9));
    assert_eq!(effective.temperature, 0.9);
    assert_eq!(definition.temperature, 0.6);
    assert!(matches!(
        definition_for_turn(&definition, None),
        std::borrow::Cow::Borrowed(_)
    ));
    assert_eq!(definition_for_turn(&definition, None).temperature, 0.6);
}

fn chat_origin(thread: &str) -> crate::agent::turn_origin::AgentTurnOrigin {
    crate::agent::turn_origin::AgentTurnOrigin::WebChat {
        thread_id: thread.into(),
        client_id: "client".into(),
        request_id: format!("req-{thread}").into(),
    }
}

fn chat_thread(origin: crate::agent::turn_origin::AgentTurnOrigin) -> String {
    match origin {
        crate::agent::turn_origin::AgentTurnOrigin::WebChat { thread_id, .. } => thread_id.into(),
        crate::agent::turn_origin::AgentTurnOrigin::DirectChat => "direct".into(),
        other => panic!("unexpected origin {other:?}"),
    }
}

#[tokio::test]
async fn agent_chat_origin_comes_from_the_turn_context_before_the_task_scope() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    let base = CoreContext::for_test(DomainSet::full(), None);
    let own = DispatchCtx::new(base.derive_channel_turn(None, Some(chat_origin("own"))));
    let scoped = base.derive_channel_turn(None, Some(chat_origin("scoped")));
    CoreContext::scope(scoped, async {
        // The turn's own context wins over whatever the task scope carries.
        assert_eq!(chat_thread(effective_agent_chat_origin(&own)), "own");
        // A context with no origin of its own falls back to the task scope.
        let bare = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None));
        assert_eq!(chat_thread(effective_agent_chat_origin(&bare)), "scoped");
    })
    .await;
    // Neither: the trusted direct-chat label.
    let bare = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None));
    assert_eq!(chat_thread(effective_agent_chat_origin(&bare)), "direct");
}
