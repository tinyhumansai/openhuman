use super::*;
use crate::core::events::ConversationToolCall;
use crate::core::runtime::context::CoreContext;
use crate::core::runtime::DomainSet;
use crate::memory::conversations;
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use tinymemory::{ItemKind, MetaFilter};

fn turn_event(workspace_dir: &std::path::Path, thread: &str) -> DomainEvent {
    DomainEvent::ConversationTurnCommitted {
        thread_id: thread.to_string(),
        agent_id: Some("orchestrator".into()),
        workspace: Some("/work".into()),
        channel: Some("Web".into()),
        user_text: "hello there".into(),
        assistant_text: "general kenobi".into(),
        tool_calls: vec![ConversationToolCall {
            name: "shell".into(),
            id: Some("call-1".into()),
        }],
        workspace_dir: workspace_dir.to_path_buf(),
    }
}

#[test]
fn committed_turn_maps_the_event_and_ignores_others() {
    let event = turn_event(std::path::Path::new("/tmp/ws"), "t-1");
    let turn = committed_turn(&event).expect("a turn");
    assert_eq!(turn.thread_id, "t-1");
    assert_eq!(turn.agent_id.as_deref(), Some("orchestrator"));
    assert_eq!(turn.workspace.as_deref(), Some("/work"));
    assert_eq!(turn.channel.as_deref(), Some("Web"));
    assert_eq!(turn.user, "hello there");
    assert_eq!(turn.assistant, "general kenobi");
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].name, "shell");
    assert_eq!(turn.tool_calls[0].id.as_deref(), Some("call-1"));

    let other = DomainEvent::CronSystemJobDue {
        job: CONTEXT_REFRESH_JOB.into(),
    };
    assert!(committed_turn(&other).is_none());
}

#[test]
fn subscribers_declare_their_names_and_domains() {
    let ingest = ConversationIngestSubscriber;
    assert_eq!(ingest.name(), "memory::conversation_ingest");
    assert_eq!(ingest.domains(), Some(&["agent"][..]));
    let jobs = SystemJobsSubscriber;
    assert_eq!(jobs.name(), "memory::system_jobs");
    assert_eq!(jobs.domains(), Some(&["cron"][..]));
}

#[tokio::test]
async fn run_system_job_refreshes_context_md_when_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    run_system_job(&config, CONTEXT_REFRESH_JOB).await;
    assert!(
        crate::memory::context::context_path(&config.workspace_dir).exists(),
        "the refresh job writes context.md"
    );
}

#[tokio::test]
async fn run_system_job_skips_refresh_when_context_is_disabled_or_memory_off() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    bind_reference(&config);
    config.memory.context.enabled = false;
    run_system_job(&config, CONTEXT_REFRESH_JOB).await;
    assert!(!crate::memory::context::context_path(&config.workspace_dir).exists());

    let tmp_off = tempfile::tempdir().unwrap();
    let off = config_in(&tmp_off);
    run_system_job(&off, CONTEXT_REFRESH_JOB).await;
    assert!(!crate::memory::context::context_path(&off.workspace_dir).exists());
}

#[tokio::test]
async fn run_system_job_starts_due_source_syncs() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    let engine = bind_reference(&config);
    let folder = tmp.path().join("notes");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("a.md"), "# Notes\n\nthe due source says hello").unwrap();
    config
        .memory
        .sources
        .push(crate::config::schema::MemorySourceConfig {
            id: "src-due".into(),
            kind: crate::config::schema::MemorySourceKind::Folder,
            target: folder.display().to_string(),
            label: "Notes".into(),
            schedule_mins: Some(15),
        });
    run_system_job(&config, SOURCES_SYNC_JOB).await;
    // The sync runs in the background; wait for its recorded state.
    for _ in 0..200 {
        let states = crate::memory::sources::state::load(&config.workspace_dir);
        if states
            .get("src-due")
            .is_some_and(|s| s.status != crate::memory::types::SourceStatus::Syncing)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let docs = stored(
        &engine,
        MetaFilter {
            kinds: vec![ItemKind::Document],
            ..MetaFilter::default()
        },
    )
    .await;
    assert!(!docs.is_empty(), "the due source was synced");
    // An unrelated job is ignored.
    run_system_job(&config, "something_else").await;
}

#[tokio::test]
async fn system_jobs_subscriber_only_reacts_to_memory_jobs() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let subscriber = SystemJobsSubscriber;
    let ctx = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    let path = crate::memory::context::context_path(&config.workspace_dir);
    CoreContext::scope(ctx, async {
        subscriber
            .handle(&DomainEvent::CronSystemJobDue {
                job: "other".into(),
            })
            .await;
        assert!(!path.exists(), "a foreign job does nothing");
        subscriber
            .handle(&DomainEvent::CronSystemJobDue {
                job: CONTEXT_REFRESH_JOB.into(),
            })
            .await;
        assert!(path.exists(), "the refresh job ran");
        subscriber
            .handle(&turn_event(std::path::Path::new("/x"), "t"))
            .await;
    })
    .await;
}

#[tokio::test]
async fn ingest_subscriber_stores_a_committed_turn_with_names_and_ids_only() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.conversations.batch_turns = 1;
    config
        .save()
        .await
        .expect("config saved beside the workspace");
    let engine = bind_reference(&config);

    ConversationIngestSubscriber
        .handle(&turn_event(&config.workspace_dir, "thread-ingest"))
        .await;
    // Other events are ignored.
    ConversationIngestSubscriber
        .handle(&DomainEvent::CronSystemJobDue { job: "x".into() })
        .await;

    let items = stored(
        &engine,
        MetaFilter {
            kinds: vec![ItemKind::Conversation],
            ..MetaFilter::default()
        },
    )
    .await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].meta.thread_id.as_deref(), Some("thread-ingest"));
    assert!(items[0].text.contains("hello there"));
    assert_eq!(
        conversations::recent(&config.workspace_dir)
            .first()
            .map(|r| r.thread_id.as_str()),
        Some("thread-ingest")
    );
}
