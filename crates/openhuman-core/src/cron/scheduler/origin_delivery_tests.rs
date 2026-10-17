use super::*;
use crate::threads::store::{self as conversations, CreateConversationThread};
use tinyagents_session::transcript::{
    SessionRef, TranscriptMessage, TranscriptMeta, TranscriptTurn,
};
use {
    tinyflows_bus::schedule::types::DeliveryConfig, tinyflows_bus::schedule::types::JobType,
    tinyflows_bus::schedule::types::Schedule, tinyflows_bus::schedule::types::SessionTarget,
};

fn config(tmp: &tempfile::TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn job(origin: Option<JobOrigin>) -> CronJob {
    CronJob {
        claim_generation: 0,
        policy: Default::default(),
        id: "job-1".into(),
        expression: "*/5 * * * *".into(),
        schedule: Schedule::Every { every_ms: 300_000 },
        command: String::new(),
        prompt: Some("drink water".into()),
        name: Some("water".into()),
        job_type: JobType::Agent,
        session_target: SessionTarget::Current,
        model: None,
        agent_id: None,
        enabled: true,
        delivery: DeliveryConfig {
            mode: "origin".into(),
            channel: None,
            to: None,
            best_effort: true,
        },
        delete_after_run: false,
        created_at: chrono::Utc::now(),
        next_run: chrono::Utc::now(),
        last_run: None,
        last_status: None,
        last_output: None,
        origin,
    }
}

fn web(thread: &str) -> JobOrigin {
    JobOrigin::Web {
        thread_id: thread.into(),
        agent_id: None,
    }
}

fn seed_thread(config: &Config, id: &str) {
    conversations::ensure_thread(
        &crate::core::runtime::DispatchCtx::ambient(),
        config.workspace_dir.clone(),
        CreateConversationThread {
            id: id.into(),
            title: "Chat".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .unwrap();
}

#[test]
fn suppressed_output_is_blank_or_exactly_no_reply() {
    for s in ["", "  \n", "NO_REPLY", "  NO_REPLY\n"] {
        assert!(is_suppressed_output(s), "{s:?}");
    }
    for s in ["no_reply", "NO_REPLY please", "Drink water!"] {
        assert!(!is_suppressed_output(s), "{s:?}");
    }
}

#[tokio::test]
async fn no_reply_is_suppressed_without_writing_anything() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    seed_thread(&cfg, "t-1");
    let status = deliver_to_origin(
        &crate::core::runtime::DispatchCtx::ambient(),
        &cfg,
        &job(Some(web("t-1"))),
        "run-1",
        " NO_REPLY ",
    )
    .await
    .unwrap();
    assert_eq!(status, DeliveryStatus::Suppressed);
    assert!(conversations::get_messages(
        &crate::core::runtime::DispatchCtx::ambient(),
        cfg.workspace_dir.clone(),
        "t-1"
    )
    .unwrap()
    .is_empty());
}

#[tokio::test]
async fn web_delivery_stores_one_row_under_the_run_reply_id_and_is_idempotent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    seed_thread(&cfg, "t-1");
    let j = job(Some(web("t-1")));

    for _ in 0..2 {
        let status = deliver_to_origin(
            &crate::core::runtime::DispatchCtx::ambient(),
            &cfg,
            &j,
            "run-1",
            "  Drink water!  ",
        )
        .await
        .unwrap();
        assert_eq!(status, DeliveryStatus::Delivered);
    }
    let messages = conversations::get_messages(
        &crate::core::runtime::DispatchCtx::ambient(),
        cfg.workspace_dir.clone(),
        "t-1",
    )
    .unwrap();
    assert_eq!(messages.len(), 1, "same run collapses onto one row");
    assert_eq!(
        messages[0].id,
        conversations::run_reply_message_id("cron:job-1:run-1")
    );
    assert_eq!(messages[0].content, "Drink water!");
    assert_eq!(messages[0].sender, "agent");

    deliver_to_origin(
        &crate::core::runtime::DispatchCtx::ambient(),
        &cfg,
        &j,
        "run-2",
        "Again!",
    )
    .await
    .unwrap();
    assert_eq!(
        conversations::get_messages(
            &crate::core::runtime::DispatchCtx::ambient(),
            cfg.workspace_dir.clone(),
            "t-1"
        )
        .unwrap()
        .len(),
        2
    );
}

#[tokio::test]
async fn web_delivery_to_a_missing_thread_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    let err = deliver_to_origin(
        &crate::core::runtime::DispatchCtx::ambient(),
        &cfg,
        &job(Some(web("gone"))),
        "run-1",
        "hi",
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("origin thread"));
}

#[tokio::test]
async fn origin_mode_without_an_origin_is_an_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    assert!(deliver_to_origin(
        &crate::core::runtime::DispatchCtx::ambient(),
        &cfg,
        &job(None),
        "run-1",
        "hi"
    )
    .await
    .is_err());
}

fn meta() -> TranscriptMeta {
    TranscriptMeta {
        session_id: None,
        parent_session_id: None,
        agent_name: "orchestrator".into(),
        agent_id: Some("orchestrator".into()),
        agent_type: None,
        dispatcher: "native".into(),
        provider: None,
        model: None,
        created: "2026-10-06T00:00:00Z".into(),
        updated: "2026-10-06T00:00:00Z".into(),
        turn_count: 1,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some("t-1".into()),
        task_id: None,
    }
}

