//! A resumed thread whose agent sets `refresh_prompt_on_resume` is sent the
//! prompt the agent renders now; without it, it keeps the one it recorded.

use super::*;
use std::sync::Arc;

use crate::agent::prompts::{PromptContext, SystemPromptBuilder};
use crate::agent_registry::host_definition::{AgentDefinition, AgentDefinitionRegistry};
use crate::storage::sessions::splice_fixture_tests::memory_locator;
use tinyagents_harness::testkit::ScriptedModel;
use tinyagents_session::transcript::TranscriptLocator;

const THREAD: &str = "thread-prompt-refresh";

/// A thread-bound session over `locator` that renders its prompt with
/// `prompt` and answers from `model`, under the registry's definition as
/// `define` edits it: the definition the embed factory stamps on its
/// sessions.
fn host_with(
    locator: &Arc<dyn TranscriptLocator>,
    model: &Arc<ScriptedModel>,
    root: &std::path::Path,
    prompt: SystemPromptBuilder,
    define: impl FnOnce(&mut AgentDefinition),
) -> crate::agent::OpenHumanSessionHost {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let chat_model: Arc<dyn tinyinference_llm::model::ChatModel<()>> = model.clone();
    let mut host = crate::agent::SessionHostBuilder::new()
        .with_session_history_locator(Arc::clone(locator))
        .model_name("test/session-fixture".to_string())
        .chat_model(chat_model)
        .model_name("test/session-fixture".to_string())
        .tools(Vec::new())
        .prompt_builder(prompt)
        .workspace_dir(root.join("workspace"))
        .action_dir(root.to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .build()
        .expect("session build");
    let mut definition = (*host.resolved_definition().expect("registry definition")).clone();
    define(&mut definition);
    host.definition = Some(Arc::new(definition));
    host.set_thread_id(Some(THREAD));
    host
}

/// As [`host_with`], for an agent whose whole prompt is `prompt`, verbatim
/// (what embed's `bare_prompt` builds), with `refresh_prompt_on_resume` set
/// to `refresh`.
fn host(
    locator: &Arc<dyn TranscriptLocator>,
    model: &Arc<ScriptedModel>,
    root: &std::path::Path,
    prompt: &str,
    refresh: bool,
) -> crate::agent::OpenHumanSessionHost {
    host_with(
        locator,
        model,
        root,
        SystemPromptBuilder::verbatim(prompt.to_string()),
        |definition| {
            definition.system_prompt = PromptSource::Verbatim(prompt.to_string());
            definition.extensions.refresh_prompt_on_resume = refresh;
        },
    )
}

/// As [`host`], with an inline prompt composed with the standard agent
/// sections, none of which depend on a connected integrations snapshot.
fn inline_host(
    locator: &Arc<dyn TranscriptLocator>,
    model: &Arc<ScriptedModel>,
    root: &std::path::Path,
    prompt: &str,
) -> crate::agent::OpenHumanSessionHost {
    host_with(
        locator,
        model,
        root,
        SystemPromptBuilder::for_subagent(prompt.to_string(), false, false),
        |definition| {
            definition.system_prompt = PromptSource::Inline(prompt.to_string());
            definition.extensions.refresh_prompt_on_resume = true;
        },
    )
}

/// A model that answers each turn with the next of `replies`. The native
/// dialect resolves from the model's profile; a profile-less scripted model
/// would be treated as text-only, like no production model is.
fn model(replies: Vec<&str>) -> Arc<ScriptedModel> {
    Arc::new(
        ScriptedModel::replies(replies).with_profile(tinyinference_llm::model::ModelProfile {
            tool_calling: true,
            ..Default::default()
        }),
    )
}

/// The system messages of the model's latest request, in order.
fn sent_prompt(model: &ScriptedModel) -> Vec<String> {
    let requests = model.requests();
    let sent = &requests.last().expect("a turn reached the model").messages;
    sent.iter()
        .filter(|message| matches!(message, Message::System(_)))
        .map(Message::text)
        .collect()
}

/// Whether the model's latest request carries a message of `role` whose text
/// contains `needle`.
fn sent_contains(model: &ScriptedModel, role: fn(&Message) -> bool, needle: &str) -> bool {
    let requests = model.requests();
    let sent = &requests.last().expect("a turn reached the model").messages;
    sent.iter()
        .any(|message| role(message) && message.text().contains(needle))
}

/// The text of the last user message in the model's latest request.
fn last_user_text(model: &ScriptedModel) -> String {
    let requests = model.requests();
    let sent = &requests.last().expect("a turn reached the model").messages;
    sent.iter()
        .rev()
        .find(|message| is_user(message))
        .map(Message::text)
        .unwrap_or_default()
}

fn is_user(message: &Message) -> bool {
    matches!(message, Message::User(_))
}

fn is_assistant(message: &Message) -> bool {
    matches!(message, Message::Assistant(_))
}

#[test]
fn a_resumed_thread_takes_up_the_current_prompt_only_when_its_agent_opts_in() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(resumed_thread_takes_up_the_current_prompt_only_when_opted_in());
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}

