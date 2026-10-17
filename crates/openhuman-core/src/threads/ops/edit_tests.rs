//! Transcript-level tests for `threads::ops::edit`'s private truncation
//! helpers (`truncate_transcript_before_turn`, `truncate_transcript_for_regenerate`).
//!
//! Every fixture is written through the transcript port the session host
//! writes through (`TranscriptStore::locator` -> `open_session` +
//! `append_turn`), never through a file path, so the helpers are exercised
//! against exactly the store and key a real turn lands in. See
//! `edit_turn_state_tests.rs` for `clear_dropped_turn_states` /
//! `next_reply_request_id_after`, which need different fixtures (turn-state
//! snapshots, the message-log store).

use super::*;
use std::path::Path;
use tempfile::TempDir;
use tinyagents_session::transcript::{MessageUsage, TranscriptTurn, TurnUsage};

const AGENT_ID: &str = "test-agent";
/// The store key a desktop turn of the default chat agent is written under.
const STORE_KEY: &str = "orchestrator";

fn store(workspace: &Path) -> TranscriptStore {
    TranscriptStore {
        workspace_dir: workspace.to_path_buf(),
        key: STORE_KEY.to_string(),
    }
}

fn base_meta(agent_id: &str, thread_id: &str) -> TranscriptMeta {
    TranscriptMeta {
        agent_name: agent_id.to_string(),
        agent_id: Some(agent_id.to_string()),
        agent_type: Some("test".into()),
        dispatcher: "native".into(),
        provider: Some("openhuman".into()),
        model: Some("test-model".into()),
        created: "2026-09-24T00:00:00Z".into(),
        updated: "2026-09-24T00:00:00Z".into(),
        turn_count: 0,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some(thread_id.to_string()),
        task_id: None,
        session_id: None,
        parent_session_id: None,
    }
}

fn turn_usage() -> TurnUsage {
    TurnUsage {
        provider: "openhuman".into(),
        model: "test-model".into(),
        usage: MessageUsage {
            input: 10,
            output: 5,
            cached_input: 0,
            context_window: 1_000_000,
            cost_usd: 0.0,
            ..Default::default()
        },
        ts: "2026-09-24T00:00:01Z".into(),
        reasoning_content: None,
        tool_calls: Vec::new(),
        iteration: 1,
    }
}

/// Write a two-turn root transcript for `thread_id`/`AGENT_ID` into `store`
/// the way the session host does, and return the two turns' `request_id`s.
fn write_two_turn_transcript(store: &TranscriptStore, thread_id: &str) -> (String, String) {
    let session = SessionRef::scoped(thread_id, AGENT_ID);
    let meta = base_meta(AGENT_ID, thread_id);
    let history = store
        .locator()
        .open_session(&session, meta.clone())
        .expect("open fixture session");
    let req1 = "turn-1".to_string();
    let req2 = "turn-2".to_string();

    let turn1 = vec![
        TranscriptMessage::new("user", "user prompt 1"),
        TranscriptMessage::assistant("answer 1"),
    ];
    let mut turn1_meta = meta.clone();
    turn1_meta.turn_count = 1;
    history
        .append_turn(TranscriptTurn {
            prev: &[],
            next: &turn1,
            meta: &turn1_meta,
            turn_usage: Some(&turn_usage()),
            request_id: Some(&req1),
            tools: None,
        })
        .expect("append turn 1");

    let mut turn2 = turn1.clone();
    turn2.push(TranscriptMessage::new("user", "user prompt 2"));
    turn2.push(TranscriptMessage::assistant("answer 2"));
    let mut turn2_meta = meta;
    turn2_meta.turn_count = 2;
    history
        .append_turn(TranscriptTurn {
            prev: &turn1,
            next: &turn2,
            meta: &turn2_meta,
            turn_usage: Some(&turn_usage()),
            request_id: Some(&req2),
            tools: None,
        })
        .expect("append turn 2");

    (req1, req2)
}