/// After delivery the thread's resumed agent transcript holds the reply as an
/// assistant message (so the next turn sees it), exactly once per run.
#[tokio::test]
async fn web_delivery_appends_to_the_resumed_agent_transcript() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    seed_thread(&cfg, "t-1");

    // A prior live turn on the thread, written the way the session host does:
    // through the store port, under the desktop tenant's key for the agent.
    let locator =
        crate::agent::session_store::transcripts_or_files("orchestrator", &cfg.workspace_dir);
    let session = SessionRef::scoped("t-1", "orchestrator");
    let next = vec![
        TranscriptMessage::user("remind me to drink water"),
        TranscriptMessage::assistant("Scheduled."),
    ];
    locator
        .open_session(&session, meta())
        .unwrap()
        .append_turn(TranscriptTurn {
            prev: &[],
            next: &next,
            meta: &meta(),
            turn_usage: None,
            request_id: None,
            tools: None,
        })
        .unwrap();

    let mut j = job(Some(JobOrigin::Web {
        thread_id: "t-1".into(),
        agent_id: Some("orchestrator".into()),
    }));
    j.id = "job-1".into();
    deliver_to_origin(
        &crate::core::runtime::DispatchCtx::ambient(),
        &cfg,
        &j,
        "run-1",
        "Drink water!",
    )
    .await
    .unwrap();
    deliver_to_origin(
        &crate::core::runtime::DispatchCtx::ambient(),
        &cfg,
        &j,
        "run-1",
        "Drink water!",
    )
    .await
    .unwrap();

    let head = locator.head_generation(&session);
    let resumed = locator
        .read_session_transcript(&head)
        .expect("the session's head is in the store")
        .read_session()
        .unwrap()
        .unwrap();
    assert_eq!(resumed.messages.len(), 3, "reply appended once");
    let last = resumed.messages.last().unwrap();
    assert_eq!(last.role, "assistant");
    assert_eq!(last.content, "Drink water!");
}

/// A background append during a live turn of the same session waits for it.
#[tokio::test]
async fn transcript_append_waits_for_a_live_turn() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    let locator =
        crate::agent::session_store::transcripts_or_files("orchestrator", &cfg.workspace_dir);
    let session = SessionRef::scoped("t-1", "orchestrator");
    let guard = tinyagents_session::transcript::lock_session_turn(locator.as_ref(), &session)
        .await
        .expect("lock");

    let task = tokio::spawn(async move {
        append_to_origin_transcript(
            locator.as_ref(),
            &TranscriptAppend {
                thread_id: "t-1",
                agent_id: "orchestrator",
                text: "hi",
                idempotency_key: "cron:job-1:run-1",
                job_id: "job-1",
                run_id: "run-1",
            },
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!task.is_finished(), "append must wait behind the live turn");
    drop(guard);
    task.await.unwrap().unwrap();
}

/// The append lands in the store the job's tenant keys: a SaaS profile's
/// `<profile>~<agent>` transcript, never the unprefixed one of the same
/// workspace.
#[tokio::test]
async fn transcript_append_lands_in_the_tenants_store() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = config(&tmp);
    let profile = crate::core::runtime::Tenant {
        profile: Some("profile-a".into()),
        agent: None,
    };
    let desktop = crate::core::runtime::Tenant::default();
    let profile_store = origin_transcripts(&cfg.workspace_dir, &profile, "orchestrator");
    let desktop_store = origin_transcripts(&cfg.workspace_dir, &desktop, "orchestrator");
    assert_ne!(
        profile_store.destination_key(),
        desktop_store.destination_key(),
        "the profile prefix keys a store of its own"
    );

    let session = SessionRef::scoped("t-p", "orchestrator");
    let next = vec![
        TranscriptMessage::user("remind me"),
        TranscriptMessage::assistant("Scheduled."),
    ];
    profile_store
        .open_session(&session, meta())
        .unwrap()
        .append_turn(TranscriptTurn {
            prev: &[],
            next: &next,
            meta: &meta(),
            turn_usage: None,
            request_id: None,
            tools: None,
        })
        .unwrap();

    let append = TranscriptAppend {
        thread_id: "t-p",
        agent_id: "orchestrator",
        text: "Ping!",
        idempotency_key: "cron:job-p:run-1",
        job_id: "job-p",
        run_id: "run-1",
    };
    // The unprefixed store has no such session: nothing is written there.
    append_to_origin_transcript(desktop_store.as_ref(), &append)
        .await
        .unwrap();
    assert!(!desktop_store.session_exists(&session));
    append_to_origin_transcript(profile_store.as_ref(), &append)
        .await
        .unwrap();

    let head = profile_store.head_generation(&session);
    let messages = profile_store
        .read_session_transcript(&head)
        .unwrap()
        .read_session()
        .unwrap()
        .unwrap()
        .messages;
    assert_eq!(
        messages.len(),
        3,
        "appended once, to the profile's transcript"
    );
    assert_eq!(messages.last().unwrap().content, "Ping!");
}
