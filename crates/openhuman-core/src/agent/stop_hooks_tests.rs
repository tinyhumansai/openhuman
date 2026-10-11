use super::*;

#[tokio::test]
async fn scoped_tool_limits_narrow_restore_and_keep_zero() {
    assert_eq!(tool_call_limit(10), 80);
    with_tool_call_limit(Some(4), async {
        assert_eq!(tool_call_limit(10), 4);
        with_tool_call_limit(Some(12), async {
            assert_eq!(tool_call_limit(10), 4);
        })
        .await;
        with_tool_call_limit(Some(0), async {
            assert_eq!(tool_call_limit(10), 0);
        })
        .await;
        assert_eq!(tool_call_limit(10), 4);
        with_tool_call_limit(None, async {
            assert_eq!(tool_call_limit(10), 4);
        })
        .await;
    })
    .await;
    assert_eq!(tool_call_limit(10), 80);
    with_tool_call_limit(Some(100), async {
        assert_eq!(tool_call_limit(1), 8);
    })
    .await;
}

#[tokio::test]
async fn concurrent_turns_do_not_share_tool_limits() {
    let (a, b) = tokio::join!(
        with_tool_call_limit(Some(2), async {
            tokio::task::yield_now().await;
            tool_call_limit(10)
        }),
        with_tool_call_limit(Some(7), async {
            tokio::task::yield_now().await;
            tool_call_limit(10)
        })
    );
    assert_eq!((a, b), (2, 7));
    assert_eq!(tool_call_limit(10), 80);
}

#[tokio::test]
async fn cancelled_scope_restores_the_callers_tool_limit() {
    let mut scoped = Box::pin(with_tool_call_limit(Some(0), async {
        assert_eq!(tool_call_limit(10), 0);
        std::future::pending::<()>().await;
    }));
    assert!(futures::poll!(&mut scoped).is_pending());
    drop(scoped);
    assert_eq!(tool_call_limit(10), 80);
}

#[tokio::test]
async fn scoped_stop_hooks_are_visible_to_run_contexts() {
    let hook: Arc<dyn StopHook<crate::agent::cost::TurnCost>> = Arc::new(BudgetStopHook::new(1.0));
    assert!(current_stop_hooks().is_empty());
    with_stop_hooks(vec![Arc::clone(&hook)], async {
        let active = current_stop_hooks();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name(), "budget");
        assert_eq!(
            crate::agent::tinyagents::host::OpenHumanRunContext::new()
                .stop_hooks
                .len(),
            1
        );
    })
    .await;
    assert!(current_stop_hooks().is_empty());
}

#[tokio::test]
async fn budget_stop_hook_fails_closed_and_stops_at_cap() {
    let hook = BudgetStopHook::new(1.0);
    let cost = TurnCost {
        cost: crate::agent::cost::CostTally {
            known_usd: 1.0,
            source: crate::agent::cost::CostSource::Charged,
        },
        ..TurnCost::default()
    };
    let state = TurnState {
        iteration: 1,
        max_iterations: 10,
        cost: &cost,
        model: "test",
    };
    assert!(matches!(
        hook.check(&state).await,
        StopDecision::Stop { .. }
    ));
    assert!(matches!(
        BudgetStopHook::new(f64::NAN).check(&state).await,
        StopDecision::Stop { .. }
    ));
}
