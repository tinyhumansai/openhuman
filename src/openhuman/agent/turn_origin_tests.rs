use super::*;

#[tokio::test]
async fn with_origin_scopes_correctly_and_unscopes_on_exit() {
    // Outside any scope: current() returns None.
    assert!(current().is_none());

    let observed = with_origin(AgentTurnOrigin::Cli, async {
        // Inside the scope: current() returns the scoped origin.
        current()
    })
    .await;
    assert!(matches!(observed, Some(AgentTurnOrigin::Cli)));

    // After the scope exits, current() is None again.
    assert!(current().is_none());
}

/// The defect this helper fixes: a `tokio::spawn`ed delegation loses the
/// task-local, so the gate saw `Unknown` and refused every shell/exec tool.
/// Capturing on the parent and re-scoping inside the spawned task restores
/// the label.
#[tokio::test]
async fn inherited_origin_crosses_a_spawn_boundary() {
    let observed = with_origin(AgentTurnOrigin::Cli, async {
        // Capture happens on the still-scoped parent task.
        let captured = capture();
        tokio::spawn(async move {
            // Without the re-scope this is `None` (task-locals don't cross
            // `tokio::spawn`).
            with_inherited_origin(captured, async { current() }).await
        })
        .await
        .expect("spawned task panicked")
    })
    .await;
    assert!(
        matches!(observed, Some(AgentTurnOrigin::Cli)),
        "expected the parent's Cli origin to be inherited, got {observed:?}"
    );
}

/// Fail-closed is preserved: an unlabelled parent produces an unlabelled
/// child. The helper must never fabricate an origin.
#[tokio::test]
async fn inherited_origin_stays_unlabelled_without_an_outer_scope() {
    let captured = capture();
    assert!(captured.is_none(), "test precondition: no ambient scope");

    let observed =
        tokio::spawn(async move { with_inherited_origin(captured, async { current() }).await })
            .await
            .expect("spawned task panicked");

    assert!(
        observed.is_none(),
        "unlabelled parent must stay unlabelled, got {observed:?}"
    );
}

/// A remote-untrusted origin is inherited *as itself* — delegation is not a
/// privilege-escalation primitive, so no upgrade to `Cli` may happen.
#[tokio::test]
async fn inherited_origin_preserves_a_non_cli_origin_verbatim() {
    let observed = with_origin(
        AgentTurnOrigin::ExternalChannel {
            channel: "telegram".into(),
            sender: Some("u-42".into()),
            reply_target: "chat-7".into(),
            message_id: "m-9".into(),
        },
        async {
            let captured = capture();
            tokio::spawn(async move { with_inherited_origin(captured, async { current() }).await })
                .await
                .expect("spawned task panicked")
        },
    )
    .await;

    match observed {
        Some(AgentTurnOrigin::ExternalChannel {
            channel,
            sender,
            reply_target,
            message_id,
        }) => {
            assert_eq!(channel, "telegram");
            assert_eq!(sender.as_deref(), Some("u-42"));
            assert_eq!(reply_target, "chat-7");
            assert_eq!(message_id, "m-9");
        }
        other => panic!("expected ExternalChannel inherited verbatim, got {other:?}"),
    }
}

/// Regression: a detached sub-agent (`spawn_async_subagent`, the
/// orchestration spawn task) starts on a fresh task, and without explicit
/// propagation its tools reach the approval gate as `Unknown` and every
/// external-effect call is refused — the parent's label silently lost at
/// the `tokio::spawn` boundary.
#[tokio::test]
async fn propagate_carries_the_origin_across_a_spawn() {
    let observed = with_origin(
        AgentTurnOrigin::TrustedAutomation {
            job_id: "run-1".to_string(),
            source: TrustedAutomationSource::Workflow {
                require_approval: false,
            },
        },
        async {
            tokio::spawn(propagate(async { current() }))
                .await
                .expect("spawned task panicked")
        },
    )
    .await;
    assert!(matches!(
        observed,
        Some(AgentTurnOrigin::TrustedAutomation {
            source: TrustedAutomationSource::Workflow {
                require_approval: false
            },
            ..
        })
    ));
}

/// Without propagation the same spawn loses the label — the behaviour the
/// helper above exists to fix, pinned so a future refactor cannot quietly
/// reintroduce it by dropping the wrapper.
#[tokio::test]
async fn a_bare_spawn_loses_the_origin() {
    let observed = with_origin(AgentTurnOrigin::Cli, async {
        tokio::spawn(async { current() })
            .await
            .expect("spawned task panicked")
    })
    .await;
    assert!(observed.is_none());
}

/// Fail-closed is preserved: propagation carries a decision, it does not
/// invent one. An unlabelled parent still yields an unlabelled child.
#[tokio::test]
async fn propagate_does_not_manufacture_an_origin() {
    let observed = tokio::spawn(propagate(async { current() }))
        .await
        .expect("spawned task panicked");
    assert!(observed.is_none());
}

