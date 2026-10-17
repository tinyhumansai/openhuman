use super::*;

#[tokio::test]
async fn dispatch_scope_uses_explicit_origin_without_mutating_shared_context() {
    let ctx = ctx("/tmp/origin-scope");
    let origin = crate::agent::turn_origin::AgentTurnOrigin::ExternalChannel {
        sender_name: None,
        channel: "test".into(),
        sender: Some("sender".into()),
        reply_target: "room".into(),
        message_id: "message".into(),
        history_key: None,
    };
    CoreContext::scope_with_turn_origin(ctx.clone(), Some(origin), async {
        assert!(matches!(
            CoreContext::current_turn_origin(),
            Some(crate::agent::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
        ));
    })
    .await;
    assert!(CoreContext::current_turn_origin().is_none());
    assert!(ctx.embedder_config().is_none());
}

#[tokio::test]
async fn missing_child_origin_inherits_bound_external_authority() {
    let ctx = ctx("/tmp/origin-inheritance");
    let external = crate::agent::turn_origin::AgentTurnOrigin::ExternalChannel {
        sender_name: None,
        channel: "test".into(),
        sender: None,
        reply_target: "room".into(),
        message_id: "message".into(),
        history_key: None,
    };
    CoreContext::scope_with_turn_origin(ctx, Some(external), async {
        let parent = CoreContext::current().expect("parent context");
        CoreContext::scope_with_turn_origin(parent, None, async {
            assert!(matches!(
                CoreContext::current_turn_origin(),
                Some(crate::agent::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
            ));
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn explicit_origin_scope_bridges_legacy_and_core_context_without_cross_talk() {
    use crate::agent::turn_origin::{self, AgentTurnOrigin};

    let run = |workspace: &'static str, channel: &'static str| async move {
        let context = ctx(workspace);
        let origin = AgentTurnOrigin::ExternalChannel {
            sender_name: None,
            channel: channel.into(),
            sender: Some(format!("{channel}-sender")),
            reply_target: format!("{channel}-room"),
            message_id: format!("{channel}-message"),
            history_key: None,
        };
        CoreContext::scope(context, async {
            turn_origin::with_origin(origin, async {
                assert!(matches!(
                    turn_origin::current(),
                    Some(AgentTurnOrigin::ExternalChannel { channel: current, .. })
                        if current == channel
                ));
                assert!(matches!(
                    CoreContext::current_turn_origin(),
                    Some(AgentTurnOrigin::ExternalChannel { channel: current, .. })
                        if current == channel
                ));
            })
            .await;

            assert!(turn_origin::current().is_none());
            assert!(CoreContext::current_turn_origin().is_none());
        })
        .await;
    };

    tokio::join!(
        run("/tmp/origin-a", "channel-a"),
        run("/tmp/origin-b", "channel-b")
    );
}

#[tokio::test]
async fn bound_external_authority_wins_over_legacy_trusted_automation() {
    use crate::agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource};

    CoreContext::scope(ctx("/tmp/origin-priority"), async {
        turn_origin::with_origin(
            AgentTurnOrigin::TrustedAutomation {
                job_id: "authorized-automation".into(),
                source: TrustedAutomationSource::Background,
            },
            async {
                let parent = CoreContext::scoped().unwrap();
                let external = AgentTurnOrigin::ExternalChannel {
                    channel: "telegram".into(),
                    sender: Some("external-sender".into()),
                    sender_name: None,
                    reply_target: "room".into(),
                    message_id: "message".into(),
                    history_key: None,
                };
                CoreContext::scope_with_turn_origin(parent, Some(external), async {
                    assert!(matches!(
                        turn_origin::current(),
                        Some(AgentTurnOrigin::ExternalChannel { sender, .. })
                            if sender.as_deref() == Some("external-sender")
                    ));
                    let child = crate::core::runtime::DispatchCtx::ambient()
                        .spawn(async { turn_origin::current() })
                        .await
                        .unwrap();
                    assert!(matches!(
                        child,
                        Some(AgentTurnOrigin::ExternalChannel { .. })
                    ));
                })
                .await;
                assert!(matches!(
                    turn_origin::current(),
                    Some(AgentTurnOrigin::TrustedAutomation { .. })
                ));
            },
        )
        .await;
        assert!(turn_origin::current().is_none());
    })
    .await;
    assert!(CoreContext::scoped().is_none());
}

#[tokio::test]
async fn concurrent_channel_turns_keep_tenant_agent_thread_and_policy_isolated() {
    use crate::agent::turn_origin::{self, AgentTurnOrigin};
    use crate::core::runtime::{ContextOverlay, DomainSet};

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let run = |label: &'static str, barrier: Arc<tokio::sync::Barrier>| async move {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().to_path_buf();
        let config = crate::config::Config {
            workspace_dir: workspace.clone(),
            action_dir: workspace.clone(),
            config_path: workspace.join("config.toml"),
            ..Default::default()
        };
        let policy = Arc::new(crate::security::SecurityPolicy::from_config(
            &config.autonomy,
            &workspace,
            &workspace,
        ));
        let context = CoreContext::for_test(DomainSet::none(), Some(workspace.clone()))
            .derive_with(
                ContextOverlay::new(config, DomainSet::full(), Default::default())
                    .session_agent(format!("agent-{label}"))
                    .profile(format!("profile-{label}"))
                    .agent_policy(policy.clone()),
            );
        let thread = format!("channel:{label}");
        let origin = AgentTurnOrigin::ExternalChannel {
            channel: label.into(),
            sender: Some(format!("sender-{label}")),
            sender_name: None,
            reply_target: format!("room-{label}"),
            message_id: format!("message-{label}"),
            history_key: None,
        };
        let turn = context.derive_channel_turn(Some(thread.clone()), Some(origin));
        CoreContext::scope(turn, async move {
            crate::core::runtime::DispatchCtx::ambient()
                .spawn(async move {
                    barrier.wait().await;
                    let child = CoreContext::scoped().unwrap();
                    assert_eq!(child.profile(), Some(format!("profile-{label}").as_str()));
                    assert_eq!(
                        child.session_agent(),
                        Some(format!("agent-{label}").as_str())
                    );
                    assert_eq!(CoreContext::current_channel_thread(), Some(thread));
                    assert_eq!(child.domains(), DomainSet::none());
                    assert_eq!(child.tool_groups(), Default::default());
                    assert_eq!(child.embedder_config().unwrap().workspace_dir, workspace);
                    assert!(Arc::ptr_eq(&child.agent_policy().unwrap(), &policy));
                    assert!(matches!(
                        turn_origin::current(),
                        Some(AgentTurnOrigin::ExternalChannel { channel, sender, .. })
                            if channel == label && sender == Some(format!("sender-{label}"))
                    ));
                })
                .await
                .unwrap();
        })
        .await;
        assert!(context.turn_origin.is_none());
        assert!(context.channel_thread.is_none());
        assert!(CoreContext::scoped().is_none());
        assert!(turn_origin::current().is_none());
    };
    tokio::join!(run("one", barrier.clone()), run("two", barrier));
}

#[tokio::test]
async fn scoped_spawn_does_not_invent_missing_turn_authority() {
    CoreContext::scope(ctx("/tmp/origin-missing"), async {
        let child = crate::core::runtime::DispatchCtx::ambient()
            .spawn(async { crate::agent::turn_origin::current() })
            .await
            .unwrap();
        assert!(child.is_none());
    })
    .await;
    assert!(crate::agent::turn_origin::current().is_none());
}

/// Regression: an embedded agent's turn scopes the agent's own derived context
/// (which carries no origin) inside the turn that the caller labelled. The label
/// must survive that replacement, or the gate sees an unlabelled turn and refuses
/// every external-effect tool.
#[tokio::test]
async fn a_turn_origin_survives_a_change_of_context() {
    let outer = ctx("/tmp/origin-survives-outer");
    let inner = ctx("/tmp/origin-survives-inner");
    let origin = crate::agent::turn_origin::AgentTurnOrigin::TrustedAutomation {
        job_id: "flow".into(),
        source: crate::agent::turn_origin::TrustedAutomationSource::Workflow {
            require_approval: true,
        },
    };
    CoreContext::scope_with_turn_origin(outer, Some(origin), async {
        CoreContext::scope(inner.clone(), async {
            assert!(matches!(
                CoreContext::current_turn_origin(),
                Some(crate::agent::turn_origin::AgentTurnOrigin::TrustedAutomation { .. })
            ));
            // It is the inner context that is scoped (its own state), not the outer.
            assert!(CoreContext::current()
                .unwrap()
                .workspace_dir()
                .unwrap()
                .ends_with("origin-survives-inner"));
        })
        .await;
        CoreContext::sync_scope(inner, || {
            assert!(CoreContext::current_turn_origin().is_some());
        });
    })
    .await;
    assert!(CoreContext::current_turn_origin().is_none());
}

/// An inner context that names its own origin keeps it.
#[tokio::test]
async fn a_contexts_own_origin_wins_over_the_enclosing_turns() {
    let outer = ctx("/tmp/origin-own-outer");
    let inner = ctx("/tmp/origin-own-inner");
    let own = crate::agent::turn_origin::AgentTurnOrigin::Cli;
    let enclosing = crate::agent::turn_origin::AgentTurnOrigin::DirectChat;
    let inner = inner.derive_channel_turn(None, Some(own));
    CoreContext::scope_with_turn_origin(outer, Some(enclosing), async {
        CoreContext::scope(inner, async {
            assert!(matches!(
                CoreContext::current_turn_origin(),
                Some(crate::agent::turn_origin::AgentTurnOrigin::Cli)
            ));
        })
        .await;
    })
    .await;
}
