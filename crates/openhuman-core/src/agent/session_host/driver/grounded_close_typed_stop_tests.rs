use super::*;

#[test]
fn classified_halt_summary_uses_typed_cause_when_halt_copy_conflicts() {
    use tinyagents_harness::turn_outcome::ToolCallOutcome;

    let outcome = TinyagentsTurnOutcome {
        deferred_terminal: None,
        text: String::new(),
        resolved_route: None,
        history: Vec::new(),
        conversation: Vec::new(),
        model_calls: 1,
        tool_calls: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cost: tinyagents_harness::cost::CostTally::default(),
        early_exit_tool: None,
        hit_cap: false,
        wrap_up_injected: false,
        truncated: false,
        breaker_halt: Some("Stopping after 1 attempt(s): blocked by the security policy".into()),
        failure_cause: Some(tinyagents_harness::no_progress::ClassifiedStopCause {
            class: "out_of_credits".into(),
            operation: "search".into(),
            summary: "Stopping after 1 attempt(s): budget exhausted".into(),
        }),
        tool_outcomes: vec![ToolCallOutcome {
            call_id: "call".into(),
            name: "search".into(),
            arguments: serde_json::Value::Null,
            success: false,
            content: "credit limit reached".into(),
            duration_ms: 0,
        }],
        compaction: None,
    };

    let close = classified_halt_close(&outcome).expect("classified halt");
    assert!(
        close.output.contains("inference credits"),
        "typed out_of_credits cause must win over contradictory prose: {}",
        close.output
    );
    assert!(
        !close.output.contains("security settings"),
        "the note's policy wording must not override the typed cause: {}",
        close.output
    );
}

#[test]
fn classified_halt_summary_uses_typed_class_for_translated_notes() {
    let note = "Interrompo aqui: bloqueado por política de segurança, orçamento esgotado.";
    let cases = [
        ("out_of_credits", "inference credits"),
        ("provider_config", "configured model or provider rejected"),
        ("transient", "temporarily unavailable"),
        ("blocked_by_policy", "security settings"),
    ];

    for (class, expected_copy) in cases {
        let outcome = classified_halt(note, class);
        let close = classified_halt_close(&outcome)
            .unwrap_or_else(|| panic!("typed `{class}` cause must close the turn"));
        assert!(
            close.output.contains(expected_copy),
            "typed `{class}` cause must select its summary copy: {}",
            close.output
        );
        assert!(
            !close.output.contains("security settings") || class == "blocked_by_policy",
            "translated prose must not override typed `{class}` cause: {}",
            close.output
        );
        assert_eq!(close.usage.model_calls, 0);
    }
}

#[test]
fn unrecognized_typed_class_does_not_fall_back_to_english_halt_phrases() {
    let outcome = classified_halt(
        "Stopping: the action is blocked by the security policy.",
        "future_failure_class",
    );
    let close = classified_halt_close(&outcome).expect("typed halt");

    assert!(
        close
            .output
            .contains("my tool calls were not getting anywhere"),
        "unknown typed classes use generic copy instead of parsing halt prose: {}",
        close.output
    );
    assert!(
        !close.output.contains("security settings"),
        "English halt wording cannot override a typed class: {}",
        close.output
    );
}

#[test]
fn classified_halt_maps_all_typed_stop_categories_and_aliases() {
    let cases = [
        ("transient", "temporarily unavailable"),
        (
            "uncertain_side_effect",
            "could not confirm whether it went through",
        ),
        ("authentication", "rejected my credentials"),
        ("permission", "don't have permission"),
        ("policy", "security settings"),
        ("missing_program", "program or feature"),
        ("not_found", "could not be found"),
        ("invalid_calls", "requests as invalid"),
        ("unavailable", "not available right now"),
        ("site_refused", "website kept refusing"),
        ("repeated_failure", "same step kept failing"),
        ("varied_failures", "different tool calls in a row failed"),
        ("repeating_step", "repeating the same step"),
        ("out_of_credits", "inference credits"),
        ("provider_config", "configured model or provider rejected"),
        ("missing_connection", "isn't connected"),
        (
            "future_failure_class",
            "my tool calls were not getting anywhere",
        ),
        ("blocked_by_policy", "security settings"),
        ("unsupported", "program or feature"),
        ("missing_app", "program or feature"),
        ("missing_window", "could not be found"),
        ("validation", "requests as invalid"),
        ("invalid_arguments", "requests as invalid"),
        ("service_refused", "not available right now"),
    ];

    for (class, expected_copy) in cases {
        let outcome = classified_halt(
            "Interrompo: bloqueado por segurança; o texto foi traduzido.",
            class,
        );
        let close = classified_halt_close(&outcome)
            .unwrap_or_else(|| panic!("typed `{class}` cause must close the turn"));
        assert!(
            close.output.contains(expected_copy),
            "typed `{class}` cause must use its stop-category copy: {}",
            close.output
        );
        assert_eq!(
            close.usage.model_calls, 0,
            "typed `{class}` cause closes without another model call"
        );
        let stop = crate::agent::session_host::stop_policy::classify_stop_with_cause(
            outcome.breaker_halt.as_deref(),
            outcome.failure_cause.clone(),
            false,
            false,
        )
        .expect("typed stop remains projected");
        assert_eq!(
            stop.operation.as_deref(),
            Some("search"),
            "typed `{class}` cause keeps its operation"
        );
    }
}

#[tokio::test]
async fn typed_cause_without_halt_note_still_closes_and_keeps_operation() {
    let mut outcome = classified_halt("", "provider_config");
    outcome.breaker_halt = None;
    outcome.text = "A partial reply that should be replaced by the classified halt summary.".into();
    outcome.tool_outcomes.clear();

    let scripted = Arc::new(tinyagents_harness::testkit::ScriptedModel::new(vec![]));
    let source = TurnModelSource::from_model(scripted.clone());
    let close = close_if_needed(
        &source,
        "host-selected-model",
        0.0,
        None,
        &tinytools_agent::dialect::NativeDialect,
        &[],
        "search",
        &outcome,
        None,
        None,
    )
    .await
    .expect("typed cause closes without a note");
    assert!(
        close
            .output
            .contains("configured model or provider rejected"),
        "summary uses typed provider_config cause even without a note: {}",
        close.output
    );
    assert_eq!(close.usage.model_calls, 0);
    assert!(
        scripted.requests().is_empty(),
        "typed cause closes deterministically without calling the provider"
    );

    let stop = crate::agent::session_host::stop_policy::classify_stop_with_cause(
        None,
        outcome.failure_cause,
        false,
        false,
    )
    .expect("typed cause is still projected to the session stop");
    assert_eq!(
        stop.failure_class.as_ref().map(|class| class.as_str()),
        Some("provider_config")
    );
    assert_eq!(stop.operation.as_deref(), Some("search"));
}