/// A root transcript with a single system row and no user/assistant turn at
/// all — feeds the "nothing to regenerate" `Ok(None)` case.
fn write_turnless_transcript(store: &TranscriptStore, thread_id: &str) {
    let session = SessionRef::scoped(thread_id, AGENT_ID);
    let meta = base_meta(AGENT_ID, thread_id);
    let messages = vec![TranscriptMessage::new("system", "boot preamble")];
    store
        .locator()
        .open_session(&session, meta.clone())
        .expect("open fixture session")
        .append_turn(TranscriptTurn {
            prev: &[],
            next: &messages,
            meta: &meta,
            turn_usage: None,
            request_id: None,
            tools: None,
        })
        .expect("append system row");
}

/// The first (sealed) generation of the thread's session, read back through
/// the store: a fork never erases it.
fn sealed_generation(store: &TranscriptStore, thread_id: &str) -> Vec<TranscriptMessage> {
    store
        .locator()
        .read_session_transcript(&SessionRef::scoped(thread_id, AGENT_ID))
        .expect("sealed generation stays in the store")
        .read_session()
        .expect("read sealed")
        .expect("sealed transcript exists")
        .messages
}

#[test]
fn truncate_transcript_before_turn_cuts_head_and_keeps_the_sealed_generation() {
    let dir = TempDir::new().expect("tempdir");
    let store = store(dir.path());
    let thread_id = "thread-before-turn";
    let (req1, req2) = write_two_turn_transcript(&store, thread_id);

    truncate_transcript_before_turn(&store, thread_id, &req2).expect("truncate");

    // Head truncated: the new head ends right before turn 2's first row.
    let (head_ref, _, head) = resolve_head_transcript(&store, thread_id).expect("resolve head");
    assert_eq!(
        head_ref.generation, 1,
        "the fork opened the next generation"
    );
    assert!(
        head.messages
            .iter()
            .all(|m| m.request_id.as_deref() != Some(req2.as_str())),
        "turn 2 must be entirely dropped from the head: {:?}",
        head.messages
    );
    let last = head.messages.last().expect("head keeps turn 1");
    assert_eq!(last.request_id.as_deref(), Some(req1.as_str()));
    assert_eq!(last.content, "answer 1", "turn 1's answer is kept intact");

    // "A compaction never erases" applies identically to this edit-driven
    // fork: the sealed generation still holds both turns.
    assert!(
        sealed_generation(&store, thread_id)
            .iter()
            .any(|m| m.request_id.as_deref() == Some(req2.as_str())),
        "the sealed generation keeps the dropped turn"
    );
}

#[test]
fn truncate_transcript_before_turn_errs_when_turn_not_found() {
    let dir = TempDir::new().expect("tempdir");
    let store = store(dir.path());
    let thread_id = "thread-missing-turn";
    write_two_turn_transcript(&store, thread_id);

    let err = truncate_transcript_before_turn(&store, thread_id, "no-such-turn")
        .expect_err("unknown request_id must fail");
    assert!(
        err.contains("no-such-turn"),
        "error should name the missing turn: {err}"
    );
}

#[test]
fn truncate_transcript_before_turn_errs_when_thread_has_no_transcript() {
    let dir = TempDir::new().expect("tempdir");
    let err = truncate_transcript_before_turn(&store(dir.path()), "thread-none", "turn-1")
        .expect_err("a thread with no transcript has nothing to cut");
    assert!(err.contains("has no session transcript"), "{err}");
}

#[test]
fn truncate_transcript_for_regenerate_targets_specific_turn() {
    let dir = TempDir::new().expect("tempdir");
    let store = store(dir.path());
    let thread_id = "thread-regen-target";
    let (_req1, req2) = write_two_turn_transcript(&store, thread_id);

    let (prompt, request_id) = truncate_transcript_for_regenerate(&store, thread_id, Some(&req2))
        .expect("truncate")
        .expect("a turn to regenerate");
    assert_eq!(prompt, "user prompt 2");
    assert_eq!(request_id, req2);

    let (_, _, head) = resolve_head_transcript(&store, thread_id).expect("resolve head");
    let last = head.messages.last().expect("head keeps turn 2's prompt");
    assert_eq!(last.role, "user");
    assert_eq!(last.content, "user prompt 2");
    assert!(
        head.messages.iter().all(|m| m.content != "answer 2"),
        "turn 2's answer must be dropped: {:?}",
        head.messages
    );
    assert!(
        sealed_generation(&store, thread_id)
            .iter()
            .any(|m| m.content == "answer 2"),
        "the sealed generation keeps the dropped answer"
    );
}

