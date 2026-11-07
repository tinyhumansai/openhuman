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
