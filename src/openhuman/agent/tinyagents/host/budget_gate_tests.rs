use super::*;
use std::time::Duration;

fn gate(compression: AgentTokenjuiceCompression) -> OpenHumanBudgetGate {
    OpenHumanBudgetGate::with_compression(Arc::new(Config::default()), compression)
}

fn crowded() -> ContextState {
    ContextState {
        message_count: 40,
        prompt_tokens: 95_000,
        context_window_tokens: Some(100_000),
        iterations: 12,
    }
}

#[test]
fn seeds_the_attributed_model_from_config() {
    let mut config = Config::default();
    config.default_model = Some("pinned-model".into());
    let gate = OpenHumanBudgetGate::new(Arc::new(config));
    assert_eq!(gate.attributed_model(), "pinned-model");

    let mut unpinned = Config::default();
    unpinned.default_model = None;
    let gate = OpenHumanBudgetGate::new(Arc::new(unpinned));
    assert_eq!(gate.attributed_model(), DEFAULT_MODEL);
}

#[tokio::test]
async fn acquire_stamps_the_model_that_record_will_attribute() {
    // Mismatch (1) in the module docs: `Usage` has no model, so `acquire`
    // is the only place the attribution can come from.
    let gate = gate(AgentTokenjuiceCompression::Auto);
    gate.acquire(&CallEstimate::new("some/model", 10, 5))
        .await
        .expect("no tracker in tests, so nothing can refuse");
    assert_eq!(gate.attributed_model(), "some/model");
}

#[tokio::test]
async fn an_empty_model_does_not_erase_the_attribution() {
    let mut config = Config::default();
    config.default_model = Some("pinned-model".into());
    let gate = OpenHumanBudgetGate::new(Arc::new(config));
    gate.acquire(&CallEstimate::new("   ", 1, 1))
        .await
        .expect("grants");
    assert_eq!(gate.attributed_model(), "pinned-model");
}

#[tokio::test]
async fn the_permit_reserves_the_estimated_total() {
    let gate = gate(AgentTokenjuiceCompression::Auto);
    let permit = gate
        .acquire(&CallEstimate::new("m", 100, 20))
        .await
        .expect("grants");
    assert_eq!(permit.reserved_tokens(), Some(120));
    assert!(permit.id().is_some(), "grants are correlatable");
}

/// An interactive gate must never enter the background scheduler.
///
/// The gate's `Paused` arm polls until background AI is re-enabled, so a
/// user-initiated turn that queued there would hang until the turn timeout
/// for anyone signed out on a local/BYOK model, or who merely paused
/// background AI. The timeout turns that stall into a failure rather than a
/// hung test.
#[tokio::test]
async fn an_interactive_gate_does_not_queue_behind_the_background_scheduler() {
    let gate = gate(AgentTokenjuiceCompression::Auto);
    assert!(!gate.background, "interactive is the default");

    let permit = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        gate.acquire(&CallEstimate::new("m", 100, 20)),
    )
    .await
    .expect("an interactive acquire must not wait on the background gate")
    .expect("grants");

    assert!(permit.id().is_some(), "grants stay correlatable");
    assert_eq!(permit.reserved_tokens(), Some(120));
}

#[tokio::test]
async fn dropping_the_crate_permit_releases_the_scheduler_permit() {
    // The global LLM semaphore has a single slot, so a leaked `LlmPermit`
    // makes the second acquire hang forever. The timeout turns that leak
    // into a failure instead of a hung test — this is the regression this
    // whole adapter is most likely to break.
    //
    // Must be a *background* gate: an interactive one skips the scheduler
    // entirely, so this would pass without ever exercising the release the
    // test exists to prove.
    let gate = gate(AgentTokenjuiceCompression::Auto).as_background_work();
    for round in 0..3 {
        let permit = tokio::time::timeout(
            Duration::from_secs(5),
            gate.acquire(&CallEstimate::new("m", 1, 1)),
        )
        .await
        .unwrap_or_else(|_| panic!("round {round} blocked: the previous permit leaked"))
        .expect("grants");
        drop(permit);
    }
}

#[tokio::test]
async fn explicit_release_returns_capacity_before_end_of_scope() {
    // Background, for the same reason as the test above.
    let gate = gate(AgentTokenjuiceCompression::Auto).as_background_work();
    let first = gate
        .acquire(&CallEstimate::new("m", 1, 1))
        .await
        .expect("grants");
    first.release();
    tokio::time::timeout(
        Duration::from_secs(5),
        gate.acquire(&CallEstimate::new("m", 1, 1)),
    )
    .await
    .expect("release freed the slot immediately")
    .expect("grants");
}

