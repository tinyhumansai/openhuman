use super::*;

use tinyagents_harness::summarization::{checkpoint_message, SummaryPlacement};
use tinyinference_llm::ToolCall;

fn external_channel_origin(
    channel: &str,
    message_id: &str,
) -> crate::core::runtime::turn_origin::AgentTurnOrigin {
    crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel {
        channel: channel.into(),
        sender: None,
        sender_name: None,
        reply_target: "chat".into(),
        message_id: message_id.into(),
        history_key: None,
        channel_session: None,
    }
}

#[test]
fn logged_channel_keeps_normalized_custom_external_provider_ids() {
    assert_eq!(
        logged_channel(Some(&external_channel_origin(
            "  MyCustomProvider  ",
            "message-1"
        ))),
        Some("mycustomprovider".into()),
        "an external provider remains the channel owner even when it is not in the built-in catalog"
    );
}

#[test]
fn logged_channel_is_none_for_non_channel_origins_none_and_cron_replays() {
    use crate::core::runtime::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

    assert_eq!(logged_channel(None), None);
    assert_eq!(logged_channel(Some(&AgentTurnOrigin::Cli)), None);
    assert_eq!(logged_channel(Some(&AgentTurnOrigin::DirectChat)), None);
    assert_eq!(logged_channel(Some(&AgentTurnOrigin::Unknown)), None);
    assert_eq!(
        logged_channel(Some(&AgentTurnOrigin::TrustedAutomation {
            job_id: "job-1".into(),
            source: TrustedAutomationSource::Cron,
        })),
        None
    );
    assert_eq!(
        logged_channel(Some(&external_channel_origin(
            "telegram",
            "cron:job-1:run-1"
        ))),
        None
    );
}

#[tokio::test]
async fn session_pre_and_post_turn_use_the_external_channel_origin_not_event_labels() {
    crate::agent::session_host::test_support::install_model_fixture();
    use crate::memory::test_fixtures::stored;
    use tinyagents_runtime::{
        CommitReceipt, ResumeMode, SessionTurnOutcome, TranscriptTurnOptions,
    };
    use tinymemory_api::{ItemKind, ListRequest, MemoryEngine, MetaFilter};

    async fn log_turn(
        prelude: &mut OpenHumanTurnPrelude,
        engine: &tinymemory_api::conformance::ReferenceEngine,
        thread_id: &str,
        origin: Option<crate::core::runtime::turn_origin::AgentTurnOrigin>,
        expected_channel: Option<&str>,
    ) {
        prelude.thread_id = Some(thread_id.into());
        prelude
            .mutable
            .lock()
            .expect("prelude state")
            .pending_user_text = Some(format!("question for {thread_id}"));
        let _pack = prelude
            .memory_pre_turn(&[], 0, None, origin)
            .await
            .expect("memory turn");
        let pending_channel = prelude
            .mutable
            .lock()
            .expect("prelude state")
            .pending_memory_turn
            .as_ref()
            .and_then(|pending| pending.channel.clone());
        assert_eq!(pending_channel.as_deref(), expected_channel);

        let receipt = CommitReceipt {
            outcome: SessionTurnOutcome {
                history: vec![],
                output: Some(format!("answer for {thread_id}")),
                interrupted: false,
            },
            options: TranscriptTurnOptions {
                request_id: None,
                thread_id: Some(thread_id.into()),
                stream: false,
                resume: ResumeMode::Never,
                context: crate::agent::tinyagents::host::OpenHumanRunContext::new(),
            },
            transcript: None,
        };
        prelude.memory_post_turn(&receipt, None);

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let items = stored(
                    engine,
                    MetaFilter {
                        thread_id: Some(thread_id.into()),
                        ..MetaFilter::kinds([ItemKind::Conversation])
                    },
                )
                .await;
                if items.len() >= 2 {
                    assert_eq!(items.len(), 2);
                    let user_turn = items
                        .iter()
                        .find(|item| item.text.contains(&format!("question for {thread_id}")))
                        .expect("logged user turn");
                    let assistant_turn = items
                        .iter()
                        .find(|item| item.text.contains(&format!("answer for {thread_id}")))
                        .expect("logged assistant turn");
                    assert_eq!(user_turn.meta.channel.as_deref(), expected_channel);
                    assert_eq!(assistant_turn.meta.channel.as_deref(), expected_channel);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("both turns should be logged with their origin metadata");
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    config.memory.conversations.enabled = true;
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");
    crate::agent::session_host::test_support::install_managed_model_fixture(&config).await;
    let engine = Arc::new(tinymemory_api::conformance::ReferenceEngine::new());
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());

    let mut host = crate::agent::OpenHumanSessionHost::from_config_for_agent(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "orchestrator",
    )
    .await
    .expect("host");
    host.ensure_runtime_session().expect("runtime session");
    let mut prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("turn prelude");
    prelude.runtime_config = Some(Arc::new(config.clone()));
    prelude.event_channel = "voice-event-label".into();

    let external = external_channel_origin("  MyCustomProvider  ", "message-1");
    log_turn(
        &mut prelude,
        &engine,
        "external-channel-thread",
        Some(external),
        Some("mycustomprovider"),
    )
    .await;

    use crate::core::runtime::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};
    log_turn(
        &mut prelude,
        &engine,
        "cli-thread",
        Some(AgentTurnOrigin::Cli),
        None,
    )
    .await;
    log_turn(
        &mut prelude,
        &engine,
        "cron-replay-thread",
        Some(external_channel_origin("telegram", "cron:job-1:run-1")),
        None,
    )
    .await;

    let all = engine
        .list(ListRequest {
            filter: MetaFilter::kinds([ItemKind::Conversation]),
            limit: 100,
            cursor: None,
        })
        .await
        .expect("list all logged turns");
    assert_eq!(all.items.len(), 6);
    assert_eq!(
        logged_channel(Some(&AgentTurnOrigin::TrustedAutomation {
            job_id: "job-1".into(),
            source: TrustedAutomationSource::Cron,
        })),
        None
    );
}

