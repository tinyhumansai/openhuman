//! `threads.transcript_get` and `threads.token_usage` read a thread through
//! the session store, under the key the request's own context resolves.

use super::*;
use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
use tinyagents_session::transcript::{
    MessageUsage, SessionRef, TranscriptMessage, TranscriptMeta, TranscriptTurn, TurnUsage,
};

const THREAD: &str = "thread-store";

fn ctx_for(workspace: &std::path::Path, profile: Option<&str>) -> DispatchCtx {
    ctx_with_chat_agent(workspace, profile, None)
}

/// A context whose config selects `chat_agent_id` as the web-chat agent.
fn ctx_with_chat_agent(
    workspace: &std::path::Path,
    profile: Option<&str>,
    chat_agent_id: Option<&str>,
) -> DispatchCtx {
    let mut config = crate::config::Config {
        workspace_dir: workspace.to_path_buf(),
        ..crate::config::Config::default()
    };
    config.agent.chat_agent_id = chat_agent_id.map(str::to_string);
    let mut overlay = ContextOverlay::new(
        config,
        DomainSet::kernel(),
        crate::tools::toolpacks::ToolGroups::none(),
    );
    if let Some(profile) = profile {
        overlay = overlay.profile(profile);
    }
    DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None).derive_with(overlay))
}

fn meta(agent: &str, agent_type: &str) -> TranscriptMeta {
    TranscriptMeta {
        agent_name: agent.into(),
        agent_id: Some(agent.into()),
        agent_type: Some(agent_type.into()),
        dispatcher: "native".into(),
        provider: Some("openhuman".into()),
        model: Some("test-model".into()),
        created: "2026-10-15T00:00:00Z".into(),
        updated: "2026-10-15T00:00:00Z".into(),
        turn_count: 1,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some(THREAD.into()),
        task_id: None,
        session_id: None,
        parent_session_id: None,
    }
}

fn usage(input: u64, output: u64) -> TurnUsage {
    TurnUsage {
        provider: "openhuman".into(),
        model: "test-model".into(),
        usage: MessageUsage {
            input,
            output,
            context_window: 200_000,
            ..Default::default()
        },
        ts: "2026-10-15T00:00:01Z".into(),
        reasoning_content: None,
        tool_calls: Vec::new(),
        iteration: 1,
    }
}

/// A root turn and one sub-agent run of [`THREAD`], written where the session
/// host and the sub-agent runner write them: `ctx`'s transcript store.
async fn write_thread(ctx: &DispatchCtx) {
    let locator = super::super::support::transcript_store(ctx, THREAD)
        .await
        .expect("store")
        .locator();
    let root_meta = meta("orchestrator", "root");
    let turn = vec![
        TranscriptMessage::user("hello"),
        TranscriptMessage::assistant("hi there"),
    ];
    locator
        .open_session(
            &SessionRef::scoped(THREAD, "orchestrator"),
            root_meta.clone(),
        )
        .unwrap()
        .append_turn(TranscriptTurn {
            prev: &[],
            next: &turn,
            meta: &root_meta,
            turn_usage: Some(&usage(100, 10)),
            request_id: Some("req-1"),
            tools: None,
        })
        .unwrap();
    let child_meta = meta("researcher", "subagent");
    let child = vec![
        TranscriptMessage::user("look it up"),
        TranscriptMessage::assistant("found it"),
    ];
    locator
        .open_stem("1700000000_orchestrator__researcher", child_meta.clone())
        .unwrap()
        .append_turn(TranscriptTurn {
            prev: &[],
            next: &child,
            meta: &child_meta,
            turn_usage: Some(&usage(40, 4)),
            request_id: None,
            tools: None,
        })
        .unwrap();
}

