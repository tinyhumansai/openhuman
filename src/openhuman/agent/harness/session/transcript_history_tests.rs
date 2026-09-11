//! Tests for the [`ChatHistory`] seam over the durable transcript.
//!
//! These pin the three properties S3 of the design doc calls out as hard
//! requirements, because each one is a place where a plausible-looking
//! implementation would silently corrupt a user's transcript:
//!
//! 1. `messages()` reads the **model-context** replay, not the raw line set.
//! 2. `replace()` compacts rather than rewriting, so history survives.
//! 3. `clear()` empties the context without destroying the file.

use tempfile::TempDir;

use super::*;
use crate::openhuman::agent::harness::session::transcript::read_transcript_display;

/// Stem every test writes under; the file lands at
/// `{workspace}/session_raw/{STEM}.jsonl`.
const STEM: &str = "1760000000_tester";

fn meta() -> TranscriptMeta {
    TranscriptMeta {
        agent_name: "tester".into(),
        agent_id: Some("tester".into()),
        agent_type: Some("root".into()),
        dispatcher: "native".into(),
        provider: None,
        model: None,
        created: "2026-08-07T10:00:00Z".into(),
        updated: "2026-08-07T10:00:00Z".into(),
        turn_count: 0,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some("thread-1".into()),
        task_id: None,
    }
}

fn history(dir: &TempDir) -> SessionTranscriptHistory {
    SessionTranscriptHistory::new(dir.path(), STEM, meta()).unwrap()
}

fn user(text: &str) -> Message {
    Message::User(tinyinference::message::UserMessage {
        content: vec![tinyinference::message::ContentBlock::Text(text.to_string())],
    })
}

/// Visible text of each message, for order-sensitive assertions.
fn texts(messages: &[Message]) -> Vec<String> {
    messages.iter().map(Message::text).collect()
}

#[tokio::test]
async fn messages_on_absent_transcript_is_empty_not_an_error() {
    let dir = TempDir::new().unwrap();
    assert!(history(&dir).messages("thread-1").await.unwrap().is_empty());
}

#[tokio::test]
async fn append_extends_and_reads_back_in_order() {
    let dir = TempDir::new().unwrap();
    let h = history(&dir);

    h.append("thread-1", user("one")).await.unwrap();
    h.append("thread-1", user("two")).await.unwrap();

    let got = h.messages("thread-1").await.unwrap();
    assert_eq!(texts(&got), vec!["one", "two"]);
}

/// S3 requirement 1: `messages()` must be the model-context replay.
///
/// After a reduction, the raw file still holds the pre-compaction lines. A
/// reader that returned the raw line set would hand the model a context that
/// includes text the compaction was meant to drop.
#[tokio::test]
async fn messages_replays_compaction_rather_than_returning_raw_lines() {
    let dir = TempDir::new().unwrap();
    let h = history(&dir);

    h.append("thread-1", user("first")).await.unwrap();
    h.append("thread-1", user("second")).await.unwrap();
    h.append("thread-1", user("third")).await.unwrap();

    // Reduce to a set that is not a prefix extension → compaction record.
    h.replace("thread-1", vec![user("summary")]).await.unwrap();

    // The model-context read sees only the replacement.
    assert_eq!(
        texts(&h.messages("thread-1").await.unwrap()),
        vec!["summary"]
    );

    // ...while the file itself still carries the superseded lines, proving the
    // reduction was a compaction record and not a rewrite.
    let path = h.path();
    let display = read_transcript_display(path).unwrap();
    let rendered = format!("{display:?}");
    assert!(
        rendered.contains("first") && rendered.contains("second"),
        "pre-compaction lines must survive on disk; display read was: {rendered}"
    );

    // And the seam agrees with the format's own model-context reader.
    assert_eq!(
        texts(&h.messages("thread-1").await.unwrap()),
        read_transcript(path)
            .unwrap()
            .messages
            .iter()
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
    );
}

