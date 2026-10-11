//! Typed transcript rows against the #6872 compatibility corpus.
//!
//! The corpus in `tests/fixtures/session_compat/` was captured from the release
//! that wrote string-envelope rows. Two gates, both reusing its goldens:
//!
//! - **Same rows, new on-disk form.** `<name>.typed.jsonl` is the legacy
//!   capture re-written by the current writer (tool calls, tool results and
//!   images stored as fields). Resuming it must give byte-identical committed
//!   history, frozen prefix, recorded tool list, journal projection and next
//!   provider request as the legacy file's golden: the typed form is purely an
//!   on-disk change.
//! - **Old session, new binary.** A legacy file continued with a native tool
//!   round keeps its bytes, gains typed rows, and a fresh process resumes the
//!   mixed file to exactly what the continuing process held.
//!
//! Regenerate the `.typed` files only when deliberately re-deriving them:
//! `OH_REGEN_SESSION_COMPAT=1 cargo test -p openhuman --lib regenerate_typed_session_compat -- --ignored`.

use serde_json::{json, Value};
use tinyagents_session::transcript::{append_tools_record, read_transcript, write_transcript};
use tinyinference_llm::message::ContentBlock;
use tinyinference_llm::model::ModelResponse;

use super::transcript_compat_tests::{
    build_host, call, fixture, golden, model, response, run_async, snapshot_with, stem_path,
    thread_id, Scenario, SCENARIOS,
};

/// Head fixtures that have a typed re-write (the chain and legacy layouts are
/// multi-file and the compaction record is not a plain row list).
const TYPED_SCENARIOS: &[&str] = &["plain", "native_tools", "image_user", "xml_tools"];

/// Native-image transport support for live fixtures, independent of the
/// selected-model facts supplied to the scripted response model.
struct LiveImageModel(std::sync::Arc<tinyagents_harness::testkit::ScriptedModel>);

#[async_trait::async_trait]
impl tinyinference_llm::model::ChatModel<()> for LiveImageModel {
    fn profile(&self) -> Option<&tinyinference_llm::model::ModelProfile> {
        tinyinference_llm::model::ChatModel::<()>::profile(self.0.as_ref())
    }
    fn supports_input(
        &self,
        modality: tinyinference_llm::model::InputModality,
        mime: &str,
        source: tinyinference_llm::model::InputSource,
    ) -> bool {
        modality == tinyinference_llm::model::InputModality::Image
            && mime == "image/png"
            && source == tinyinference_llm::model::InputSource::Base64
    }
    async fn invoke(
        &self,
        state: &(),
        request: tinyinference_llm::model::ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        tinyinference_llm::model::ChatModel::invoke(self.0.as_ref(), state, request).await
    }
}

fn live_image_model(reply: &str) -> std::sync::Arc<LiveImageModel> {
    use tinyinference_llm::model::{Modalities, ModelProfile};
    std::sync::Arc::new(LiveImageModel(std::sync::Arc::new(
        tinyagents_harness::testkit::ScriptedModel::new(vec![ModelResponse::assistant(reply)])
            .with_profile(ModelProfile {
                tool_calling: true,
                modalities: Modalities {
                    image_in: true,
                    ..Default::default()
                },
                ..Default::default()
            }),
    )))
}

fn build_live_image_host(
    root: &std::path::Path,
    model: std::sync::Arc<LiveImageModel>,
    thread: &str,
) -> crate::agent::OpenHumanSessionHost {
    let mut config = crate::config::Config::default();
    config.workspace_dir = root.join("workspace");
    config.action_dir = root.to_path_buf();
    config.config_path = root.join("config.toml");
    config.modules.enabled = false;
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model_with_config(model, std::sync::Arc::new(config))
        .tools(Vec::new())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .build()
        .expect("configured image session");
    host.set_thread_id(Some(thread));
    host
}

fn scenario(name: &str) -> &'static Scenario {
    SCENARIOS
        .iter()
        .find(|scenario| scenario.name == name)
        .expect("scenario")
}

fn message_lines(raw: &str) -> Vec<Value> {
    raw.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| value.get("role").is_some())
        .collect()
}

#[test]
fn typed_rewrites_of_the_corpus_resume_to_the_legacy_goldens() {
    for name in TYPED_SCENARIOS {
        run_async(async move {
            let got = snapshot_with(scenario(name), ".typed").await;
            assert_eq!(
                got,
                golden(name),
                "{name}: typed rows must resume to the legacy golden"
            );
        });
    }
}

