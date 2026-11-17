//! A finished turn must release every clone of its progress sender once the
//! caller clears the sink. The web channel's progress bridge only exits when
//! its receiver closes, so a clone retained by the cached session (the
//! committed receipt's run context carries `progress` and the attached
//! parent's `on_progress`) kept one bridge task — and in SaaS mode its
//! profile's turn context — alive per turn.

use super::*;

#[tokio::test]
async fn clearing_progress_after_a_turn_closes_the_receiver() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        text_response("first"),
        text_response("second"),
    ]));
    let (mut agent, _tmp) =
        build_agent_with(provider, vec![Box::new(EchoTool)], Box::new(NativeDialect));

    // Two turns on the same (cached) session host, each with its own sink, as
    // the web channel does.
    for message in ["hi", "again"] {
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let weak = tx.downgrade();
        agent.set_on_progress(Some(tx));
        agent.turn(message).await.expect("turn");
        agent.set_on_progress(None);

        assert_eq!(
            weak.strong_count(),
            0,
            "the session kept progress sender clone(s) alive after the turn"
        );
        // The turn's own events are still delivered, then the channel closes.
        let mut saw_completed = false;
        while let Some(event) = rx.recv().await {
            saw_completed |= matches!(event, crate::progress::AgentProgress::TurnCompleted { .. });
        }
        assert!(saw_completed, "turn must still report TurnCompleted");
    }
}
