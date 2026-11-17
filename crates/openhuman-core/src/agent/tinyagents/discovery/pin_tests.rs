use tinyagents_harness::tool::discover::ToolDiscoveryPolicy;

use super::*;

fn config(promote: bool, compact: bool, collapse: bool, repair: bool) -> ToolSearchConfig {
    ToolSearchConfig {
        promote_found_tools: promote,
        compact_search_results: compact,
        collapse_search_results: collapse,
        repair_tool_call_wrapper: repair,
        ..ToolSearchConfig::default()
    }
}

#[test]
fn today_is_the_harness_default_and_the_config_default() {
    let policy = ToolDiscoveryPolicy::default();
    assert_eq!(
        DiscoveryOptions::TODAY.overrides(),
        DiscoveryOverrides {
            promote_found_tools: Some(policy.promote_found_tools),
            compact_search_results: Some(policy.compact_search_results),
            collapse_search_results: Some(policy.collapse_search_results),
            repair_tool_call_wrapper: Some(policy.repair_tool_call_wrapper),
        }
    );
    assert_eq!(
        DiscoveryOptions::from_config(&ToolSearchConfig::default()),
        DiscoveryOptions::TODAY
    );
}

#[test]
fn config_options_map_onto_the_harness_policy_and_back() {
    let wanted = DiscoveryOptions {
        promote_found_tools: false,
        compact_search_results: true,
        collapse_search_results: true,
        repair_tool_call_wrapper: false,
    };
    let mut settings = ToolSearchConfig::default();
    wanted.apply_to(&mut settings);
    assert_eq!(settings, config(false, true, true, false));
    assert_eq!(DiscoveryOptions::from_config(&settings), wanted);

    let policy = ToolDiscoveryPolicy::default().with_overrides(&wanted.overrides());
    assert!(!policy.promote_found_tools);
    assert!(policy.compact_search_results);
    assert!(policy.collapse_search_results);
    assert!(!policy.repair_tool_call_wrapper);
}

#[test]
fn a_new_thread_takes_the_config_and_records_only_a_change_from_today() {
    let changed = resolve(&config(false, false, true, true), &[], true);
    assert_eq!(
        changed.effective,
        DiscoveryOptions::from_config(&config(false, false, true, true))
    );
    assert_eq!(changed.record, Some(changed.effective));

    let unchanged = resolve(&ToolSearchConfig::default(), &[], true);
    assert_eq!(unchanged.effective, DiscoveryOptions::TODAY);
    assert_eq!(unchanged.record, None);
}

#[test]
fn a_resumed_thread_keeps_what_it_recorded_whatever_the_config_says_now() {
    let started_with = DiscoveryOptions {
        promote_found_tools: false,
        collapse_search_results: true,
        ..DiscoveryOptions::TODAY
    };
    let mut rows = vec![
        TranscriptMessage::system("prompt"),
        TranscriptMessage::new("user", "hi"),
    ];
    assert!(started_with.record_on(&mut rows, &[true, true]));

    // A later config that asks for something else changes nothing.
    let now = config(true, true, false, false);
    let resumed = resolve(&now, &rows, false);
    assert_eq!(resumed.effective, started_with);
    assert_eq!(resumed.record, None);
}

#[test]
fn a_thread_that_recorded_nothing_keeps_todays_behaviour_whatever_the_config_says() {
    let rows = vec![TranscriptMessage::system("prompt")];
    let resumed = resolve(&config(false, true, true, false), &rows, false);
    assert_eq!(resumed.effective, DiscoveryOptions::TODAY);
    assert_eq!(resumed.record, None);
}

#[test]
fn recording_touches_only_the_first_fresh_row_and_survives_serialization() {
    let options = DiscoveryOptions {
        promote_found_tools: false,
        ..DiscoveryOptions::TODAY
    };
    let mut rows = vec![
        TranscriptMessage::new("user", "imported"),
        TranscriptMessage::system("prompt"),
        TranscriptMessage::new("user", "hi"),
    ];
    let before = rows[0].clone();
    assert!(options.record_on(&mut rows, &[false, true, true]));
    assert_eq!(rows[0], before, "a row already stored is never rewritten");
    assert!(rows[2].extra_metadata.is_none());

    let stored: Vec<TranscriptMessage> =
        serde_json::from_str(&serde_json::to_string(&rows).unwrap()).unwrap();
    assert_eq!(DiscoveryOptions::recorded_in(&stored), Some(options));

    assert!(!options.record_on(&mut rows, &[false, false, false]));
}

fn forced(config: ToolSearchConfig) -> ToolSearchConfig {
    ToolSearchConfig {
        force_pin: true,
        ..config
    }
}

