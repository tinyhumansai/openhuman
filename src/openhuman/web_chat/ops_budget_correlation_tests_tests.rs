use super::*;

#[test]
fn classify_budget_correlation_matrix() {
    // A budget error always records + surfaces budget copy, regardless of
    // the other flags.
    assert_eq!(
        classify_budget_correlation(true, false, false),
        BudgetCorrelation::BudgetExhausted
    );
    assert_eq!(
        classify_budget_correlation(true, true, true),
        BudgetCorrelation::BudgetExhausted
    );
    // Empty response only upgrades when a fresh signal is present.
    assert_eq!(
        classify_budget_correlation(false, true, true),
        BudgetCorrelation::UpgradeEmptyToBudget
    );
    assert_eq!(
        classify_budget_correlation(false, true, false),
        BudgetCorrelation::PassThrough
    );
    // A fresh signal without an empty response does not invent an upgrade.
    assert_eq!(
        classify_budget_correlation(false, false, true),
        BudgetCorrelation::PassThrough
    );
    // Neither flag: untouched.
    assert_eq!(
        classify_budget_correlation(false, false, false),
        BudgetCorrelation::PassThrough
    );
}

#[test]
fn budget_signal_is_fresh_boundary() {
    let ttl = Duration::from_secs(300);
    assert!(budget_signal_is_fresh(Duration::from_secs(0), ttl));
    assert!(budget_signal_is_fresh(Duration::from_secs(299), ttl));
    assert!(budget_signal_is_fresh(ttl, ttl)); // inclusive at the boundary
    assert!(!budget_signal_is_fresh(Duration::from_secs(301), ttl));
}

const BINDING: &str = "openhuman-managed";

#[tokio::test]
async fn record_then_fresh_then_clear() {
    let thread = "budget-corr-test-lifecycle";
    clear_budget_signal(thread).await; // isolate from other tests
    assert!(!has_fresh_budget_signal(thread, BINDING).await);

    record_budget_signal(thread, BINDING).await;
    assert!(has_fresh_budget_signal(thread, BINDING).await);

    clear_budget_signal(thread).await;
    assert!(!has_fresh_budget_signal(thread, BINDING).await);
}

#[tokio::test]
async fn stale_signal_is_not_fresh_and_is_evicted() {
    let thread = "budget-corr-test-stale";
    // Seed a signal older than the TTL.
    record_budget_signal_aged(thread, BINDING, BUDGET_SIGNAL_TTL + Duration::from_secs(1)).await;
    // Reads as not-fresh and self-evicts.
    assert!(!has_fresh_budget_signal(thread, BINDING).await);
    // Confirm eviction: still not fresh, and a later in-window seed works.
    assert!(!has_fresh_budget_signal(thread, BINDING).await);
    record_budget_signal_aged(thread, BINDING, Duration::from_secs(1)).await;
    assert!(has_fresh_budget_signal(thread, BINDING).await);
    clear_budget_signal(thread).await;
}

#[tokio::test]
async fn signal_does_not_cross_provider_bindings() {
    let thread = "budget-corr-test-binding";
    clear_budget_signal(thread).await;
    // Budget hit on the managed route.
    record_budget_signal(thread, "openhuman-managed").await;
    // A turn re-routed to a different (BYO/local) provider must NOT inherit
    // the managed exhaustion — its empty response is unrelated.
    assert!(!has_fresh_budget_signal(thread, "byo-deepseek").await);
    // The same managed binding still reads fresh (mismatch read above
    // evicted it, so re-record to prove the same-binding path).
    record_budget_signal(thread, "openhuman-managed").await;
    assert!(has_fresh_budget_signal(thread, "openhuman-managed").await);
    clear_budget_signal(thread).await;
}

#[tokio::test]
async fn record_prunes_other_threads_stale_entries() {
    let abandoned = "budget-corr-test-abandoned";
    let active = "budget-corr-test-active";
    // An abandoned thread leaves a stale entry behind...
    record_budget_signal_aged(
        abandoned,
        BINDING,
        BUDGET_SIGNAL_TTL + Duration::from_secs(1),
    )
    .await;
    // ...which a later budget event on a DIFFERENT thread sweeps away.
    record_budget_signal(active, BINDING).await;
    {
        let signals = THREAD_BUDGET_SIGNALS.lock().await;
        assert!(
            !signals.contains_key(abandoned),
            "stale entry should be pruned"
        );
        assert!(signals.contains_key(active));
    }
    clear_budget_signal(active).await;
}