async fn resumed_thread_takes_up_the_current_prompt_only_when_opted_in() {
    let root = tempfile::tempdir().expect("tempdir");
    let locator = memory_locator();
    let model = model(vec![
        "first reply",
        "second reply",
        "third reply",
        "fourth reply",
    ]);

    // A new thread is sent the prompt of its first turn.
    let mut first = host(&locator, &model, root.path(), "You are release one.", true);
    assert_eq!(first.turn("first message").await.unwrap(), "first reply");
    let session = first.session.clone().expect("a thread-bound session");
    drop(first);
    assert_eq!(sent_prompt(&model), ["You are release one."]);
    assert!(!last_user_text(&model).contains(PROMPT_UPDATE_NOTE));
    assert_eq!(locator.session_chain(&session).len(), 1);

    // After a restart with an edited prompt, the resumed thread is sent the
    // current prompt alone, with its whole conversation, and the prompt
    // change opens the transcript's next generation.
    let mut second = host(&locator, &model, root.path(), "You are release two.", true);
    assert_eq!(second.turn("second message").await.unwrap(), "second reply");
    drop(second);
    assert_eq!(sent_prompt(&model), ["You are release two."]);
    assert!(sent_contains(&model, is_user, "first message"));
    assert!(sent_contains(&model, is_assistant, "first reply"));
    assert!(sent_contains(&model, is_user, "second message"));
    assert!(
        last_user_text(&model).starts_with(PROMPT_UPDATE_NOTE),
        "the turn the prompt changed on says so"
    );
    assert_eq!(
        locator.session_chain(&session).len(),
        2,
        "a changed prompt seals the generation it replaced"
    );

    // An unchanged prompt is a no-op: same prompt, no new generation.
    let mut third = host(&locator, &model, root.path(), "You are release two.", true);
    assert_eq!(third.turn("third message").await.unwrap(), "third reply");
    drop(third);
    assert_eq!(sent_prompt(&model), ["You are release two."]);
    assert!(sent_contains(&model, is_assistant, "second reply"));
    assert!(!last_user_text(&model).contains(PROMPT_UPDATE_NOTE));
    assert!(
        sent_contains(&model, is_user, PROMPT_UPDATE_NOTE),
        "the note stays where the change happened"
    );
    assert_eq!(locator.session_chain(&session).len(), 2);

    // Without the opt-in the thread keeps the prompt it recorded.
    let mut fourth = host(
        &locator,
        &model,
        root.path(),
        "You are release three.",
        false,
    );
    assert_eq!(fourth.turn("fourth message").await.unwrap(), "fourth reply");
    assert_eq!(sent_prompt(&model), ["You are release two."]);
    assert!(!last_user_text(&model).contains(PROMPT_UPDATE_NOTE));
    assert_eq!(locator.session_chain(&session).len(), 2);
}