#[tokio::test]
async fn recording_usage_is_a_soft_no_op_without_a_tracker() {
    // `cost::try_global()` is `None` in unit tests. Recording must still
    // succeed — the trait says `record` is called even for failed calls,
    // and a metering hiccup must never fail a turn.
    let gate = gate(AgentTokenjuiceCompression::Auto);
    gate.record(&Usage::new(1_000, 250))
        .await
        .expect("recording never fails the turn");
}

#[test]
fn a_healthy_budget_declines_to_ask_for_compression() {
    // Union semantics: `None` is "not asking", not "do not compress". A
    // full context with no budget pressure must still yield `None` here so
    // the crate's own SummarizationPolicy stays the authority on the
    // window.
    let gate = gate(AgentTokenjuiceCompression::Full);
    assert_eq!(gate.compression_hint(&crowded()), CompressionHint::None);
}

#[test]
fn budget_warning_asks_softly_and_escalates_only_when_context_is_crowded() {
    let gate = gate(AgentTokenjuiceCompression::Full);
    gate.pressure.store(PRESSURE_WARNING, Ordering::Relaxed);

    let roomy = ContextState {
        message_count: 4,
        prompt_tokens: 1_000,
        context_window_tokens: Some(100_000),
        iterations: 1,
    };
    assert_eq!(gate.compression_hint(&roomy), CompressionHint::Soft);
    assert_eq!(gate.compression_hint(&crowded()), CompressionHint::Hard);

    // An unknown window must not escalate — `utilization()` is `None`
    // there, and "unknown" is not "full".
    let unknown_window = ContextState {
        message_count: 4,
        prompt_tokens: 999_999,
        context_window_tokens: None,
        iterations: 1,
    };
    assert_eq!(
        gate.compression_hint(&unknown_window),
        CompressionHint::Soft
    );
}

#[test]
fn an_exceeded_budget_asks_hard_regardless_of_context() {
    let gate = gate(AgentTokenjuiceCompression::Full);
    gate.pressure.store(PRESSURE_EXCEEDED, Ordering::Relaxed);
    assert_eq!(
        gate.compression_hint(&ContextState::default()),
        CompressionHint::Hard
    );
}

#[test]
fn the_tokenjuice_profile_caps_the_hint_but_never_raises_it() {
    for (profile, warning, exceeded) in [
        (
            AgentTokenjuiceCompression::Auto,
            CompressionHint::Soft,
            CompressionHint::Hard,
        ),
        (
            AgentTokenjuiceCompression::Full,
            CompressionHint::Soft,
            CompressionHint::Hard,
        ),
        // Light tolerates non-lossy reductions only.
        (
            AgentTokenjuiceCompression::Light,
            CompressionHint::Soft,
            CompressionHint::Soft,
        ),
        // Off opts the agent out of TokenJuice entirely.
        (
            AgentTokenjuiceCompression::Off,
            CompressionHint::None,
            CompressionHint::None,
        ),
    ] {
        let gate = gate(profile);
        gate.pressure.store(PRESSURE_WARNING, Ordering::Relaxed);
        assert_eq!(
            gate.compression_hint(&ContextState::default()),
            warning,
            "warning under {}",
            profile.as_str()
        );
        gate.pressure.store(PRESSURE_EXCEEDED, Ordering::Relaxed);
        assert_eq!(
            gate.compression_hint(&ContextState::default()),
            exceeded,
            "exceeded under {}",
            profile.as_str()
        );
    }
}

#[test]
fn refreshing_pressure_without_a_tracker_has_no_opinion() {
    let gate = gate(AgentTokenjuiceCompression::Full);
    gate.pressure.store(PRESSURE_WARNING, Ordering::Relaxed);
    assert!(gate.refresh_pressure(1.0).is_none());
    // The uninitialised tracker must not clear a previously-cached
    // pressure — absence of a reading is not a reading of "fine".
    assert_eq!(gate.pressure.load(Ordering::Relaxed), PRESSURE_WARNING);
}

#[tokio::test]
async fn is_usable_as_a_trait_object() {
    // Pins object safety: the harness stores this as `Arc<dyn BudgetGate>`.
    let gate: Arc<dyn BudgetGate> = Arc::new(gate(AgentTokenjuiceCompression::Auto));
    let permit = gate
        .acquire(&CallEstimate::new("m", 1, 1).with_agent("lead"))
        .await
        .expect("grants");
    drop(permit);
    assert_eq!(
        gate.compression_hint(&ContextState::default()),
        CompressionHint::None
    );
}