/// S3 requirement 2: `replace()` must not rewrite the file.
///
/// The trait's default `replace` is clear-then-append; if that default were
/// ever inherited here it would destroy the append-only history. This asserts
/// the file only ever grows.
#[tokio::test]
async fn replace_appends_a_compaction_record_and_never_shrinks_the_file() {
    let dir = TempDir::new().unwrap();
    let h = history(&dir);

    h.append("thread-1", user("alpha")).await.unwrap();
    h.append("thread-1", user("beta")).await.unwrap();

    let path = h.path();
    let before = std::fs::read_to_string(path).unwrap();

    h.replace("thread-1", vec![user("condensed")])
        .await
        .unwrap();

    let after = std::fs::read_to_string(path).unwrap();
    assert!(
        after.starts_with(&before),
        "replace must append; earlier bytes were modified"
    );
    assert!(
        after.contains("\"kind\":\"compaction\""),
        "replace must write a compaction record, got: {after}"
    );
}

/// S3 requirement 3: `clear()` semantics are explicit and non-destructive.
#[tokio::test]
async fn clear_empties_the_context_but_preserves_the_file() {
    let dir = TempDir::new().unwrap();
    let h = history(&dir);

    h.append("thread-1", user("kept on disk")).await.unwrap();
    let path = h.path();
    let before = std::fs::read_to_string(path).unwrap();

    h.clear("thread-1").await.unwrap();

    assert!(h.messages("thread-1").await.unwrap().is_empty());
    assert!(path.exists(), "clear must not delete the transcript");

    let after = std::fs::read_to_string(path).unwrap();
    assert!(
        after.starts_with(&before),
        "clear must append, not truncate"
    );
    assert!(
        after.contains("kept on disk"),
        "clear must preserve prior lines for the display read"
    );
}

#[tokio::test]
async fn clear_on_absent_transcript_is_a_noop_not_an_error() {
    let dir = TempDir::new().unwrap();
    history(&dir).clear("thread-1").await.unwrap();
}

/// Appending after a compaction continues from the replacement set, not from
/// the superseded lines — otherwise dropped context would resurrect itself.
#[tokio::test]
async fn append_after_compaction_extends_the_replacement_set() {
    let dir = TempDir::new().unwrap();
    let h = history(&dir);

    h.append("thread-1", user("old")).await.unwrap();
    h.replace("thread-1", vec![user("summary")]).await.unwrap();
    h.append("thread-1", user("new")).await.unwrap();

    assert_eq!(
        texts(&h.messages("thread-1").await.unwrap()),
        vec!["summary", "new"]
    );
}

/// An existing transcript's cumulative `_meta` wins over the handle's seed, so
/// reopening a session does not reset its turn/token rollups to zero.
#[tokio::test]
async fn existing_meta_is_preferred_over_the_seed() {
    let dir = TempDir::new().unwrap();
    history(&dir).append("thread-1", user("one")).await.unwrap();

    let mut stale_seed = meta();
    stale_seed.turn_count = 999;
    stale_seed.agent_name = "wrong".into();
    let reopened = SessionTranscriptHistory::new(dir.path(), STEM, stale_seed).unwrap();
    reopened.append("thread-1", user("two")).await.unwrap();

    let persisted = read_transcript(reopened.path()).unwrap();
    assert_eq!(persisted.meta.agent_name, "tester");
    assert_ne!(persisted.meta.turn_count, 999);
}

// ── S4: the `SessionHistory` write seam ──────────────────────────────

fn chat(role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        id: None,
        role: role.into(),
        content: content.into(),
        extra_metadata: None,
    }
}

