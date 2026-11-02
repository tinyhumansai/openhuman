use crate::channels::routes::tests::runtime_context;
use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tinyagents_session::transcript::{SessionRef, TranscriptMessage, TranscriptMeta};

fn transcript_meta(thread_id: &str, agent_id: &str) -> TranscriptMeta {
    TranscriptMeta {
        agent_name: agent_id.into(),
        agent_id: Some(agent_id.into()),
        agent_type: Some("root".into()),
        dispatcher: "native".into(),
        provider: Some("mock".into()),
        model: Some("session-test".into()),
        created: "2026-10-20T00:00:00Z".into(),
        updated: "2026-10-20T00:00:00Z".into(),
        turn_count: 1,
        prefix_message_count: Some(0),
        prefix_reset: false,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some(thread_id.into()),
        task_id: None,
        session_id: None,
        parent_session_id: None,
    }
}

fn remote_session_scope(session_key: &str) -> tinychannels_bus::module_remote::RemoteSessionScope {
    tinychannels_bus::module_remote::RemoteSessionScope {
        channel: tinychannels_bus::ChannelRef {
            id: "telegram".into(),
            account_id: Some("telegram:bot-a".into()),
        },
        conversation: tinychannels_bus::ConversationRef {
            kind: tinychannels_bus::ConversationKind::Group,
            id: "room-1".into(),
            topic_id: Some("topic-3".into()),
            ..Default::default()
        },
        session_key: session_key.into(),
    }
}

fn seed_session(
    core: &CoreContext,
    workspace: &Path,
    key: &str,
    agent_id: &str,
    store_agent_id: &str,
) -> SessionRef {
    // The session host resolves its identity through the tenant-aware
    // admission, whose agent is the scoped store key, not the bare definition id.
    let session = SessionRef::scoped(key, store_agent_id);
    let history =
        crate::storage::sessions::transcripts_or_files_in(core, store_agent_id, workspace)
            .open_session(&session, transcript_meta(key, agent_id))
            .unwrap();
    history
        .append(TranscriptMessage::user("prior tenant turn"))
        .unwrap();
    history
        .append(TranscriptMessage::assistant("prior tenant reply"))
        .unwrap();
    session
}

fn session_messages(
    core: &CoreContext,
    workspace: &Path,
    store_agent_id: &str,
    session: &SessionRef,
) -> Vec<String> {
    let locator =
        crate::storage::sessions::transcripts_or_files_in(core, store_agent_id, workspace);
    let head = locator.head_generation(session);
    locator
        .read_session_transcript(&head)
        .expect("durable transcript exists")
        .read_session()
        .unwrap()
        .expect("transcript is readable")
        .messages
        .into_iter()
        .map(|message| message.content)
        .collect()
}