#[test]
fn truncate_transcript_for_regenerate_none_resolves_last_turn() {
    let dir = TempDir::new().expect("tempdir");
    let store = store(dir.path());
    let thread_id = "thread-regen-last";
    let (_req1, req2) = write_two_turn_transcript(&store, thread_id);

    let (prompt, request_id) = truncate_transcript_for_regenerate(&store, thread_id, None)
        .expect("truncate")
        .expect("a turn to regenerate");

    assert_eq!(prompt, "user prompt 2", "last turn's prompt is resent");
    assert_eq!(request_id, req2, "last turn's request_id is resolved");
}

#[test]
fn truncate_transcript_for_regenerate_returns_none_when_no_turn_exists() {
    let dir = TempDir::new().expect("tempdir");
    let store = store(dir.path());
    let thread_id = "thread-no-turns";
    write_turnless_transcript(&store, thread_id);

    let result = truncate_transcript_for_regenerate(&store, thread_id, None).expect("truncate");
    assert_eq!(result, None, "no user/assistant turn to regenerate");
}

/// Sentry TAURI-REACT-AK: regenerating on a thread that never wrote a session
/// transcript surfaced "thread <id> has no session transcript" instead of the
/// friendly "has no turn to regenerate" the caller maps `Ok(None)` to.
#[test]
fn truncate_transcript_for_regenerate_returns_none_when_thread_has_no_transcript() {
    let dir = TempDir::new().expect("tempdir");

    let result =
        truncate_transcript_for_regenerate(&store(dir.path()), "thread-without-transcript", None)
            .expect("a thread with no transcript has no turn to regenerate, not an error");
    assert_eq!(result, None);
}

/// A SaaS profile's turns are written under `<profile>~<agent>`. The edit
/// path resolves its key from the request's tenant (not the config alone),
/// so it finds and forks that same transcript, and the unprefixed key of the
/// same workspace never sees it.
#[test]
fn a_profile_prefixed_key_round_trips_through_edit_and_regenerate() {
    let dir = TempDir::new().expect("tempdir");
    let config = crate::config::Config {
        workspace_dir: dir.path().to_path_buf(),
        ..crate::config::Config::default()
    };
    let tenant = crate::core::runtime::Tenant {
        profile: Some("profile-a".to_string()),
        agent: None,
    };

    // What the session host writes under: the ambient-tenant resolution, fed
    // the same tenant the request carries.
    let writer_key = crate::agent::session_store::agent_key_for(
        &tenant,
        &crate::web_chat::pick_target_agent_id(&config),
    );
    let store = super::super::support::transcript_store_for(&config, &tenant);
    assert_eq!(
        store.key, writer_key,
        "the reader resolves the writer's key"
    );
    assert_eq!(
        store.key, "profile-a~default",
        "the key keeps the profile prefix"
    );
    assert_eq!(store.workspace_dir, dir.path());

    let thread_id = "thread-profile";
    let (req1, req2) = write_two_turn_transcript(&store, thread_id);

    let unprefixed = TranscriptStore {
        workspace_dir: dir.path().to_path_buf(),
        key: crate::web_chat::pick_target_agent_id(&config),
    };
    assert_eq!(
        truncate_transcript_for_regenerate(&unprefixed, thread_id, None).expect("lookup"),
        None,
        "the config-only key does not see the profile's transcript"
    );

    truncate_transcript_before_turn(&store, thread_id, &req2).expect("edit forks the transcript");
    let (_, _, head) = resolve_head_transcript(&store, thread_id).expect("resolve head");
    assert_eq!(
        head.messages.last().and_then(|m| m.request_id.as_deref()),
        Some(req1.as_str()),
        "the edit cut the profile's own transcript"
    );

    let (prompt, request_id) = truncate_transcript_for_regenerate(&store, thread_id, None)
        .expect("regenerate")
        .expect("turn 1 is left to regenerate");
    assert_eq!(prompt, "user prompt 1");
    assert_eq!(request_id, req1);
}