/// A turn's worth of provenance, with fixed timestamps so byte comparison is
/// not clock-dependent.
fn turn_usage() -> TurnUsage {
    TurnUsage {
        provider: "anthropic".into(),
        model: "claude-x".into(),
        usage: crate::openhuman::agent::harness::session::transcript::MessageUsage {
            input: 20,
            output: 8,
            cached_input: 0,
            context_window: 200_000,
            cost_usd: 0.002,
        },
        ts: "2026-08-07T10:00:05Z".into(),
        reasoning_content: Some("thinking".into()),
        tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
            id: "call-1".into(),
            name: "get_weather".into(),
            arguments: r#"{"city":"NYC"}"#.into(),
            extra_content: None,
        }],
        iteration: 2,
    }
}

/// The core S4 correctness claim: `append_turn` is a **pure forwarder**.
///
/// The turn path stopped calling `append_transcript_turn` directly and now goes
/// through the handle. That is only safe if the handle changes nothing, so this
/// writes the same turn twice — once each way — and compares the files byte for
/// byte. Cheaper and stricter than re-projecting the result: it fails on any
/// transformation at all, not just ones the projection happens to notice.
#[test]
fn append_turn_is_byte_identical_to_the_free_function() {
    let direct_dir = TempDir::new().unwrap();
    let seam_dir = TempDir::new().unwrap();

    let messages = vec![
        chat("user", "what's the weather?"),
        chat("assistant", "72F and sunny."),
    ];
    let usage = turn_usage();

    let direct_path = resolve_keyed_transcript_path(direct_dir.path(), STEM).unwrap();
    append_transcript_turn(
        &direct_path,
        &[],
        &messages,
        &meta(),
        Some(&usage),
        Some("req-1"),
    )
    .unwrap();

    let seam = SessionTranscriptHistory::new(seam_dir.path(), STEM, meta()).unwrap();
    seam.append_turn(TranscriptTurn {
        prev: &[],
        next: &messages,
        meta: &meta(),
        turn_usage: Some(&usage),
        request_id: Some("req-1"),
    })
    .unwrap();

    assert_eq!(
        std::fs::read(&direct_path).unwrap(),
        std::fs::read(seam.path()).unwrap(),
        "append_turn must forward every argument unchanged"
    );
}

/// `new_in_dir` addresses a profile-scoped raw dir.
///
/// `new` hardcodes `{workspace}/session_raw/`, which is the wrong directory for
/// a dedicated-memory profile (`session_raw-<id>/`). Before this constructor
/// existed, wiring the handle into the turn path would have silently written a
/// profile session into the shared profile's transcripts.
#[test]
fn new_in_dir_writes_into_the_profile_scoped_directory() {
    let dir = TempDir::new().unwrap();
    let profile_dir = dir.path().join("session_raw-1");

    let h = SessionTranscriptHistory::new_in_dir(&profile_dir, STEM, meta()).unwrap();
    h.append_turn(TranscriptTurn {
        prev: &[],
        next: &[chat("user", "profile scoped")],
        meta: &meta(),
        turn_usage: None,
        request_id: None,
    })
    .unwrap();

    assert_eq!(
        h.path(),
        profile_dir.join(format!("{STEM}.jsonl")),
        "handle must be bound to the profile-scoped dir"
    );
    assert!(h.path().exists());
    assert!(
        !dir.path()
            .join("session_raw")
            .join(format!("{STEM}.jsonl"))
            .exists(),
        "nothing may be written into the shared profile's session_raw/"
    );
    assert_eq!(
        read_transcript(h.path()).unwrap().messages[0].content,
        "profile scoped"
    );
}