/// A hosted effect-executor spawn site
/// (#5508 / #5499): the device-tool bridge fires the local sub-agent from a
/// bare `tokio::spawn` where there is **no ambient turn** to inherit — unlike
/// the four sites PR #5465 fixed with `capture`/`propagate`, `capture()` here
/// is `None`, so `with_inherited_origin` would leave the task `Unknown` and
/// the gate would refuse every external-effect tool (`cron_add`, shell, …).
/// The fix scopes an **explicit** `Cli` origin on the spawned task instead
/// (device automation past the Master-chat gate is trusted, turn-less
/// internal dispatch). This pins that shape: nothing to inherit on the
/// parent, a real `Cli` origin observed across the spawn boundary.
#[tokio::test]
async fn explicit_cli_origin_survives_a_turnless_spawn() {
    // No outer scope — exactly the device-tool bridge's situation.
    assert!(
        capture().is_none(),
        "precondition: the effect_executor spawn has no ambient origin to inherit"
    );

    let observed = tokio::spawn(with_origin(AgentTurnOrigin::Cli, async { current() }))
        .await
        .expect("spawned task panicked");

    assert!(
        matches!(observed, Some(AgentTurnOrigin::Cli)),
        "the explicitly-scoped Cli origin must be visible on the spawned task, got {observed:?}"
    );
}

// ── spawn / spawn_unlabelled ────────────────────────────────────────

/// The helper carries the label with no separate capture step, so a call
/// site cannot forget one.
#[tokio::test]
async fn spawn_carries_the_origin_onto_the_new_task() {
    let observed = with_origin(AgentTurnOrigin::Cli, async {
        spawn(async { current() })
            .await
            .expect("spawned task panicked")
    })
    .await;

    assert!(
        matches!(observed, Some(AgentTurnOrigin::Cli)),
        "expected the parent's Cli origin on the spawned task, got {observed:?}"
    );
}

/// **The reason this helper exists.**
///
/// `propagate` reads the origin when it is *called*, so it has to be called
/// on the spawning task. Both forms below compile and neither warns, but
/// evaluating `propagate` inside the spawned future captures nothing — the
/// task-local is already gone — and the child silently runs unlabelled.
/// Every external-effect tool it calls is then refused by the approval gate.
///
/// Routing through `spawn` makes that ordering unexpressible.
#[tokio::test]
async fn spawn_is_immune_to_capturing_inside_the_spawned_task() {
    let (wrong, right) = with_origin(AgentTurnOrigin::Cli, async {
        // The mistake: `propagate` evaluated on the *new* task.
        let wrong = tokio::spawn(async move { propagate(async { current() }).await })
            .await
            .expect("spawned task panicked");

        // The helper: capture happens before the spawn, inside `spawn`.
        let right = spawn(async { current() })
            .await
            .expect("spawned task panicked");

        (wrong, right)
    })
    .await;

    assert!(
        wrong.is_none(),
        "precondition: capturing inside the spawned task loses the origin — \
         this is the hazard `spawn` removes, got {wrong:?}"
    );
    assert!(
        matches!(right, Some(AgentTurnOrigin::Cli)),
        "the helper must keep the label regardless of how the call site is \
         written, got {right:?}"
    );
}

/// Fail-closed: no ambient origin in, no origin out. The helper carries a
/// decision, it never invents one.
#[tokio::test]
async fn spawn_does_not_manufacture_an_origin() {
    assert!(current().is_none(), "test precondition: no ambient scope");

    let observed = spawn(async { current() })
        .await
        .expect("spawned task panicked");

    assert!(
        observed.is_none(),
        "an unlabelled parent must produce an unlabelled child, got {observed:?}"
    );
}

/// A remote-untrusted origin crosses as itself. Delegation must not be a
/// privilege-escalation primitive, so no upgrade to `Cli` may happen.
#[tokio::test]
async fn spawn_preserves_an_untrusted_origin_verbatim() {
    let observed = with_origin(
        AgentTurnOrigin::ExternalChannel {
            channel: "telegram".into(),
            sender: Some("u-42".into()),
            reply_target: "chat-7".into(),
            message_id: "m-9".into(),
        },
        async {
            spawn(async { current() })
                .await
                .expect("spawned task panicked")
        },
    )
    .await;

    match observed {
        Some(AgentTurnOrigin::ExternalChannel {
            channel, sender, ..
        }) => {
            assert_eq!(channel, "telegram");
            assert_eq!(sender.as_deref(), Some("u-42"));
        }
        other => panic!("expected ExternalChannel carried verbatim, got {other:?}"),
    }
}

/// The explicit opt-out drops the label, which is its whole purpose — the
/// value is that the call site says so by name instead of looking identical
/// to a site that forgot.
#[tokio::test]
async fn spawn_unlabelled_drops_the_origin_on_purpose() {
    let observed = with_origin(AgentTurnOrigin::Cli, async {
        spawn_unlabelled("test: not a continuation of this turn", async { current() })
            .await
            .expect("spawned task panicked")
    })
    .await;

    assert!(
        observed.is_none(),
        "spawn_unlabelled must not carry the caller's origin, got {observed:?}"
    );
}

#[tokio::test]
async fn current_returns_none_outside_scope() {
    assert!(current().is_none());
}

#[tokio::test]
async fn current_returns_inner_origin_on_nested_scope() {
    let observed = with_origin(
        AgentTurnOrigin::WebChat {
            thread_id: "outer".into(),
            client_id: "c-outer".into(),
            request_id: Some("req-outer".into()),
        },
        async {
            with_origin(
                AgentTurnOrigin::TrustedAutomation {
                    job_id: "j-1".into(),
                    source: TrustedAutomationSource::Cron,
                },
                async { current() },
            )
            .await
        },
    )
    .await;
    match observed {
        Some(AgentTurnOrigin::TrustedAutomation { job_id, source }) => {
            assert_eq!(job_id, "j-1");
            assert_eq!(source, TrustedAutomationSource::Cron);
        }
        other => panic!("expected inner TrustedAutomation, got {other:?}"),
    }
}
