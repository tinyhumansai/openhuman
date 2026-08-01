use super::*;

// Origin-gated conversation autosave (#5312).
//
// `turn()` stores the user message only when the turn's text was written by a
// person — `turn_origin::current_is_user_authored`. These cover both directions
// of that allowlist: the two user-authored origins that were not already pinned
// by the `WebChat` case in part 01, and the two host-written ones that must not
// reach the user's conversation memory.

/// Poll for a `user_msg:` conversation memory over the same one-second window
/// the positive autosave test uses, and report whether one ever landed.
///
/// The user message is stored fire-and-forget (`tokio::spawn` in
/// `turn/core.rs`, #3610), so both directions need the *same* window or the
/// negative tests are the weaker assertion: a broken guard whose spawned store
/// lands after a short fixed sleep would pass them while failing in production.
/// Returns as soon as one appears, so a genuinely broken guard fails fast
/// instead of costing the full second.
async fn poll_for_stored_user_message(mem: &Arc<dyn Memory>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for _ in 0..50 {
        keys = mem
            .list(None, None, None)
            .await
            .expect(
                "memory list must succeed; an empty list here would let the \
                     autosave assertions below pass without reading storage",
            )
            .into_iter()
            .map(|e| e.key)
            .collect();
        if keys.iter().any(|k| k.starts_with("user_msg:")) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    keys
}

/// `ExternalChannel` is in the user-authored allowlist alongside `WebChat`, so
/// it needs its own positive case: a Telegram/Discord/Slack message is a person
/// talking, and the allowlist would be half-tested if only the web thread had a
/// stored-message assertion.
#[tokio::test]
async fn an_external_channel_turn_stores_the_user_message() {
    use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin};

    let (mem, _tmp) = make_sqlite_memory();
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("got it")]));
    let (mut agent, _tmp2) = build_agent_with_memory(provider, vec![], mem.clone(), true);

    let origin = AgentTurnOrigin::ExternalChannel {
        channel: "telegram".into(),
        sender: Some("user-42".into()),
        reply_target: "chat-7".into(),
        message_id: "m-1".into(),
    };
    let _ = with_origin(origin, agent.turn("remember my flight is at nine"))
        .await
        .unwrap();

    let keys = poll_for_stored_user_message(&mem).await;
    assert!(
        keys.iter().any(|k| k.starts_with("user_msg:")),
        "a channel message is a person talking and must be stored: {keys:?}"
    );
}

/// The desktop Settings agent-chat panel calls `openhuman.agent_chat`, which
/// scopes `DirectChat`. That is a person typing, so it stores — the origin
/// exists precisely because `Cli` (its previous label, chosen for the approval
/// gate) would have dropped these messages.
#[tokio::test]
async fn a_direct_chat_turn_stores_the_user_message() {
    use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin};

    let (mem, _tmp) = make_sqlite_memory();
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("noted")]));
    let (mut agent, _tmp2) = build_agent_with_memory(provider, vec![], mem.clone(), true);

    let _ = with_origin(
        AgentTurnOrigin::DirectChat,
        agent.turn("my dog is called Pip"),
    )
    .await
    .unwrap();

    let keys = poll_for_stored_user_message(&mem).await;
    assert!(
        keys.iter().any(|k| k.starts_with("user_msg:")),
        "a direct-chat message is a person typing and must be stored: {keys:?}"
    );
}


/// An internal agent runs on the same config as the chat, so it inherits
/// `auto_save` — but its "user message" is the prompt the host wrote for it.
/// Storing that as a conversation memory puts prompt boilerplate where the
/// user's own words belong, and it competes for slots in every later recall
/// (#5312). A `TrustedAutomation` turn therefore saves nothing.
#[tokio::test]
async fn an_automation_turn_does_not_store_its_prompt_as_the_users_memory() {
    use crate::openhuman::agent::turn_origin::{
        with_origin, AgentTurnOrigin, TrustedAutomationSource,
    };

    let (mem, _tmp) = make_sqlite_memory();
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("goals updated")]));
    let (mut agent, _tmp2) = build_agent_with_memory(
        provider,
        vec![],
        mem.clone(),
        true, // auto_save enabled, exactly as a config-built internal agent gets it
    );

    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "memory_goals:enrich:1".into(),
        source: TrustedAutomationSource::Subconscious,
    };
    let _ = with_origin(origin, agent.turn("Maintain the existing goals list."))
        .await
        .unwrap();

    // The fire-and-forget store would land shortly after the turn returns, so
    // give it the same one-second window the positive tests poll for before
    // concluding it never happened.
    let keys = poll_for_stored_user_message(&mem).await;
    assert!(
        !keys.iter().any(|k| k.starts_with("user_msg:")),
        "an automation prompt must not be stored as a user message: {keys:?}"
    );
}

/// An unscoped turn is not a user turn either. `turn_origin` documents that
/// every entry point scopes an origin and that an unlabelled one fails closed;
/// the autosave follows the same allowlist, so a caller that forgets cannot
/// quietly write host text into the user's memory.
#[tokio::test]
async fn an_unscoped_turn_stores_no_user_message() {
    let (mem, _tmp) = make_sqlite_memory();
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("ok")]));
    let (mut agent, _tmp2) = build_agent_with_memory(provider, vec![], mem.clone(), true);

    let _ = agent.turn("who wrote this?").await.unwrap();

    let keys = poll_for_stored_user_message(&mem).await;
    assert!(
        !keys.iter().any(|k| k.starts_with("user_msg:")),
        "an unscoped turn must not be credited to the user: {keys:?}"
    );
}