/// `rows` of a thread that started with `options`.
fn thread_started_with(options: DiscoveryOptions) -> Vec<TranscriptMessage> {
    let mut rows = vec![
        TranscriptMessage::system("prompt"),
        TranscriptMessage::new("user", "hi"),
    ];
    assert!(options.record_on(&mut rows, &[true, true]));
    rows
}

#[test]
fn forced_discovery_options_apply_to_existing_thread() {
    let started_with = DiscoveryOptions {
        promote_found_tools: false,
        collapse_search_results: true,
        ..DiscoveryOptions::TODAY
    };
    let rows = thread_started_with(started_with);
    let wanted = config(true, true, false, true);

    // Without the override the thread keeps what it started with.
    assert_eq!(resolve(&wanted, &rows, false).effective, started_with);

    // With it, this turn runs with the new options and records them.
    let resolved = resolve(&forced(wanted.clone()), &rows, false);
    let new_options = DiscoveryOptions::from_config(&wanted);
    assert_eq!(resolved.effective, new_options);
    assert_eq!(resolved.record, Some(new_options));

    // Recorded on a later row, they are what every later turn reads back.
    let mut rows = rows;
    rows.push(TranscriptMessage::new("user", "next"));
    assert!(new_options.record_on(&mut rows, &[false, false, true]));
    assert_eq!(DiscoveryOptions::recorded_in(&rows), Some(new_options));
    assert_eq!(resolve(&wanted, &rows, false).effective, new_options);
}

#[test]
fn forcing_todays_behaviour_replaces_a_recorded_pin() {
    let rows = thread_started_with(DiscoveryOptions {
        collapse_search_results: true,
        ..DiscoveryOptions::TODAY
    });
    let resolved = resolve(&forced(ToolSearchConfig::default()), &rows, false);
    assert_eq!(resolved.effective, DiscoveryOptions::TODAY);
    assert_eq!(
        resolved.record,
        Some(DiscoveryOptions::TODAY),
        "today's behaviour must be written down, or the old pin would win"
    );
}

#[test]
fn forcing_what_the_thread_already_runs_with_records_nothing() {
    let wanted = config(false, false, true, true);
    let rows = thread_started_with(DiscoveryOptions::from_config(&wanted));
    let resolved = resolve(&forced(wanted), &rows, false);
    assert_eq!(resolved.record, None);

    // A thread that recorded nothing already runs with today's behaviour.
    let legacy = vec![TranscriptMessage::system("prompt")];
    assert_eq!(
        resolve(&forced(ToolSearchConfig::default()), &legacy, false).record,
        None
    );
}

#[test]
fn the_latest_record_on_a_thread_wins() {
    let first = DiscoveryOptions {
        collapse_search_results: true,
        ..DiscoveryOptions::TODAY
    };
    let second = DiscoveryOptions {
        compact_search_results: true,
        ..DiscoveryOptions::TODAY
    };
    let mut rows = thread_started_with(first);
    rows.push(TranscriptMessage::new("user", "later"));
    assert!(second.record_on(&mut rows, &[false, false, true]));
    assert_eq!(DiscoveryOptions::recorded_in(&rows), Some(second));
}

#[test]
fn a_prompt_that_already_records_its_options_keeps_them_when_it_is_prepared_again() {
    let started_with = DiscoveryOptions {
        collapse_search_results: true,
        ..DiscoveryOptions::TODAY
    };
    let rows = thread_started_with(started_with);
    let resolved = resolve(&ToolSearchConfig::default(), &rows, true);
    assert_eq!(resolved.effective, started_with);
    assert_eq!(resolved.record, None);
}

#[test]
fn compaction_carries_the_record_onto_the_first_row_of_the_new_generation() {
    let options = DiscoveryOptions {
        collapse_search_results: true,
        ..DiscoveryOptions::TODAY
    };
    let prior = thread_started_with(options);
    let mut next = vec![
        TranscriptMessage::system("refreshed prompt"),
        TranscriptMessage::new("user", "summary"),
    ];
    assert!(carry_forward(&prior, &mut next, None));
    assert_eq!(DiscoveryOptions::recorded_in(&next), Some(options));
    assert!(next[1].extra_metadata.is_none());

    // Already there, or nothing to carry: nothing is rewritten.
    let before = next.clone();
    assert!(!carry_forward(&prior, &mut next, None));
    assert_eq!(next, before);
    let mut bare = vec![TranscriptMessage::system("prompt")];
    assert!(!carry_forward(
        &[TranscriptMessage::system("p")],
        &mut bare,
        None
    ));
    assert!(bare[0].extra_metadata.is_none());
    assert!(!carry_forward(&prior, &mut [], None));
}