/// The executable form of this module's "why the write does not cross the crate
/// trait" note: the same logical message set, written through `append_turn`
/// versus through `ChatHistory::replace`, produces display lines that differ in
/// exactly the three fields the trait cannot carry.
///
/// The trait-path assertions are not a bug being pinned — `replace` genuinely
/// has nowhere to put this data. They are here so that anyone tempted to route
/// the turn path through `ChatHistory` sees the cost first.
#[tokio::test]
async fn trait_path_loses_the_provenance_that_append_turn_preserves() {
    let usage = turn_usage();
    let messages = vec![chat("assistant", "72F and sunny.")];

    // Seam path: request_id + turn_usage reach the line.
    let seam_dir = TempDir::new().unwrap();
    let seam = history(&seam_dir);
    seam.append_turn(TranscriptTurn {
        prev: &[],
        next: &messages,
        meta: &meta(),
        turn_usage: Some(&usage),
        request_id: Some("req-1"),
    })
    .unwrap();
    let seam_line = first_display_message(seam.path());
    assert_eq!(seam_line.request_id.as_deref(), Some("req-1"));
    let seam_usage = seam_line.turn_usage.expect("turn_usage persisted");
    assert_eq!(seam_usage.model, "claude-x");
    assert_eq!(seam_usage.iteration, 2);
    assert_eq!(seam_usage.tool_calls.len(), 1);

    // Trait path: the same messages, none of the provenance.
    let trait_dir = TempDir::new().unwrap();
    let trait_history = history(&trait_dir);
    trait_history
        .replace(
            "thread-1",
            vec![Message::Assistant(
                tinyinference::message::AssistantMessage {
                    id: None,
                    content: vec![tinyinference::message::ContentBlock::Text(
                        "72F and sunny.".into(),
                    )],
                    tool_calls: vec![],
                    usage: None,
                },
            )],
        )
        .await
        .unwrap();
    let trait_line = first_display_message(trait_history.path());
    assert!(
        trait_line.request_id.is_none(),
        "ChatHistory has no channel for request_id — no turn boundary"
    );
    assert!(
        trait_line.turn_usage.is_none(),
        "ChatHistory has no channel for turn_usage — no model/iteration/tool calls"
    );
}

/// First `role != "system"` display message line of a transcript.
fn first_display_message(
    path: &Path,
) -> crate::openhuman::agent::harness::session::transcript::DisplayMessage {
    read_transcript_display(path)
        .unwrap()
        .records
        .into_iter()
        .find_map(|r| match r {
            crate::openhuman::agent::harness::session::transcript::DisplayRecord::Message(m)
                if m.message.role != "system" =>
            {
                Some(m)
            }
            _ => None,
        })
        .expect("a display message line")
}

// ─────────────────────────────────────────────────────────────────────
// S4 read half: the locator + `read_session`
// ─────────────────────────────────────────────────────────────────────

/// The `_meta`/`messages` pair rendered exhaustively.
///
/// `SessionTranscript` cannot derive `PartialEq` here — that would mean editing
/// `transcript.rs`, and this branch's zero-on-disk-change rule keeps that file
/// untouched. `ChatMessage`'s `id` and `extra_metadata` are `skip_serializing`,
/// so a JSON comparison would silently ignore exactly the fields most at risk;
/// `Debug` prints every field, so it is the stricter check.
fn transcript_fingerprint(t: &SessionTranscript) -> String {
    format!("{:?}|{:?}", t.meta, t.messages)
}

fn locator(dir: &TempDir) -> FileTranscriptLocator {
    FileTranscriptLocator::new(dir.path(), "session_raw")
}

/// A tool round persisted the way the turn path persists it: the assistant
/// carries the native `{content, tool_calls}` envelope and the tool row carries
/// the matching `tool_call_id`.
fn native_tool_round() -> Vec<ChatMessage> {
    vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("what is the weather"),
        ChatMessage::assistant(
            serde_json::json!({
                "content": "calling get_weather",
                // The flat `{id, name, arguments}` shape
                // `NativeToolDispatcher::to_provider_messages` persists — the
                // one `parse_native_assistant_envelope` accepts.
                "tool_calls": [{
                    "id": "call-1",
                    "name": "get_weather",
                    "arguments": "{\"city\":\"SF\"}"
                }]
            })
            .to_string(),
        ),
        ChatMessage::tool(
            serde_json::json!({"tool_call_id": "call-1", "content": "72F and sunny"}).to_string(),
        ),
        ChatMessage::assistant("It is 72F and sunny."),
    ]
}