#[test]
fn typed_fixtures_really_use_the_typed_form() {
    let shapes = |name: &str| -> Vec<String> {
        let raw = std::fs::read_to_string(fixture(&format!("{name}.typed.jsonl"))).unwrap();
        message_lines(&raw)
            .iter()
            .filter_map(|line| line.get("shape").and_then(Value::as_str).map(String::from))
            .collect()
    };
    let native = shapes("native_tools");
    assert!(native.iter().any(|shape| shape == "assistant_calls"));
    assert!(native.iter().any(|shape| shape == "tool_result"));
    assert!(shapes("plain").is_empty());
    // The envelope string no longer appears in a typed row's content.
    let raw = std::fs::read_to_string(fixture("native_tools.typed.jsonl")).unwrap();
    for line in message_lines(&raw) {
        if line.get("v").is_some() {
            assert!(
                !line["content"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with('{'),
                "{line}"
            );
        }
    }
}

#[test]
fn an_old_session_continued_by_this_binary_reloads_identically() {
    run_async(async {
        let scenario = scenario("native_tools");
        let root = tempfile::tempdir().expect("tempdir");
        let text = |s: &str| response(vec![ContentBlock::Text(s.into())], Vec::new());
        let thread = thread_id(scenario.name);

        let mut first = build_host(
            root.path(),
            model(
                vec![
                    response(
                        vec![ContentBlock::Text("one more call".into())],
                        vec![call("call_new", "echo", json!({"q": "later"}))],
                    ),
                    text("continued"),
                ],
                true,
            ),
            true,
            &thread,
        );
        let stem = first.session_id().expect("session id");
        let path = stem_path(root.path(), &stem);
        std::fs::copy(fixture("native_tools.jsonl"), &path).expect("place legacy file");
        let legacy_bytes = std::fs::read(&path).expect("legacy bytes");

        assert!(first.resume_bound_session().await.expect("resume"));
        first.turn("keep going").await.expect("continued turn");
        let continued_history = serde_json::to_value(
            first
                .runtime_session
                .as_ref()
                .expect("runtime session")
                .history(),
        )
        .expect("history");
        drop(first);

        // The old rows are byte-for-byte what was captured; new rows follow.
        let after = std::fs::read(&path).expect("file after");
        assert!(
            after.starts_with(&legacy_bytes),
            "existing bytes were rewritten"
        );
        let appended = String::from_utf8(after[legacy_bytes.len()..].to_vec()).unwrap();
        let new_rows = message_lines(&appended);
        let shape = |index: usize| new_rows[index].get("shape").and_then(Value::as_str);
        assert!(
            new_rows.iter().any(|row| row["shape"] == "assistant_calls")
                && new_rows.iter().any(|row| row["shape"] == "tool_result"),
            "continued tool round must be typed: {shape:?}",
            shape = (0..new_rows.len()).map(shape).collect::<Vec<_>>()
        );

        // A fresh process resumes the mixed file to what the first one held.
        let second = {
            let mut host = build_host(
                root.path(),
                model(vec![ModelResponse::assistant("unused")], true),
                true,
                &thread,
            );
            assert!(host.resume_bound_session().await.expect("second resume"));
            host
        };
        let runtime = second.runtime_session.as_ref().expect("runtime session");
        assert_eq!(
            serde_json::to_value(runtime.history()).expect("history"),
            continued_history
        );
        let transcript = read_transcript(&path).expect("read mixed");
        assert!(transcript
            .messages
            .iter()
            .any(|row| row.tool_calls.iter().any(|call| call.id == "call_new")));
    });
}

/// A live turn whose text carries a ready `[IMAGE:..]` marker persists the
/// image as a typed `user_parts` row (not a text marker), a fresh process
/// resumes the same model history, and the legacy `[IMAGE:]` text rows of the
/// corpus (`image_user`) keep resuming to their golden.
#[test]
fn a_live_image_turn_persists_typed_image_parts_and_resumes_identically() {
    run_async(async {
        let root = tempfile::tempdir().expect("tempdir");
        let thread = thread_id("live_image");
        let vision = live_image_model("saw it");
        let mut host = build_live_image_host(root.path(), vision.clone(), &thread);
        let stem = host.session_id().expect("session id");
        let png = "data:image/png;base64,iVBORw0KGgo=";
        host.turn(&format!("look [IMAGE:{png}] please"))
            .await
            .expect("turn");
        let history = serde_json::to_value(
            host.runtime_session
                .as_ref()
                .expect("runtime session")
                .history(),
        )
        .expect("history");
        drop(host);

        let path = stem_path(root.path(), &stem);
        let raw = std::fs::read_to_string(&path).expect("transcript");
        let user_line = message_lines(&raw)
            .into_iter()
            .find(|line| line["shape"] == "user_parts")
            .expect("the image turn is a typed user_parts row");
        assert!(!user_line["content"].as_str().unwrap().contains("[IMAGE:"));
        let image_path = user_line["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["type"] == "image")
            .and_then(|part| part["url"].as_str())
            .expect("durable image path");
        assert!(image_path.starts_with("uploads/"), "{image_path}");
        assert_eq!(
            std::fs::read(root.path().join(image_path)).unwrap(),
            b"\x89PNG\r\n\x1a\n"
        );
        assert!(
            !raw.contains("base64,"),
            "provider bytes must remain ephemeral"
        );
        assert!(!raw.contains("[IMAGE:"), "no text marker is persisted");

        assert!(serde_json::to_string(&vision.0.requests())
            .unwrap()
            .contains(png));

        let mut second = build_live_image_host(root.path(), live_image_model("unused"), &thread);
        assert!(second.resume_bound_session().await.expect("resume"));
        assert_eq!(
            serde_json::to_value(
                second
                    .runtime_session
                    .as_ref()
                    .expect("runtime session")
                    .history()
            )
            .expect("history"),
            history
        );
    });
}

/// What the provider is sent for a live image turn on a vision-capable model:
/// the user message's content blocks of the request the model actually saw.
async fn live_image_request_blocks() -> Vec<ContentBlock> {
    use tinyinference_llm::message::Message;

    let root = tempfile::tempdir().expect("tempdir");
    let vision = live_image_model("saw it");
    let mut host = build_live_image_host(root.path(), vision.clone(), &thread_id("vision"));
    host.turn("look [IMAGE:data:image/png;base64,iVBORw0KGgo=] please")
        .await
        .expect("turn");
    let request = vision
        .0
        .requests()
        .last()
        .expect("request")
        .messages
        .clone();
    request
        .into_iter()
        .rev()
        .find_map(|message| match message {
            Message::User(user) => Some(user.content),
            _ => None,
        })
        .expect("user message in the request")
}

/// A live image turn reaches a vision-capable provider as a real image
/// content block (it used to arrive as the literal private marker text). The
/// vendor providers serialize that block for both OpenAI-compatible
/// (`image_url` part) and Anthropic (`image` source) requests.
#[test]
fn live_image_turn_request_carries_a_real_image_block() {
    run_async(async {
        let blocks = live_image_request_blocks().await;
        assert!(
            blocks.iter().any(|block| matches!(
                block,
                ContentBlock::Image(image)
                    if image.url == "data:image/png;base64,iVBORw0KGgo="
                        && image.mime_type.as_deref() == Some("image/png")
            )),
            "{blocks:?}"
        );
        assert!(
            !blocks.iter().any(|block| matches!(
                block,
                ContentBlock::Text(text) if text.contains("OH_IMAGE")
            )),
            "{blocks:?}"
        );
    });
}

/// The host's own message bridge sees the same model messages whether a row
/// was stored as a string envelope or as typed fields, for every shape
/// (including an inline image, which the live flow only produces on a
/// provider-bound row).
#[test]
fn typed_and_legacy_rows_bridge_to_identical_model_messages() {
    use crate::agent::message_convert::history_to_messages;
    use tinyagents_runtime::transcript::message_to_native_transcript as message_to_native_chat_message;
    use tinyinference_llm::message::{
        AssistantMessage, ContentBlock, ImageRef, Message, ToolMessage, UserMessage,
    };

    let messages = [
        Message::User(UserMessage {
            content: vec![
                ContentBlock::Text("see ".into()),
                ContentBlock::Image(ImageRef {
                    url: "data:image/png;base64,AAAA".into(),
                    mime_type: Some("image/png".into()),
                }),
            ],
        }),
        Message::Assistant(AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text("calling".into())],
            tool_calls: vec![call("c1", "echo", json!({"q": "x"}))],
            usage: None,
            origin: None,
        }),
        Message::Tool(ToolMessage {
            tool_call_id: "c1".into(),
            content: vec![ContentBlock::Text("echo:x".into())],
            trusted_verbatim: false,
            artifact: None,
        }),
    ];
    let rows: Vec<_> = messages
        .iter()
        .filter_map(message_to_native_chat_message)
        .collect();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("typed.jsonl");
    let transcript = read_transcript(&fixture("plain.jsonl")).expect("meta source");
    write_transcript(&path, &rows, &transcript.meta, None).expect("write");
    let raw = std::fs::read_to_string(&path).expect("raw");
    let shapes: Vec<_> = message_lines(&raw)
        .iter()
        .map(|line| line["shape"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(shapes, ["user_parts", "assistant_calls", "tool_result"]);
    let read = read_transcript(&path).expect("read").messages;
    assert_eq!(read.len(), rows.len());
    for (read, written) in read.iter().zip(&rows) {
        assert_eq!(read.content, written.content);
    }
    assert_eq!(history_to_messages(&read), history_to_messages(&rows));
}

#[test]
#[ignore = "re-derives the committed .typed fixtures; run deliberately (OH_REGEN_SESSION_COMPAT=1)"]
fn regenerate_typed_session_compat() {
    if std::env::var("OH_REGEN_SESSION_COMPAT").as_deref() != Ok("1") {
        return;
    }
    for name in TYPED_SCENARIOS {
        let transcript = read_transcript(&fixture(&format!("{name}.jsonl"))).expect("read legacy");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("typed.jsonl");
        write_transcript(&path, &transcript.messages, &transcript.meta, None).expect("write typed");
        // The rows' sibling record: the tool list the session was sent with.
        let legacy = std::fs::read_to_string(fixture(&format!("{name}.jsonl"))).expect("raw");
        let tools = legacy
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .rfind(|value| value.get("kind").and_then(Value::as_str) == Some("tools"))
            .map(|value| value["tools"].clone())
            .expect("legacy fixture records its tools");
        append_tools_record(&path, &tools).expect("tools record");
        std::fs::copy(&path, fixture(&format!("{name}.typed.jsonl"))).expect("store typed");
    }
}