fn tenant_runtime(
    profile: &str,
    workspace: &Path,
) -> (
    Arc<CoreContext>,
    crate::channels::context::ChannelRuntimeContext,
) {
    let workspace = workspace.to_path_buf();
    let mut config = crate::config::Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace.clone(),
        config_path: workspace.join("config.toml"),
        ..Default::default()
    };
    config.cloud_providers = vec![crate::config::schema::CloudProviderCreds {
        id: "p_openai_reset_test".into(),
        slug: "openai".into(),
        label: "Reset test provider".into(),
        endpoint: "http://127.0.0.1:9/v1".into(),
        default_model: Some("reset-test-model".into()),
        ..Default::default()
    }];
    let core = CoreContext::for_test(DomainSet::none(), Some(workspace.clone())).derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), Default::default()).profile(profile),
    );
    // Store a standing credential in the tenant's own scope so a storage-backed
    // secret store records it where the host agent later reads it.
    CoreContext::sync_scope(Arc::clone(&core), || {
        crate::security::credentials::api_key::store_api_key(&config, "test-session-fixture-key")
    })
    .expect("store reset test credential");
    let mut runtime = runtime_context(workspace);
    runtime.config = Some(Arc::new(config));
    runtime.core = Some(Arc::downgrade(&core));
    runtime.channels_by_name = Arc::new(HashMap::from([(
        "telegram:bot-a".into(),
        Arc::new(crate::channels::runtime::ChannelHandle::Test(Arc::new(
            crate::channels::runtime::TestChannel::new("telegram:bot-a"),
        ))),
    )]));
    (core, runtime)
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn remote_clear_resets_only_the_captured_tenant_session() {
    use tinychannels_bus::module_remote::{RemoteHostRequest, RemoteHostResult};

    let _storage = crate::channels::module_fixture::install_memory_storage();
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path();
    let (alice_core, alice_runtime) = tenant_runtime("remote-reset-alice", workspace);
    let (bob_core, _bob_runtime) = tenant_runtime("remote-reset-bob", workspace);
    let alice_dispatch = DispatchCtx::new(Arc::clone(&alice_core));
    let bob_dispatch = DispatchCtx::new(Arc::clone(&bob_core));
    let alice_tenant = alice_dispatch.tenant().unwrap();
    let bob_tenant = bob_dispatch.tenant().unwrap();
    let alice_route = crate::channels::routes::get_route_selection(&alice_runtime, "remote-reset");
    let bob_route = crate::channels::routes::get_route_selection(&_bob_runtime, "remote-reset");
    let alice_agent =
        match crate::channels::runtime::dispatch::host_agent::route(&alice_runtime, "telegram") {
            crate::channels::runtime::dispatch::host_agent::HostRoute::Agent(host) => *host,
            crate::channels::runtime::dispatch::host_agent::HostRoute::Unbound => {
                crate::channels::runtime::dispatch::host_agent::configured_agent(
                    &alice_runtime,
                    &alice_route,
                )
                .await
                .expect("Alice's configured channel agent resolves")
            }
            crate::channels::runtime::dispatch::host_agent::HostRoute::Missing(agent) => {
                panic!("Alice's channel agent {agent} is unavailable")
            }
        };
    let bob_agent =
        match crate::channels::runtime::dispatch::host_agent::route(&_bob_runtime, "telegram") {
            crate::channels::runtime::dispatch::host_agent::HostRoute::Agent(host) => *host,
            crate::channels::runtime::dispatch::host_agent::HostRoute::Unbound => {
                crate::channels::runtime::dispatch::host_agent::configured_agent(
                    &_bob_runtime,
                    &bob_route,
                )
                .await
                .expect("Bob's configured channel agent resolves")
            }
            crate::channels::runtime::dispatch::host_agent::HostRoute::Missing(agent) => {
                panic!("Bob's channel agent {agent} is unavailable")
            }
        };
    let canonical = "channel-session:telegram:bot-a:group:room-1";
    let alice_key = crate::core::runtime::tenant::tenant_key(&alice_tenant, canonical);
    let bob_key = crate::core::runtime::tenant::tenant_key(&bob_tenant, canonical);
    let alice_store_agent =
        crate::storage::sessions::current_agent_key_or(&alice_dispatch, &alice_agent.definition.id);
    let bob_store_agent =
        crate::storage::sessions::current_agent_key_or(&bob_dispatch, &bob_agent.definition.id);
    assert_ne!(alice_key, bob_key);
    let alice_session = seed_session(
        &alice_core,
        workspace,
        &alice_key,
        &alice_agent.definition.id,
        &alice_store_agent,
    );
    let bob_session = seed_session(
        &bob_core,
        workspace,
        &bob_key,
        &bob_agent.definition.id,
        &bob_store_agent,
    );
    assert_eq!(
        super::durable_channel_history_len(
            &alice_runtime,
            &alice_route,
            "telegram",
            Some(&alice_key),
        )
        .await,
        2,
        "remote status history comes from the durable session",
    );

    let scope = remote_session_scope(canonical);
    let reset = crate::channels::host::remote_control::RemoteResetContext {
        scope: scope.clone(),
        durable_session_key: alice_key.clone(),
        message: tinychannels_bus::ChannelMessage {
            channel: "telegram".into(),
            reply_target: "room-1".into(),
            content: "/new".into(),
            ..Default::default()
        },
        history_key: "legacy-key-is-not-authority".into(),
    };
    let cancellation = tokio_util::sync::CancellationToken::new();

    let mut forged = scope.clone();
    forged.conversation.id = "other-room".into();
    assert!(crate::channels::host::remote_control::dispatch_callback(
        &alice_dispatch,
        &alice_runtime,
        &alice_tenant,
        &cancellation,
        Some(&reset),
        RemoteHostRequest::ClearHistory {
            session_scope: forged,
        },
    )
    .await
    .is_err());
    assert_eq!(
        session_messages(&alice_core, workspace, &alice_store_agent, &alice_session).len(),
        2
    );
    assert_eq!(
        session_messages(&bob_core, workspace, &bob_store_agent, &bob_session).len(),
        2
    );

    let cleared = crate::channels::host::remote_control::dispatch_callback(
        &alice_dispatch,
        &alice_runtime,
        &alice_tenant,
        &cancellation,
        Some(&reset),
        RemoteHostRequest::ClearHistory {
            session_scope: scope,
        },
    )
    .await
    .expect("captured tenant reset succeeds");
    assert!(matches!(cleared, RemoteHostResult::Completed));
    let alice_after = session_messages(&alice_core, workspace, &alice_store_agent, &alice_session);
    assert!(
        alice_after.is_empty(),
        "reset should empty Alice's actual session: agent={}, session={alice_session:?}, \
         head={:?}, messages={alice_after:?}, history_len={}",
        alice_agent.definition.id,
        crate::storage::sessions::transcripts_or_files_in(
            &alice_core,
            &alice_store_agent,
            workspace,
        )
        .head_generation(&alice_session),
        super::durable_channel_history_len(
            &alice_runtime,
            &alice_route,
            "telegram",
            Some(&alice_key),
        )
        .await,
    );
    assert_eq!(
        super::durable_channel_history_len(
            &alice_runtime,
            &alice_route,
            "telegram",
            Some(&alice_key),
        )
        .await,
        0,
        "a reset clears the durable status count",
    );

    let alice_locator = crate::storage::sessions::transcripts_or_files_in(
        &alice_core,
        &alice_store_agent,
        workspace,
    );
    let cleared_head = alice_locator.head_generation(&alice_session);
    let repeated = crate::channels::host::remote_control::dispatch_callback(
        &alice_dispatch,
        &alice_runtime,
        &alice_tenant,
        &cancellation,
        Some(&reset),
        RemoteHostRequest::ClearHistory {
            session_scope: remote_session_scope(canonical),
        },
    )
    .await
    .expect("resetting an already-empty durable session is an idempotent success");
    assert!(matches!(repeated, RemoteHostResult::Completed));
    assert_eq!(alice_locator.head_generation(&alice_session), cleared_head);
    assert!(
        session_messages(&alice_core, workspace, &alice_store_agent, &alice_session).is_empty()
    );

    let empty_canonical = "channel-session:telegram:bot-a:group:empty-room";
    let empty_key = crate::core::runtime::tenant::tenant_key(&alice_tenant, empty_canonical);
    let empty_session = SessionRef::scoped(empty_key, &alice_agent.definition.id);
    let empty_scope = remote_session_scope(empty_canonical);
    let empty_reset = crate::channels::host::remote_control::RemoteResetContext {
        scope: empty_scope.clone(),
        durable_session_key: empty_session.session_key.clone(),
        message: tinychannels_bus::ChannelMessage {
            channel: "telegram".into(),
            reply_target: "empty-room".into(),
            content: "/new".into(),
            ..Default::default()
        },
        history_key: "legacy-key-is-not-authority".into(),
    };
    let absent = crate::channels::host::remote_control::dispatch_callback(
        &alice_dispatch,
        &alice_runtime,
        &alice_tenant,
        &cancellation,
        Some(&empty_reset),
        RemoteHostRequest::ClearHistory {
            session_scope: empty_scope,
        },
    )
    .await
    .expect("clearing a valid but not-yet-created conversation is a no-op");
    assert!(matches!(absent, RemoteHostResult::Completed));
    let empty_head = alice_locator.head_generation(&empty_session);
    assert!(alice_locator.read_session_transcript(&empty_head).is_none());

    assert_eq!(
        session_messages(&bob_core, workspace, &bob_store_agent, &bob_session),
        ["prior tenant turn", "prior tenant reply"]
    );
}