#[tokio::test]
async fn resume_refreshes_the_prompt_without_an_integrations_backend() {
    let root = tempfile::tempdir().expect("tempdir");
    let locator = memory_locator();
    let model = model(vec!["first reply", "second reply"]);

    let mut first = inline_host(&locator, &model, root.path(), "You are release one.");
    assert_eq!(first.turn("first message").await.unwrap(), "first reply");
    drop(first);

    // This host has no hydrated integrations snapshot or backend connection.
    // Inline prompts do not render connected integrations, so that absence
    // must not keep the resumed thread on its recorded prompt.
    let mut resumed = inline_host(&locator, &model, root.path(), "You are release two.");
    assert_eq!(
        resumed.turn("second message").await.unwrap(),
        "second reply"
    );
    assert!(
        sent_prompt(&model)
            .iter()
            .any(|message| message.contains("You are release two.")),
        "the second request should carry the changed inline prompt: {:?}",
        sent_prompt(&model)
    );
    assert!(!sent_prompt(&model)
        .iter()
        .any(|message| message.contains("You are release one.")));
}

/// The prelude a turn of `host` is prepared by.
fn prelude_of(mut host: crate::agent::OpenHumanSessionHost) -> OpenHumanTurnPrelude {
    host.ensure_runtime_session().expect("runtime session");
    let state = host
        .runtime_state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.prelude.clone().expect("prelude")
}

/// The prelude of a session rendering `prompt` verbatim, with `refresh` as
/// its definition's `refresh_prompt_on_resume`.
fn prelude(root: &std::path::Path, prompt: &str, refresh: bool) -> OpenHumanTurnPrelude {
    prelude_of(host(
        &memory_locator(),
        &model(Vec::new()),
        root,
        prompt,
        refresh,
    ))
}

/// The prelude of a session whose prompt is composed by `prompt` (the
/// registry definition's own, not verbatim), opted into the refresh.
fn composed_prelude(root: &std::path::Path, prompt: SystemPromptBuilder) -> OpenHumanTurnPrelude {
    prelude_of(host_with(
        &memory_locator(),
        &model(Vec::new()),
        root,
        prompt,
        |definition| {
            definition.system_prompt = PromptSource::Dynamic(());
            definition.extensions.refresh_prompt_on_resume = true;
        },
    ))
}

/// Marks `prelude`'s connected integrations as read this session.
fn hydrate(prelude: &OpenHumanTurnPrelude) {
    let mut mutable = prelude
        .mutable
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    mutable.connected_integrations_initialized = true;
    mutable.connected_integrations_authoritative = true;
}

/// A resumed thread's view: `prefix` restored, `history` decoded.
fn resumed<'a>(prefix: &'a PrefixSnapshot, history: &'a [Message]) -> SessionStateView<'a> {
    SessionStateView {
        history,
        raw_history: &[],
        prefix,
        prefix_is_frozen: !prefix.messages().is_empty(),
        transcript_target: None,
        committed_turns: 1,
        resumed: true,
        identity: None,
    }
}

/// A transcript snapshot whose request boundary is `base`, as the hook seeds
/// it from the history before the turn is prepared.
fn snapshot(base: usize) -> TranscriptSnapshotSink {
    let sink = TranscriptSnapshotSink::default();
    sink.lock().unwrap().request_base_len = base;
    sink
}

fn current() -> PrefixSnapshot {
    PrefixSnapshot::new(vec![Message::system("current")])
}

/// The user's message this turn, as the hook hands it over.
fn request() -> SessionTurnRequest {
    SessionTurnRequest::new(Message::user("hi"))
}