fn assistant_with_call(id: &str, name: &str) -> Message {
    let mut message = Message::assistant("");
    if let Message::Assistant(assistant) = &mut message {
        assistant
            .tool_calls
            .push(ToolCall::new(id, name, serde_json::json!({})));
    }
    message
}

#[test]
fn without_a_checkpoint_the_whole_thread_is_in_the_prompt() {
    let history = vec![
        Message::user("one"),
        Message::assistant("a"),
        Message::user("two"),
    ];
    assert_eq!(in_prompt_window(&history, 2, None), (0, false));
}

#[test]
fn after_a_checkpoint_the_window_starts_at_the_first_kept_turn() {
    let current = Message::user("five");
    let history = vec![
        Message::system("prompt"),
        checkpoint_message(SummaryPlacement::User, "earlier turns"),
        Message::user("three"),
        Message::assistant("c"),
        Message::user("four"),
        Message::assistant("d"),
        current.clone(),
    ];
    // Five turns committed before this one; turns 3 and 4 are verbatim, so
    // the window opens at turn 3, user index 6.
    assert_eq!(in_prompt_window(&history, 5, Some(&current)), (6, true));
    assert_eq!(
        in_prompt_window(&history[..6], 5, None),
        (6, true),
        "the request input is not counted whether or not it was appended"
    );
}

#[test]
fn committed_tool_calls_pair_each_call_with_its_result() {
    let history = vec![
        Message::user("earlier"),
        assistant_with_call("old", "ignored"),
        Message::user("find the file"),
        assistant_with_call("c1", "glob"),
        Message::tool("c1", "src/main.rs"),
        assistant_with_call("c2", "noop"),
        Message::assistant("Found it."),
    ];
    let calls = committed_tool_calls(&history);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "glob");
    assert_eq!(calls[0].id.as_deref(), Some("c1"));
    assert_eq!(calls[0].result.as_deref(), Some("src/main.rs"));
    assert_eq!(calls[1].result, None);
}

