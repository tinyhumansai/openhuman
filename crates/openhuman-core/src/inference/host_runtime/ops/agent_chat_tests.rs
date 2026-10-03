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