#[tokio::test]
async fn a_resumed_thread_is_offered_the_current_prompt_as_a_refresh() {
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = prelude(root.path(), "current", true);
    let recorded = PrefixSnapshot::new(vec![Message::system("recorded")]);
    let history = [
        Message::system("recorded"),
        Message::user("hi"),
        Message::assistant("hello"),
    ];
    let mut request = request();
    let prepared = prelude
        .prepare_turn(
            false,
            &resumed(&recorded, &history),
            &snapshot(4),
            &mut request,
        )
        .await
        .expect("prepared turn");
    assert_eq!(prepared.prefix, Some(current().refreshing()));
    assert!(prepared.tools.is_some());
    assert_eq!(
        request.input.text(),
        format!("{PROMPT_UPDATE_NOTE}\n\nhi"),
        "the turn says the prompt changed"
    );
}

#[tokio::test]
async fn a_current_prompt_is_offered_without_a_note() {
    // The runtime finds the offer identical to the recorded prompt and keeps
    // the thread as it is; nothing changed, so nothing is said.
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = prelude(root.path(), "current", true);
    let history = [
        Message::system("current"),
        Message::user("earlier"),
        Message::assistant("hello"),
    ];
    let mut request = request();
    let prepared = prelude
        .prepare_turn(
            false,
            &resumed(&current(), &history),
            &snapshot(4),
            &mut request,
        )
        .await
        .expect("prepared turn");
    assert_eq!(prepared.prefix, Some(current().refreshing()));
    assert_eq!(request.input.text(), "hi");
}

#[tokio::test]
async fn without_the_opt_in_a_resumed_thread_keeps_its_recorded_prompt() {
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = prelude(root.path(), "current", false);
    let recorded = PrefixSnapshot::new(vec![Message::system("recorded")]);
    let history = [Message::system("recorded"), Message::user("hi")];
    let sink = snapshot(3);
    let mut request = request();
    let prepared = prelude
        .prepare_turn(false, &resumed(&recorded, &history), &sink, &mut request)
        .await
        .expect("prepared turn");
    assert_eq!(prepared.prefix, None);
    assert_eq!(sink.lock().unwrap().request_base_len, 3, "nothing moved");
    assert_eq!(request.input.text(), "hi");
}

#[tokio::test]
async fn a_new_session_gets_its_prompt_frozen_whatever_the_opt_in() {
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = prelude(root.path(), "current", true);
    let empty = PrefixSnapshot::default();
    let view = SessionStateView {
        resumed: false,
        committed_turns: 0,
        ..resumed(&empty, &[])
    };
    // The hook seeds the boundary with the user message alone; the request
    // opens with the prompt in front of it.
    let sink = snapshot(1);
    let mut request = request();
    let prepared = prelude
        .prepare_turn(true, &view, &sink, &mut request)
        .await
        .expect("prepared turn");
    assert_eq!(prepared.prefix, Some(current()));
    assert_eq!(sink.lock().unwrap().request_base_len, 2);
    assert_eq!(
        request.input.text(),
        "hi",
        "a new thread has no earlier prompt"
    );
}

#[tokio::test]
async fn the_snapshot_boundary_moves_with_a_prompt_of_another_length() {
    // Recorded under a two-message prompt, now rendered as one: the request
    // is one message shorter, and so is the start of this run's rounds.
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = prelude(root.path(), "current", true);
    let recorded = PrefixSnapshot::new(vec![
        Message::system("tier one"),
        Message::system("tier two"),
    ]);
    let history = [
        Message::system("tier one"),
        Message::system("tier two"),
        Message::user("hi"),
        Message::assistant("hello"),
    ];
    let sink = snapshot(history.len() + 1);
    let prepared = prelude
        .prepare_turn(false, &resumed(&recorded, &history), &sink, &mut request())
        .await
        .expect("prepared turn");
    assert_eq!(prepared.prefix, Some(current().refreshing()));
    assert_eq!(sink.lock().unwrap().request_base_len, 4);
}