#[tokio::test]
async fn deferred_memory_commit_survives_the_turn_scope_without_pinning_the_profile() {
    use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};
    use crate::profiles::{ProfileHost, ProfileId};
    use tinymemory_api::{
        LearningKind, ListRequest, MemoryEngine, MemoryMeta, MetaFilter, StoreItem,
    };

    let root = tempfile::tempdir().unwrap();
    let engine = Arc::new(tinymemory_api::conformance::ReferenceEngine::new());
    let capabilities = Arc::new(crate::profiles::capabilities::ProfileCapabilities::new(
        None,
        None,
        None,
        Default::default(),
        Vec::new(),
        Some(engine.clone()),
    ));
    let host = ProfileHost::new(
        SaasConfig::new(root.path()),
        CoreContext::for_test(DomainSet::full(), None),
    )
    .with_capabilities(Some(capabilities));
    let id = ProfileId::parse("deferred-memory").unwrap();
    host.provision(&id).await.unwrap();
    let profile = host.open(&id).await.unwrap();
    let captured =
        capture_profile_context(&DispatchCtx::new(profile.context().clone()), Some(&host));
    // The entire temporary turn context is gone before this callback is polled.
    let config = profile.config.clone();
    let work = async {
        assert_eq!(
            crate::core::runtime::current_tenant()
                .unwrap()
                .profile
                .as_deref(),
            Some(id.as_str())
        );
        let binding = crate::memory::engine::resolve(&config)
            .await
            .engine()
            .unwrap();
        binding
            .engine
            .store(StoreItem::learning(
                "delayed committed reply",
                LearningKind::Fact,
                0.9,
                MemoryMeta {
                    namespace: config.memory.root.as_ref().unwrap().parse().unwrap(),
                    ..Default::default()
                },
            ))
            .await
            .unwrap();
    };
    deferred_memory_work(captured.clone(), work).await;
    assert_eq!(
        engine
            .list(ListRequest::new(MetaFilter::default(), 10))
            .await
            .unwrap()
            .items
            .len(),
        1,
        "a still-hosted profile must persist its deferred commit after its turn ends"
    );
    drop(profile);
    assert!(
        host.release(&id).await.unwrap(),
        "the pending weak capture cannot pin an idle profile"
    );
    assert!(
        matches!(&captured, ProfileMemoryContext::BoundWeak(context) if context.upgrade().is_none()),
        "the retired owner is not retained"
    );
    deferred_memory_work(captured, async {
        panic!("retired callbacks must fail closed")
    })
    .await;
}

#[tokio::test]
async fn missing_profile_owner_refuses_deferred_memory_instead_of_running_unscoped() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet, SaasConfig};
    use crate::profiles::ProfileHost;
    use crate::tools::toolpacks::ToolGroups;
    let root = tempfile::tempdir().unwrap();
    let operator = CoreContext::for_test(DomainSet::full(), None);
    let host = ProfileHost::new(SaasConfig::new(root.path()), operator.clone());
    let missing = operator.derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            ToolGroups::none(),
        )
        .profile("missing-owner"),
    );
    let captured = capture_profile_context(&DispatchCtx::new(missing), Some(&host));
    deferred_memory_work(captured, async {
        panic!("a failed profile owner capture must not run under the configured external engine")
    })
    .await;
}

#[tokio::test]
async fn failed_profile_capture_is_denied_but_ordinary_desktop_work_still_runs() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    let operator = CoreContext::for_test(DomainSet::full(), None);
    for id in ["missing-host", "invalid/owner"] {
        let profile = operator.derive_with(
            ContextOverlay::new(
                crate::config::Config::default(),
                DomainSet::full(),
                ToolGroups::none(),
            )
            .profile(id),
        );
        let captured = capture_profile_context(&DispatchCtx::new(profile), None);
        assert!(matches!(captured, ProfileMemoryContext::Denied));
        deferred_memory_work(captured, async {
            panic!("a profile without its host cannot resolve an external engine")
        })
        .await;
    }
    let captured = capture_profile_context(&DispatchCtx::new(operator), None);
    assert!(matches!(captured, ProfileMemoryContext::NonProfile));
    let mut ran = false;
    deferred_memory_work(captured, async {
        ran = true;
    })
    .await;
    assert!(
        ran,
        "ordinary desktop work retains its existing deferred behavior"
    );
}