/// Writes a transcript exercising every replay rule the read must preserve: a
/// plain extension turn, a **compaction** (a reduction, not a prefix), an
/// `interrupted: true` partial, and a failure-annotated tool row.
fn write_torture_transcript(dir: &TempDir) -> PathBuf {
    let path = resolve_keyed_transcript_path(dir.path(), STEM).unwrap();

    let first = native_tool_round();
    append_transcript_turn(&path, &[], &first, &meta(), None, Some("req-1")).unwrap();

    // A reduction, so the writer must emit a compaction record rather than a
    // tail append. Replaying it wrongly is the corruption §3.1 calls the single
    // most important constraint in the design.
    let mut failed_tool = ChatMessage::tool(
        serde_json::json!({"tool_call_id": "call-2", "content": "boom"}).to_string(),
    );
    crate::openhuman::agent::harness::session::transcript::attach_tool_failure_metadata(
        &mut failed_tool,
        Some("exit status 1"),
    );
    let reduced = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::assistant("[summary] asked about weather"),
        failed_tool,
        ChatMessage::assistant("Sorry, that failed."),
    ];
    append_transcript_turn(&path, &first, &reduced, &meta(), None, Some("req-2")).unwrap();

    // Display-only line the model-context replay must skip.
    crate::openhuman::agent::harness::session::transcript::append_interrupted_partial(
        &path,
        "half a sent",
        Some("req-3"),
        Some(1),
        None,
    )
    .unwrap();

    path
}

/// The locator's read must be the free function's read — same struct, same
/// call, no `Message` round trip. Compaction replay and interrupted-partial
/// skipping therefore come along for free rather than being re-implemented.
#[test]
fn locator_read_is_equivalent_to_the_free_function() {
    let dir = TempDir::new().unwrap();
    let path = write_torture_transcript(&dir);

    let direct = read_transcript(&path).unwrap();
    let through_seam = locator(&dir)
        .latest_for_agent("tester")
        .expect("locator discovers the transcript")
        .read_session()
        .unwrap()
        .expect("file exists");

    assert_eq!(
        transcript_fingerprint(&through_seam),
        transcript_fingerprint(&direct),
        "read_session must return exactly what read_transcript returns"
    );
    // Guard the fixture itself: an equivalence that compared two empty replays
    // would pass for the wrong reason.
    let roles_and_text: Vec<(&str, String)> = direct
        .messages
        .iter()
        .map(|m| {
            // The tool row is JSON, and a serde round trip may reorder its
            // keys; compare it parsed so the assertion pins content, not
            // serialisation order.
            let text = serde_json::from_str::<serde_json::Value>(&m.content)
                .map(|v| v["content"].as_str().unwrap_or_default().to_string())
                .unwrap_or_else(|_| m.content.clone());
            (m.role.as_str(), text)
        })
        .collect();
    assert_eq!(
        roles_and_text,
        vec![
            ("system", "system prompt".to_string()),
            ("assistant", "[summary] asked about weather".to_string()),
            ("tool", "boom".to_string()),
            ("assistant", "Sorry, that failed.".to_string()),
        ],
        "the fixture must actually exercise the compaction + interrupted skip"
    );
}

/// The cold-boot lookup keyed on `_meta.thread_id` goes through the same seam.
#[test]
fn locator_root_for_thread_reads_the_matching_transcript() {
    let dir = TempDir::new().unwrap();
    let path = write_torture_transcript(&dir);

    let through_seam = locator(&dir)
        .root_for_thread("thread-1")
        .expect("locator resolves by _meta.thread_id")
        .read_session()
        .unwrap()
        .expect("file exists");

    assert_eq!(
        transcript_fingerprint(&through_seam),
        transcript_fingerprint(&read_transcript(&path).unwrap())
    );
    assert!(locator(&dir).root_for_thread("thread-absent").is_none());
}