#[tokio::test]
async fn a_recorded_prompt_of_unknown_extent_is_kept() {
    // A compacted legacy head restores no prefix, and its old instructions
    // are still in the history: a new prompt beside them would leave them.
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = prelude(root.path(), "current", true);
    let unknown = PrefixSnapshot::default();
    let history = [
        Message::system("old instructions"),
        Message::system("summary of earlier turns"),
        Message::user("hi"),
    ];
    let prepared = prelude
        .prepare_turn(
            false,
            &resumed(&unknown, &history),
            &snapshot(4),
            &mut request(),
        )
        .await
        .expect("prepared turn");
    assert_eq!(prepared.prefix, None);
}

#[tokio::test]
async fn a_thread_recorded_without_a_prompt_takes_up_the_current_one() {
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = prelude(root.path(), "current", true);
    let none = PrefixSnapshot::default();
    let history = [Message::user("hi"), Message::assistant("hello")];
    let prepared = prelude
        .prepare_turn(
            false,
            &resumed(&none, &history),
            &snapshot(3),
            &mut request(),
        )
        .await
        .expect("prepared turn");
    assert_eq!(prepared.prefix, Some(current().refreshing()));
}

#[tokio::test]
async fn a_composed_prompt_waits_for_the_integrations_to_be_read() {
    // A composed prompt renders the connected integrations; rendered before
    // they are read it would replace a complete prompt with a bare one.
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = composed_prelude(root.path(), SystemPromptBuilder::verbatim("current".into()));
    let recorded = PrefixSnapshot::new(vec![Message::system("recorded")]);
    let history = [Message::system("recorded"), Message::user("hi")];
    let prepared = prelude
        .prepare_turn(
            false,
            &resumed(&recorded, &history),
            &snapshot(3),
            &mut request(),
        )
        .await
        .expect("prepared turn");
    assert_eq!(prepared.prefix, None);

    hydrate(&prelude);
    let prepared = prelude
        .prepare_turn(
            false,
            &resumed(&recorded, &history),
            &snapshot(3),
            &mut request(),
        )
        .await
        .expect("prepared turn");
    assert_eq!(prepared.prefix, Some(current().refreshing()));
}

#[test]
fn refresh_log_names_the_prefix_in_use() {
    assert_eq!(
        prompt_refresh_log_fields(true),
        ("refreshed", "prompt_changed")
    );
    assert_eq!(prompt_refresh_log_fields(false), ("kept", "unchanged"));
}

fn failing_prompt(_: &PromptContext<'_>) -> anyhow::Result<String> {
    anyhow::bail!("the prompt source is unavailable")
}

#[tokio::test]
async fn a_prompt_that_fails_to_render_keeps_the_recorded_one() {
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = composed_prelude(
        root.path(),
        SystemPromptBuilder::from_dynamic(failing_prompt),
    );
    hydrate(&prelude);
    let recorded = PrefixSnapshot::new(vec![Message::system("recorded")]);
    let history = [Message::system("recorded"), Message::user("hi")];
    let prepared = prelude
        .prepare_turn(
            false,
            &resumed(&recorded, &history),
            &snapshot(3),
            &mut request(),
        )
        .await
        .expect("a resumed turn does not fail on its prompt");
    assert_eq!(prepared.prefix, None);
}

#[tokio::test]
async fn a_catalogue_change_under_a_kept_prompt_carries_no_note() {
    // The prompt is kept (the integrations it renders are not read yet), but
    // a permanent tool was added since the thread was recorded: the catalogue
    // is refreshed on the recorded prompt. The prompt itself was not
    // replaced, so the turn says nothing about it.
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = composed_prelude(root.path(), SystemPromptBuilder::verbatim("current".into()));
    {
        let mut surface = prelude
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let spec = tinytools::ToolSpec {
            name: "A_TOOL".into(),
            description: "d".into(),
            parameters: serde_json::json!({"type": "object"}),
        };
        surface.visible_tool_specs = Arc::new(vec![Arc::new(spec)]);
        surface.permanent_tool_names = ["A_TOOL".to_string()].into();
    }
    let recorded = PrefixSnapshot::new(vec![Message::system("recorded")]);
    let history = [Message::system("recorded"), Message::user("hi")];
    let sink = snapshot(3);
    let mut request = request();
    let prepared = prelude
        .prepare_turn(false, &resumed(&recorded, &history), &sink, &mut request)
        .await
        .expect("prepared turn");
    let prefix = prepared
        .prefix
        .as_ref()
        .expect("the refreshed catalogue is offered");
    assert_eq!(
        prefix.messages()[0].text(),
        "recorded",
        "the prompt is kept"
    );
    let catalogue = prefix.messages().last().expect("a catalogue").text();
    assert!(catalogue.contains("A_TOOL"));
    assert_eq!(sink.lock().unwrap().request_base_len, 4);
    assert_eq!(
        request.input.text(),
        "hi",
        "a catalogue change alone is not a replaced prompt"
    );
}