#[tokio::test]
async fn a_profiles_thread_is_read_from_its_own_store() {
    let dir = tempfile::tempdir().unwrap();
    let profile = ctx_for(dir.path(), Some("profile-t"));
    write_thread(&profile).await;

    let page = transcript_get(
        &profile,
        TranscriptGetRequest {
            thread_id: THREAD.into(),
            cursor: None,
            limit: None,
        },
    )
    .await
    .expect("transcript page")
    .value
    .data
    .expect("page data");
    assert!(page.has_transcript, "the profile reads its own transcript");
    assert!(!page.items.is_empty());

    let spend = super::super::usage::token_usage(
        &profile,
        super::super::usage::ThreadTokenUsageRequest {
            thread_id: THREAD.into(),
        },
    )
    .await
    .expect("usage")
    .value
    .data
    .expect("usage data");
    assert!(spend.has_usage);
    assert_eq!(
        spend.input_tokens, 140,
        "root and sub-agent are both counted"
    );
    assert_eq!(spend.output_tokens, 14);
    assert_eq!(spend.subagents.len(), 1);
    assert_eq!(spend.subagents[0].agent_id, "researcher");

    // The same workspace without the profile resolves another key, and so
    // never sees the profile's thread.
    let desktop = ctx_for(dir.path(), None);
    let page = transcript_get(
        &desktop,
        TranscriptGetRequest {
            thread_id: THREAD.into(),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap()
    .value
    .data
    .unwrap();
    assert!(!page.has_transcript);
}

#[tokio::test]
async fn a_blank_thread_id_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let err = transcript_get(
        &ctx_for(dir.path(), None),
        TranscriptGetRequest {
            thread_id: "  ".into(),
            cursor: None,
            limit: None,
        },
    )
    .await
    .expect_err("blank thread id");
    assert!(err.contains("thread_id is required"));
}

/// After the operator changes `[agent] chat_agent_id`, a thread written under
/// the earlier chat agent keeps its transcript store: the readers use the key
/// recorded with the thread, not the one the current config would derive.
#[tokio::test]
async fn changing_the_chat_agent_does_not_strand_an_existing_threads_turns() {
    let _ = crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins();
    let dir = tempfile::tempdir().unwrap();
    let before = ctx_with_chat_agent(dir.path(), None, None);
    let after = ctx_with_chat_agent(dir.path(), None, Some("planner"));

    // The two configs derive different keys, or this proves nothing.
    let rule_before = super::super::support::transcript_store(&before, THREAD)
        .await
        .unwrap()
        .key;
    let rule_after = super::super::support::transcript_store(&after, "unrecorded-thread")
        .await
        .unwrap()
        .key;
    assert_ne!(
        rule_before, rule_after,
        "the chat agent flip changes the key"
    );

    // The first session on the thread records its key, then writes under it.
    let recorded = crate::threads::store::bind_transcript_key(
        &before,
        dir.path(),
        THREAD,
        rule_before.clone(),
    )
    .await;
    assert_eq!(recorded, rule_before);
    write_thread(&before).await;

    let request = || TranscriptGetRequest {
        thread_id: THREAD.into(),
        cursor: None,
        limit: None,
    };
    let page = transcript_get(&after, request())
        .await
        .unwrap()
        .value
        .data
        .unwrap();
    assert!(
        page.has_transcript && !page.items.is_empty(),
        "the earlier turns are still read after the chat agent changed"
    );
    let spend = super::super::usage::token_usage(
        &after,
        super::super::usage::ThreadTokenUsageRequest {
            thread_id: THREAD.into(),
        },
    )
    .await
    .unwrap()
    .value
    .data
    .unwrap();
    assert_eq!(spend.input_tokens, 140);

    // A thread with no recorded key falls back to the current rule.
    let other = tempfile::tempdir().unwrap();
    let unrecorded_before = ctx_with_chat_agent(other.path(), None, None);
    let unrecorded_after = ctx_with_chat_agent(other.path(), None, Some("planner"));
    write_thread(&unrecorded_before).await;
    let page = transcript_get(&unrecorded_after, request())
        .await
        .unwrap()
        .value
        .data
        .unwrap();
    assert!(
        !page.has_transcript,
        "without a recorded key the reader keeps today's rule"
    );
}