#[test]
fn transcript_store_for_a_desktop_tenant_uses_the_chat_agent_and_an_embedded_agent_its_own() {
    let dir = TempDir::new().expect("tempdir");
    let config = crate::config::Config {
        workspace_dir: dir.path().to_path_buf(),
        ..crate::config::Config::default()
    };

    let desktop = super::super::support::transcript_store_for(
        &config,
        &crate::core::runtime::Tenant::default(),
    );
    assert_eq!(desktop.key, crate::web_chat::pick_target_agent_id(&config));

    let embedded = super::super::support::transcript_store_for(
        &config,
        &crate::core::runtime::Tenant {
            profile: None,
            agent: Some("agent-7".to_string()),
        },
    );
    assert_eq!(embedded.key, "agent-7");
}

/// The handler-level resolution: the key comes from the request's own
/// `DispatchCtx` (its profile), with no ambient context in scope.
#[tokio::test]
async fn transcript_store_reads_the_profile_from_the_request_context() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    let dir = TempDir::new().expect("tempdir");
    let config = crate::config::Config {
        workspace_dir: dir.path().to_path_buf(),
        ..crate::config::Config::default()
    };
    let overlay = ContextOverlay::new(
        config,
        DomainSet::kernel(),
        crate::tools::toolpacks::ToolGroups::none(),
    )
    .profile("profile-b");
    let ctx = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None).derive_with(overlay));

    let store = transcript_store(&ctx, "any-thread")
        .await
        .expect("resolve the store");
    assert_eq!(store.key, "profile-b~default");
    assert_eq!(store.workspace_dir, dir.path());
}

/// Edit and regenerate read the store the thread's first session recorded, so
/// they still find the earlier turns after `[agent] chat_agent_id` changed.
#[tokio::test]
async fn edit_and_regenerate_use_the_threads_recorded_key_after_a_chat_agent_change() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    let _ = crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins();
    let dir = TempDir::new().expect("tempdir");
    let ctx_with = |chat_agent: Option<&str>| {
        let mut config = crate::config::Config {
            workspace_dir: dir.path().to_path_buf(),
            ..crate::config::Config::default()
        };
        config.agent.chat_agent_id = chat_agent.map(str::to_string);
        let overlay = ContextOverlay::new(
            config,
            DomainSet::kernel(),
            crate::tools::toolpacks::ToolGroups::none(),
        );
        DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None).derive_with(overlay))
    };
    let (before, after) = (ctx_with(None), ctx_with(Some("planner")));
    let thread_id = "thread-flipped";

    // The first session records its key and writes the transcript there.
    let written = transcript_store(&before, thread_id).await.expect("store");
    let recorded = crate::threads::store::bind_transcript_key(
        &before,
        dir.path(),
        thread_id,
        written.key.clone(),
    )
    .await;
    assert_eq!(recorded, written.key);
    let (req1, req2) = write_two_turn_transcript(&written, thread_id);

    // After the flip the current rule names another store, the record wins.
    let flipped = transcript_store(&after, thread_id).await.expect("store");
    assert_eq!(flipped.key, written.key, "the recorded key is used");
    assert_ne!(
        transcript_store(&after, "unrecorded-thread")
            .await
            .expect("store")
            .key,
        written.key,
        "the chat agent change does move the derived key"
    );

    truncate_transcript_before_turn(&flipped, thread_id, &req2).expect("edit forks the transcript");
    let (_, _, head) = resolve_head_transcript(&flipped, thread_id).expect("resolve head");
    assert_eq!(
        head.messages.last().and_then(|m| m.request_id.as_deref()),
        Some(req1.as_str())
    );
    let (prompt, request_id) = truncate_transcript_for_regenerate(&flipped, thread_id, None)
        .expect("regenerate")
        .expect("turn 1 is left to regenerate");
    assert_eq!(prompt, "user prompt 1");
    assert_eq!(request_id, req1);
}