#[tokio::test]
async fn a_catalogue_change_under_a_rendered_but_identical_prompt_carries_no_note() {
    // The agent opts in and the current prompt renders identical to the
    // recorded one, but a permanent tool was added since: the catalogue
    // refresh changes the prefix. The prompt was not replaced, so no note.
    let root = tempfile::tempdir().expect("tempdir");
    let prelude = prelude(root.path(), "current", true);
    {
        let mut surface = prelude
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let spec = tinytools::ToolSpec {
            name: "A_TOOL".into(),
            description: "d".into(),
            parameters: serde_json::json!({"type": "object"}),
        };
        surface.visible_tool_specs = Arc::new(vec![Arc::new(spec)]);
        surface.permanent_tool_names = ["A_TOOL".to_string()].into();
    }
    let history = [
        Message::system("current"),
        Message::user("earlier"),
        Message::assistant("hello"),
    ];
    let mut request = request();
    let prepared = prelude
        .prepare_turn(
            false,
            &resumed(&current(), &history),
            &snapshot(4),
            &mut request,
        )
        .await
        .expect("prepared turn");
    let prefix = prepared
        .prefix
        .as_ref()
        .expect("the refreshed catalogue is offered");
    assert!(prefix
        .messages()
        .last()
        .expect("a catalogue")
        .text()
        .contains("A_TOOL"));
    assert_eq!(
        request.input.text(),
        "hi",
        "a catalogue change under a kept prompt is not a replaced prompt"
    );
}

#[tokio::test]
async fn a_recorded_catalogue_is_not_read_as_a_replaced_prompt() {
    // The thread recorded a permanent-tool catalogue beside its prompt, and
    // the current prompt renders identical to the recorded one. Whether the
    // catalogue is unchanged or names another tool now, the prompt was not
    // replaced, so the user's message carries no note.
    let spec = |name: &str| {
        Arc::new(tinytools::ToolSpec {
            name: name.into(),
            description: "d".into(),
            parameters: serde_json::json!({"type": "object"}),
        })
    };
    let recorded = tinyagents_runtime::refresh_prefix(
        &current(),
        &[spec("A_TOOL")],
        &["A_TOOL".to_string()].into(),
        super::super::permanent::PERMANENT_TOOLS_MARKER,
        "</openhuman-permanent-tools>",
    )
    .expect("a recorded catalogue");
    for now in ["A_TOOL", "B_TOOL"] {
        let root = tempfile::tempdir().expect("tempdir");
        let prelude = prelude(root.path(), "current", true);
        {
            let mut surface = prelude
                .tool_surface
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            surface.visible_tool_specs = Arc::new(vec![spec(now)]);
            surface.permanent_tool_names = [now.to_string()].into();
        }
        let history = [Message::user("earlier"), Message::assistant("hello")];
        let mut request = request();
        prelude
            .prepare_turn(
                false,
                &resumed(&recorded, &history),
                &snapshot(4),
                &mut request,
            )
            .await
            .expect("prepared turn");
        assert_eq!(
            request.input.text(),
            "hi",
            "catalogue now names {now}: the prompt was not replaced"
        );
    }
}
