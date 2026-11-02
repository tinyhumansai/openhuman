use super::*;
use std::sync::Arc;

use super::super::prompt_refresh::PROMPT_UPDATE_NOTE;
use crate::agent::prompts::SystemPromptBuilder;
use crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshotSink;
use crate::agent_registry::host_definition::{AgentDefinitionRegistry, MemoryBlock};
use crate::config::Config;
use crate::memory::test_fixtures::{bind_reference, config_in, RefusingEngine};
use crate::storage::sessions::splice_fixture_tests::memory_locator;
use tinyagents_definition::PromptSource;
use tinyagents_runtime::{SessionTurnRequest, TranscriptTarget};
use tinyagents_session::transcript::{
    SessionRef, TranscriptLocator, TranscriptMessage, TranscriptMeta, TranscriptTurn,
};
use tinymemory_api::{LearningKind, MemoryEngine, MemoryMeta, StoreItem};

const PROMPT: &str = "You are blocky.";

/// A session rendering `prompt` verbatim, with a memory block and `refresh`
/// as its `refresh_prompt_on_resume`, reading memory under `config`.
fn host(config: &Config, prompt: &str, refresh: bool) -> OpenHumanSessionHost {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let mut host = crate::agent::SessionHostBuilder::new()
        .with_session_history_locator(memory_locator())
        .model_name("test/session-fixture".to_string())
        .chat_model(Arc::new(
            tinyagents_harness::testkit::ScriptedModel::replies(Vec::<&str>::new()),
        ))
        .tools(Vec::new())
        .prompt_builder(SystemPromptBuilder::verbatim(prompt.to_string()))
        .workspace_dir(config.workspace_dir.clone())
        .action_dir(config.workspace_dir.clone())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .build()
        .expect("session build");
    let mut definition = (*host.resolved_definition().expect("registry definition")).clone();
    definition.system_prompt = PromptSource::Verbatim(prompt.to_string());
    definition.extensions.refresh_prompt_on_resume = refresh;
    definition.extensions.memory_block = Some(MemoryBlock { max_tokens: 500 });
    host.definition = Some(Arc::new(definition));
    host.runtime_config = Some(Arc::new(config.clone()));
    host.set_thread_id(Some("thread-memory-block"));
    host
}

/// The prelude a turn of [`host`] is prepared by.
fn prelude(config: &Config, prompt: &str, refresh: bool) -> OpenHumanTurnPrelude {
    let mut host = host(config, prompt, refresh);
    host.ensure_runtime_session().expect("runtime session");
    let state = host
        .runtime_state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.prelude.clone().expect("prelude")
}

/// Files `text` as a learning every agent under the config's root sees.
async fn learn(engine: &tinymemory_api::conformance::ReferenceEngine, config: &Config, text: &str) {
    let (_, identity) = crate::memory::lifecycle::current_agent_memory(
        config,
        &tinymemory_tools::layout::identity::Identity::root(),
    )
    .expect("memory identity");
    engine
        .store(StoreItem::learning(
            text,
            LearningKind::Fact,
            0.9,
            MemoryMeta {
                namespace: identity.layout.learnings().clone(),
                ..MemoryMeta::default()
            },
        ))
        .await
        .expect("stored");
}

fn block(text: &str) -> Message {
    Message::system(format!("{MEMORY_BLOCK_MARKER}{text}\n</memory>"))
}

fn view<'a>(
    prefix: &'a PrefixSnapshot,
    history: &'a [Message],
    target: Option<&'a TranscriptTarget>,
) -> SessionStateView<'a> {
    SessionStateView {
        history,
        raw_history: &[],
        prefix,
        prefix_is_frozen: !prefix.messages().is_empty(),
        transcript_target: target,
        committed_turns: usize::from(!prefix.messages().is_empty()),
        resumed: !prefix.messages().is_empty(),
        identity: None,
    }
}

async fn prepare(
    prelude: &OpenHumanTurnPrelude,
    new_session: bool,
    view: &SessionStateView<'_>,
) -> (TurnPreparation, SessionTurnRequest) {
    let mut request = SessionTurnRequest::new(Message::user("hi"));
    let snapshot = TranscriptSnapshotSink::default();
    let prepared = prelude
        .prepare_turn(new_session, view, &snapshot, &mut request)
        .await
        .expect("prepared turn");
    (prepared, request)
}

/// The prefix the turn is sent: the offer, else the recorded one.
fn sent(prepared: &TurnPreparation, recorded: &PrefixSnapshot) -> Vec<Message> {
    prepared
        .prefix
        .as_ref()
        .unwrap_or(recorded)
        .messages()
        .to_vec()
}

