use super::*;

#[test]
fn each_origin_maps_to_its_thread_and_label() {
    let web = AgentTurnOrigin::WebChat {
        thread_id: "t1".into(),
        client_id: "c".into(),
        request_id: None,
    };
    assert_eq!(
        describe_origin(Some(&web)),
        (Some("t1".into()), Some("web_chat".into()))
    );
    let channel = AgentTurnOrigin::ExternalChannel {
        channel: "telegram".into(),
        sender: None,
        sender_name: None,
        history_key: None,
        reply_target: "r".into(),
        message_id: "m".into(),
    };
    assert_eq!(
        describe_origin(Some(&channel)),
        (None, Some("channel:telegram".into()))
    );
    // A channel conversation's history key is its thread.
    let conversation = AgentTurnOrigin::ExternalChannel {
        channel: "telegram".into(),
        sender: None,
        sender_name: None,
        history_key: Some("telegram:42".into()),
        reply_target: "r".into(),
        message_id: "m".into(),
    };
    assert_eq!(
        describe_origin(Some(&conversation)).0.as_deref(),
        Some("telegram:42")
    );
    let cron = AgentTurnOrigin::TrustedAutomation {
        job_id: "j".into(),
        source: TrustedAutomationSource::Cron,
    };
    assert_eq!(describe_origin(Some(&cron)).1.as_deref(), Some("cron"));
    assert_eq!(
        describe_origin(Some(&AgentTurnOrigin::Cli)).1.as_deref(),
        Some("cli")
    );
    assert_eq!(describe_origin(None), (None, None));
    assert_eq!(
        describe_origin(Some(&AgentTurnOrigin::Unknown)),
        (None, None)
    );
}

#[tokio::test]
async fn ambient_scope_reads_the_turn_and_prefers_the_subagent() {
    let origin = AgentTurnOrigin::WebChat {
        thread_id: "thread-9".into(),
        client_id: "c".into(),
        request_id: None,
    };
    // The origin is read from the turn's `CoreContext`, which `with_origin`
    // binds when there is a context to bind it on.
    let ctx =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None);
    let (parent, child) = {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(ctx);
        crate::agent::turn_origin::with_origin(origin, async {
            async {
                (
                    UsageScope::in_ctx(&scope_ctx, Some("openhuman"), None)
                        .with_definition(Some("orchestrator")),
                    UsageScope::in_ctx(
                        &scope_ctx,
                        Some("openhuman"),
                        Some(("researcher", "task-1")),
                    )
                    .with_definition(Some("orchestrator")),
                )
            }
            .await
        })
        .await
    };
    assert_eq!(parent.thread_id.as_deref(), Some("thread-9"));
    assert_eq!(parent.origin.as_deref(), Some("web_chat"));
    assert_eq!(parent.agent_id.as_deref(), Some("orchestrator"));
    assert_eq!(parent.provider.as_deref(), Some("openhuman"));
    assert_eq!(parent.subagent_task_id, None);
    assert_eq!(child.agent_id.as_deref(), Some("researcher"));
    assert_eq!(child.subagent_task_id.as_deref(), Some("task-1"));
}

#[test]
fn outside_any_turn_and_without_a_provider_the_scope_is_empty() {
    let scope = UsageScope::in_ctx(&DispatchCtx::ambient(), Some(""), None);
    assert!(scope.is_empty(), "{scope:?}");
}

#[tokio::test]
async fn the_explicit_context_names_the_session_agent_not_the_ambient_one() {
    let root =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None);
    let alice = crate::core::runtime::agent_scope::test_agent_context(&root, "alice");
    let bob = crate::core::runtime::agent_scope::test_agent_context(&root, "bob");
    let alice_ctx = DispatchCtx::new(alice.clone());
    let bob_ctx = DispatchCtx::new(bob.clone());

    // Concurrent tasks, each ambient under the other tenant, across an await
    // and into a spawned task: the explicit context decides.
    let a = tokio::spawn(crate::core::runtime::CoreContext::scope(bob, {
        let ctx = alice_ctx.clone();
        async move {
            tokio::task::yield_now().await;
            UsageScope::in_ctx(&ctx, Some("p"), None)
        }
    }));
    let b = tokio::spawn(crate::core::runtime::CoreContext::scope(alice, {
        let ctx = bob_ctx.clone();
        async move {
            tokio::task::yield_now().await;
            UsageScope::in_ctx(&ctx, Some("p"), None)
        }
    }));
    assert_eq!(a.await.unwrap().session_agent.as_deref(), Some("alice"));
    assert_eq!(b.await.unwrap().session_agent.as_deref(), Some("bob"));
}