/// `opened_at` binds a discovered path verbatim, so a legacy `.md` transcript
/// still resolves. Re-resolving through `resolve_keyed_transcript_path*` — the
/// obvious-looking alternative — would rewrite the extension to `.jsonl` and
/// hand back a path that does not exist.
#[test]
fn md_legacy_path_resolves_through_the_locator() {
    let dir = TempDir::new().unwrap();
    let raw = dir.path().join("session_raw");
    std::fs::create_dir_all(&raw).unwrap();
    let md = raw.join(format!("{STEM}.md"));
    std::fs::write(
        &md,
        "<!-- session_transcript\nagent: tester\ndispatcher: native\n-->\n\n\
         <!--MSG role=\"user\"-->\nlegacy question\n<!--/MSG-->\n",
    )
    .unwrap();

    let handle = locator(&dir)
        .latest_for_agent("tester")
        .expect("locator finds the legacy .md");
    assert_eq!(
        handle.path(),
        md,
        "the discovered path must be used verbatim"
    );

    let session = handle.read_session().unwrap().expect("legacy file exists");
    assert_eq!(
        session
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec!["legacy question"]
    );
}

/// A read handle bound to a file that vanished is `Ok(None)`, not an error —
/// the callers fold it into their existing "nothing to resume from" branch.
#[test]
fn read_session_on_absent_file_is_none_not_an_error() {
    let dir = TempDir::new().unwrap();
    let handle = SessionTranscriptHistory::opened_at(
        dir.path().join("session_raw").join("nope.jsonl"),
        meta(),
    );
    assert!(handle.read_session().unwrap().is_none());
}

/// **The mutation gate for the read half.**
///
/// Routing the read through `ChatHistory::messages()` and converting back with
/// `message_to_chat_message` flattens the assistant's native `tool_calls`
/// envelope into prose, orphaning the following `role:"tool"` row — the
/// provider `400 An assistant message with 'tool_calls' must be followed by
/// tool messages`. This test asserts both halves: what the seam preserves, and
/// that the rejected route really does lose it. Swap `read_session` for
/// `messages()` in `try_load_session_transcript` and this fails.
#[tokio::test]
async fn resumed_native_tool_round_keeps_tool_calls() {
    let dir = TempDir::new().unwrap();
    let path = resolve_keyed_transcript_path(dir.path(), STEM).unwrap();
    let round = native_tool_round();
    append_transcript_turn(&path, &[], &round, &meta(), None, None).unwrap();

    let through_seam = locator(&dir)
        .latest_for_agent("tester")
        .unwrap()
        .read_session()
        .unwrap()
        .unwrap()
        .messages;

    let assistant = through_seam
        .iter()
        .find(|m| m.role == "assistant" && m.content.contains("tool_calls"))
        .expect("the assistant envelope survived the seam");
    let envelope: serde_json::Value = serde_json::from_str(&assistant.content).unwrap();
    assert_eq!(envelope["tool_calls"][0]["id"], "call-1");
    let tool_row = through_seam
        .iter()
        .find(|m| m.role == "tool")
        .expect("the tool result survived");
    let tool_json: serde_json::Value = serde_json::from_str(&tool_row.content).unwrap();
    assert_eq!(
        tool_json["tool_call_id"], "call-1",
        "the tool result must still correlate to the assistant's call"
    );

    // The rejected route, run for real so the rejection stays evidence-backed.
    let lossy: Vec<ChatMessage> = SessionTranscriptHistory::new(dir.path(), STEM, meta())
        .unwrap()
        .messages("thread-1")
        .await
        .unwrap()
        .iter()
        .map(message_to_chat_message)
        .collect();
    assert!(
        !lossy
            .iter()
            .any(|m| m.role == "assistant" && m.content.contains("tool_calls")),
        "ChatHistory::messages() is expected to drop the tool_calls envelope — \
         if this ever stops being true, revisit the read seam's rationale"
    );
}