#[tokio::test]
async fn a_new_thread_ends_its_prompt_with_the_memory_block() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    learn(
        &bind_reference(&config),
        &config,
        "The user's favourite colour is teal",
    )
    .await;
    let prelude = prelude(&config, PROMPT, false);
    let empty = PrefixSnapshot::default();

    let (prepared, request) = prepare(&prelude, true, &view(&empty, &[], None)).await;
    let messages = sent(&prepared, &empty);
    let last = messages.last().expect("a prompt");
    assert!(is_memory_block(last), "{messages:?}");
    assert!(last.text().contains("teal"), "{}", last.text());
    assert_eq!(messages.first().map(Message::text).as_deref(), Some(PROMPT));
    assert_eq!(request.input.text(), "hi");
}

#[tokio::test]
async fn a_new_thread_with_nothing_remembered_has_no_block() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let prelude = prelude(&config, PROMPT, false);
    let empty = PrefixSnapshot::default();

    let (prepared, _) = prepare(&prelude, true, &view(&empty, &[], None)).await;
    let messages = sent(&prepared, &empty);
    assert!(!messages.iter().any(is_memory_block), "{messages:?}");
    assert_eq!(messages.first().map(Message::text).as_deref(), Some(PROMPT));
}

#[tokio::test]
async fn a_memory_read_error_leaves_a_new_thread_without_a_block() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    RefusingEngine::out_of_credits().bind(&config);
    let prelude = prelude(&config, PROMPT, false);
    let empty = PrefixSnapshot::default();

    let (prepared, _) = prepare(&prelude, true, &view(&empty, &[], None)).await;
    let messages = sent(&prepared, &empty);
    assert!(!messages.iter().any(is_memory_block), "{messages:?}");
    assert_eq!(messages.first().map(Message::text).as_deref(), Some(PROMPT));
}

#[tokio::test]
async fn a_resumed_thread_keeps_its_recorded_block_when_memory_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    learn(
        &bind_reference(&config),
        &config,
        "The user lives in Lisbon",
    )
    .await;
    // Refreshing the prompt renders it again on every turn: the block must
    // still be the recorded one, and the unchanged prompt adds no note.
    let prelude = prelude(&config, PROMPT, true);
    let recorded = PrefixSnapshot::new(vec![Message::system(PROMPT), block("- teal")]);
    let history = [
        Message::system(PROMPT),
        block("- teal"),
        Message::user("hello"),
        Message::assistant("hi"),
    ];

    let (prepared, request) = prepare(&prelude, false, &view(&recorded, &history, None)).await;
    assert_eq!(sent(&prepared, &recorded), recorded.messages());
    assert_eq!(request.input.text(), "hi", "no {PROMPT_UPDATE_NOTE}");
}

#[tokio::test]
async fn a_resumed_thread_recorded_without_a_block_takes_one_up() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    learn(
        &bind_reference(&config),
        &config,
        "The user lives in Lisbon",
    )
    .await;
    let prelude = prelude(&config, PROMPT, false);
    let recorded = PrefixSnapshot::new(vec![Message::system(PROMPT)]);
    let history = [Message::system(PROMPT), Message::user("hello")];

    let (prepared, request) = prepare(&prelude, false, &view(&recorded, &history, None)).await;
    let offered = prepared.prefix.expect("a refreshed prefix");
    assert!(offered.messages()[1].text().contains("Lisbon"));
    assert_eq!(offered.messages().len(), 2);
    assert_eq!(offered.messages()[0].text(), PROMPT);
    assert_eq!(request.input.text(), "hi");
}

#[tokio::test]
async fn a_replaced_prompt_takes_a_fresh_block_and_still_carries_its_note() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    learn(
        &bind_reference(&config),
        &config,
        "The user lives in Lisbon",
    )
    .await;
    let prelude = prelude(&config, "You are blocky, v2.", true);
    let recorded = PrefixSnapshot::new(vec![Message::system(PROMPT), block("- teal")]);
    let history = [
        Message::system(PROMPT),
        block("- teal"),
        Message::user("hello"),
    ];

    let (prepared, request) = prepare(&prelude, false, &view(&recorded, &history, None)).await;
    let offered = prepared.prefix.expect("a refreshed prefix");
    assert_eq!(offered.messages()[0].text(), "You are blocky, v2.");
    let last = offered.messages().last().expect("a block");
    assert!(
        is_memory_block(last) && last.text().contains("Lisbon"),
        "{offered:?}"
    );
    assert!(request.input.text().starts_with(PROMPT_UPDATE_NOTE));
}

