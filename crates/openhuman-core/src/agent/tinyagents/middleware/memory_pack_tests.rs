use super::*;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::middleware::Middleware;
use tinyinference_llm::model::ModelRequest;

use std::sync::Arc;

use tinyagents_harness::context::RunConfig;
use tinyinference_llm::message::Message;

use crate::memory::lifecycle::hooks::{MemoryTurn, TurnPack, OPEN_TAG};
use tinymemory_tools::lifecycle::turn::TurnPack as NativeTurnPack;

fn context(pack: Option<TurnPack>) -> RunContext<OpenHumanRunContext> {
    let config = crate::config::Config::default();
    let mut data = OpenHumanRunContext::new();
    data.memory_turn = Some(Arc::new(MemoryTurn {
        identity: tinymemory_tools::layout::identity::Identity::agent("a")
            .resolve(&crate::memory::scope::identity_pins(&config)),
        config: Arc::new(config),
        thread_id: "t".into(),
        pack,
        in_prompt_from: Arc::new(tinymemory_tools::lifecycle::window::InPromptFrom::new(0, 0)),
        core: None,
    }));
    RunContext::new(RunConfig::new("memory-pack-test"), data)
}

fn pack() -> TurnPack {
    TurnPack::from_native(NativeTurnPack {
        markdown: "# Memory\n\n## Learnings\n- The user prefers metric units".into(),
        tokens: 14,
        refs: vec!["id-1".into()],
        engine: "reference".into(),
        citations: Vec::new(),
        refusal: None,
    })
}

#[tokio::test]
async fn the_pack_rides_every_request_of_the_turn() {
    let mut ctx = context(Some(pack()));
    let history = vec![
        Message::system("You are helpful."),
        Message::user("How far is it?"),
    ];

    for _ in 0..2 {
        let mut request = ModelRequest::new(history.clone());
        memory_pack_middleware()
            .before_model(&mut ctx, &(), &mut request)
            .await
            .unwrap();
        assert_eq!(request.messages.len(), 3, "added after the transcript");
        let tail = request.messages.last().unwrap().text();
        assert!(tail.starts_with(OPEN_TAG), "{tail}");
        assert!(tail.contains("metric units"));
        assert_eq!(
            &request.messages[..2],
            &history[..],
            "the transcript is untouched"
        );
    }
}

#[tokio::test]
async fn a_turn_without_a_pack_is_left_alone() {
    let mut ctx = context(None);
    let mut request = ModelRequest::new(vec![Message::user("hi")]);
    memory_pack_middleware()
        .before_model(&mut ctx, &(), &mut request)
        .await
        .unwrap();
    assert_eq!(request.messages.len(), 1);
    assert!(ctx.data.child().memory_turn.is_none());
}

// ── prompt-cache stability across turns (#6718, #7023) ──────────────────────
//
// A provider caches the longest prefix two requests share. The pack is
// recalled per turn, so it differs every turn; what must never happen is the
// pack rewriting what came before it, or one turn's pack surviving into the
// next turn's history.

fn pack_saying(fact: &str) -> TurnPack {
    let mut pack = pack();
    pack.native.markdown = format!("# Memory\n\n## Learnings\n- {fact}");
    pack
}

async fn request_for(
    history: &[Message],
    pack: TurnPack,
    profile: Option<tinyinference_llm::model::ModelProfile>,
) -> ModelRequest {
    let mut ctx = context(Some(pack));
    ctx.model_profile = profile;
    let mut request = ModelRequest::new(history.to_vec());
    memory_pack_middleware()
        .before_model(&mut ctx, &(), &mut request)
        .await
        .unwrap();
    request
}

/// Messages two requests share from the start: what a prefix cache can reuse.
fn shared_prefix(a: &[Message], b: &[Message]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn two_turns() -> (Vec<Message>, Vec<Message>) {
    let turn_one = vec![
        Message::system("You are helpful."),
        Message::user("How far is Porto from Lisbon?"),
    ];
    let mut turn_two = turn_one.clone();
    turn_two.push(Message::assistant("About 310 km."));
    turn_two.push(Message::user("And by train?"));
    (turn_one, turn_two)
}

#[tokio::test]
async fn a_new_turn_reuses_the_whole_previous_transcript_as_cached_prefix() {
    let (turn_one, turn_two) = two_turns();
    let first = request_for(
        &turn_one,
        pack_saying("The user prefers metric units"),
        None,
    )
    .await;
    let second = request_for(&turn_two, pack_saying("The user travels by rail"), None).await;

    // Turn two's request starts with turn two's transcript, byte for byte.
    assert_eq!(&second.messages[..turn_two.len()], &turn_two[..]);
    // The cache hit covers every message turn one sent before its pack.
    assert_eq!(
        shared_prefix(&first.messages, &second.messages),
        turn_one.len()
    );
    // Only the tail differs, and turn one's pack is nowhere in turn two.
    assert_eq!(second.messages.len(), turn_two.len() + 1);
    let sent: String = second.messages.iter().map(Message::text).collect();
    assert!(
        !sent.contains("metric units"),
        "turn one's pack leaked: {sent}"
    );
    assert!(sent.contains("travels by rail"));
}

#[tokio::test]
async fn on_a_hoisting_model_only_the_previous_tail_message_falls_out_of_the_cache() {
    let hoisting = Some(tinyinference_llm::model::ModelProfile {
        hoists_system_messages: true,
        ..Default::default()
    });
    let (turn_one, turn_two) = two_turns();
    let first = request_for(
        &turn_one,
        pack_saying("The user prefers metric units"),
        hoisting.clone(),
    )
    .await;
    let second = request_for(&turn_two, pack_saying("The user travels by rail"), hoisting).await;

    // No system message is added: a hoisted one would rewrite the head (#6962).
    assert_eq!(first.messages.len(), turn_one.len());
    assert_eq!(second.messages.len(), turn_two.len());
    // The pack rides the tail user turn; everything before the tail is the
    // transcript untouched, so the head stays cached.
    assert_eq!(
        &second.messages[..turn_two.len() - 1],
        &turn_two[..turn_two.len() - 1]
    );
    assert!(second
        .messages
        .last()
        .unwrap()
        .text()
        .contains("travels by rail"));
    // Turn one's tail carried turn one's pack, so the shared prefix stops just
    // before it: one user message, never the whole history.
    assert_eq!(
        shared_prefix(&first.messages, &second.messages),
        turn_one.len() - 1
    );
    let sent: String = second.messages.iter().map(Message::text).collect();
    assert!(
        !sent.contains("metric units"),
        "turn one's pack leaked: {sent}"
    );
}
