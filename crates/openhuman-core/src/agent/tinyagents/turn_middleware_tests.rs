use super::*;

#[test]
fn a_poisoned_halt_summary_does_not_abort_stop_reporting() {
    let slot: HaltSummarySlot = Arc::new(std::sync::Mutex::new(None));
    let poisoned = Arc::clone(&slot);
    assert!(std::thread::spawn(move || {
        let _guard = poisoned.lock().unwrap();
        panic!("synthetic prior writer failure");
    })
    .join()
    .is_err());

    record_stop_summary(&slot, 1, "limit reached");

    assert_eq!(
        slot.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_deref(),
        Some("Stopping after 1 model call(s): limit reached")
    );
}

#[test]
fn a_resolved_routes_pricing_is_zero_for_free_models_and_the_list_price_otherwise() {
    let free = generation_pricing("openrouter/some-model:free").expect("free models are priced");
    assert_eq!(free.input_per_mtok_usd, Some(0.0));
    assert_eq!(free.cached_input_per_mtok_usd, Some(0.0));
    assert_eq!(free.output_per_mtok_usd, Some(0.0));

    let listed = tinyagents_registry::pricing::KNOWN_MODEL_PRICING
        .iter()
        .find(|price| !crate::agent::cost::is_free_model(price.model_id))
        .expect("precondition: the pricing catalog lists a paid model");
    let priced = generation_pricing(listed.model_id).expect("a catalogued model is priced");
    assert_eq!(priced.input_per_mtok_usd, Some(listed.input_per_mtok_usd));
    assert_eq!(
        priced.cached_input_per_mtok_usd,
        Some(listed.cached_input_per_mtok_usd)
    );
    assert_eq!(priced.output_per_mtok_usd, Some(listed.output_per_mtok_usd));

    assert!(
        generation_pricing("no-such-model-anywhere").is_none(),
        "an unknown model is left unpriced rather than given a default rate"
    );
}