/// A session-bound target over `locator` whose head is generation 1, the
/// successor of a generation 0 recorded with `parent_block`, carrying
/// `head_block` and `head_turns` committed turns.
fn generations(
    locator: &Arc<dyn TranscriptLocator>,
    parent_block: &Message,
    head_block: &Message,
    head_turns: usize,
) -> TranscriptTarget {
    let meta = |turns: usize| TranscriptMeta {
        session_id: None,
        parent_session_id: None,
        agent_name: "blocky".into(),
        agent_id: Some("blocky".into()),
        agent_type: Some("root".into()),
        dispatcher: "test".into(),
        provider: None,
        model: None,
        created: chrono::Utc::now().to_rfc3339(),
        updated: chrono::Utc::now().to_rfc3339(),
        turn_count: turns,
        prefix_message_count: Some(2),
        prefix_reset: false,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some("thread-memory-block".into()),
        task_id: None,
    };
    let rows = |block: &Message| {
        vec![
            TranscriptMessage::system(PROMPT),
            TranscriptMessage::system(block.text()),
            TranscriptMessage::user("hello"),
        ]
    };
    let root = SessionRef::scoped("thread-memory-block", "blocky");
    let parent = rows(parent_block);
    locator
        .open_session(&root, meta(3))
        .expect("root")
        .append_turn(TranscriptTurn {
            prev: &[],
            next: &parent,
            meta: &meta(3),
            turn_usage: None,
            request_id: None,
            tools: None,
        })
        .expect("root written");
    let (head, handle) = locator
        .begin_generation_from_baseline(&root, meta(head_turns), &parent)
        .expect("successor");
    handle
        .append_turn(TranscriptTurn {
            prev: &[],
            next: &rows(head_block),
            meta: &meta(head_turns),
            turn_usage: None,
            request_id: None,
            tools: None,
        })
        .expect("head written");
    TranscriptTarget::for_session(Arc::clone(locator), head, meta(head_turns))
}

#[tokio::test]
async fn a_generation_that_inherited_its_block_renders_a_fresh_one() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    learn(
        &bind_reference(&config),
        &config,
        "The user lives in Lisbon",
    )
    .await;
    let prelude = prelude(&config, PROMPT, false);
    let locator = memory_locator();
    let old = block("- teal");
    let target = generations(&locator, &old, &old, 1);
    let recorded = PrefixSnapshot::new(vec![Message::system(PROMPT), old.clone()]);
    let history = [Message::system(PROMPT), old, Message::user("hello")];

    let (prepared, request) =
        prepare(&prelude, false, &view(&recorded, &history, Some(&target))).await;
    let offered = prepared.prefix.expect("a refreshed prefix");
    let last = offered.messages().last().expect("a block");
    assert!(last.text().contains("Lisbon"), "{offered:?}");
    assert_eq!(request.input.text(), "hi", "a memory refresh says nothing");
}

#[tokio::test]
async fn a_generation_opened_by_a_block_refresh_keeps_its_block() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    learn(
        &bind_reference(&config),
        &config,
        "The user lives in Lisbon",
    )
    .await;
    let prelude = prelude(&config, PROMPT, false);
    let locator = memory_locator();
    let fresh = block("- teal");
    let target = generations(&locator, &block("- older"), &fresh, 1);
    let recorded = PrefixSnapshot::new(vec![Message::system(PROMPT), fresh.clone()]);
    let history = [Message::system(PROMPT), fresh, Message::user("hello")];

    let (prepared, _) = prepare(&prelude, false, &view(&recorded, &history, Some(&target))).await;
    assert_eq!(sent(&prepared, &recorded), recorded.messages());
}

#[tokio::test]
async fn an_inherited_block_is_kept_once_the_generation_has_moved_on() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    learn(
        &bind_reference(&config),
        &config,
        "The user lives in Lisbon",
    )
    .await;
    let prelude = prelude(&config, PROMPT, false);
    let locator = memory_locator();
    let old = block("- teal");
    let target = generations(&locator, &old, &old, 2);
    let recorded = PrefixSnapshot::new(vec![Message::system(PROMPT), old.clone()]);
    let history = [Message::system(PROMPT), old, Message::user("hello")];

    let (prepared, _) = prepare(&prelude, false, &view(&recorded, &history, Some(&target))).await;
    assert_eq!(sent(&prepared, &recorded), recorded.messages());
}

#[test]
fn the_block_goes_after_every_other_prefix_message() {
    let prefix = PrefixSnapshot::new(vec![
        Message::system(PROMPT),
        block("- old"),
        Message::system("<openhuman-permanent-tools>\ncatalogue"),
    ]);
    let placed = with_memory_block(&prefix, Some(block("- new")));
    let texts: Vec<String> = placed.messages().iter().map(Message::text).collect();
    assert_eq!(
        texts,
        vec![
            PROMPT.to_string(),
            "<openhuman-permanent-tools>\ncatalogue".to_string(),
            block("- new").text(),
        ]
    );
    assert_eq!(with_memory_block(&prefix, None).messages().len(), 2);
}

#[tokio::test]
async fn a_generation_opened_outside_a_turn_can_take_a_fresh_block() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    learn(
        &bind_reference(&config),
        &config,
        "The user lives in Lisbon",
    )
    .await;
    let block = host(&config, PROMPT, false)
        .memory_block_for_new_generation()
        .await
        .expect("a block");
    assert!(is_memory_block(&block) && block.text().contains("Lisbon"));

    let mut without = host(&config, PROMPT, false);
    let mut definition = (**without.definition.as_ref().expect("a definition")).clone();
    definition.extensions.memory_block = None;
    without.definition = Some(Arc::new(definition));
    assert_eq!(without.memory_block_for_new_generation().await, None);
}
