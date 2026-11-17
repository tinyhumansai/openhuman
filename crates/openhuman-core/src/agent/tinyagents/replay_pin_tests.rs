use tinyagents_harness::reasoning_replay::ReasoningReplay as HarnessReplay;
use tinyagents_harness::result_expiry::{ExpiryOverrides, ToolResultExpiryPolicy};

use super::*;
use crate::config::ReasoningReplay;

fn settings(age: Option<u32>, exempt: &[&str], replay: ReasoningReplay) -> ContextSettings {
    let mut settings = ContextSettings::default();
    settings.tool_result_max_age_turns = age;
    settings.tool_result_exempt_tools = exempt.iter().map(|name| (*name).to_owned()).collect();
    settings.replay_reasoning = replay;
    settings
}

#[test]
fn today_is_the_harness_default_and_the_config_default() {
    let policy = ToolResultExpiryPolicy::default();
    let overrides = ReplayOptions::TODAY.expiry_overrides();
    assert_eq!(overrides.enabled, Some(policy.enabled));
    assert_eq!(overrides.exempt_tools, Some(policy.exempt_tools));
    assert_eq!(
        ReplayOptions::TODAY.replay_reasoning,
        HarnessReplay::default()
    );
    assert_eq!(
        ReplayOptions::from_config(&ContextSettings::default()),
        ReplayOptions::TODAY
    );
}

#[test]
fn config_options_map_onto_the_harness_expiry_and_replay_options() {
    let options = ReplayOptions::from_config(&settings(
        Some(4),
        &["tool_search"],
        ReasoningReplay::CurrentTurn,
    ));
    assert_eq!(
        options.expiry_overrides(),
        ExpiryOverrides {
            enabled: Some(true),
            max_age_turns: Some(4),
            exempt_tools: Some(vec!["tool_search".into()]),
        }
    );
    assert_eq!(options.replay_reasoning, HarnessReplay::CurrentTurn);

    let policy = ToolResultExpiryPolicy::default().with_overrides(&options.expiry_overrides());
    assert!(policy.enabled);
    assert_eq!(policy.max_age_turns, 4);
    assert_eq!(policy.exempt_tools, ["tool_search"]);
}

#[test]
fn an_age_of_zero_or_none_switches_expiry_off_and_drops_the_exempt_list() {
    for age in [None, Some(0)] {
        let options =
            ReplayOptions::from_config(&settings(age, &["tool_search"], ReasoningReplay::All));
        assert_eq!(options, ReplayOptions::TODAY);
        assert_eq!(options.expiry_overrides().enabled, Some(false));
    }
}

#[test]
fn a_new_thread_takes_the_config_and_records_only_a_change_from_today() {
    let wanted = settings(Some(6), &[], ReasoningReplay::All);
    let changed = resolve(&wanted, &[], true);
    assert_eq!(changed.effective, ReplayOptions::from_config(&wanted));
    assert_eq!(changed.record.as_ref(), Some(&changed.effective));

    let unchanged = resolve(&ContextSettings::default(), &[], true);
    assert_eq!(unchanged.effective, ReplayOptions::TODAY);
    assert_eq!(unchanged.record, None);
}

#[test]
fn a_resumed_thread_keeps_what_it_recorded_whatever_the_config_says_now() {
    let started_with =
        ReplayOptions::from_config(&settings(Some(3), &["a"], ReasoningReplay::CurrentTurn));
    let mut rows = vec![
        TranscriptMessage::system("prompt"),
        TranscriptMessage::new("user", "hi"),
    ];
    assert!(started_with.record_on(&mut rows, &[true, true]));

    let now = settings(None, &[], ReasoningReplay::All);
    let resumed = resolve(&now, &rows, false);
    assert_eq!(resumed.effective, started_with);
    assert_eq!(resumed.record, None);
}

#[test]
fn a_generation_start_over_recorded_rows_keeps_the_recorded_pin() {
    // A compacted head whose prefix is prepared again is a generation-start
    // turn, but its rows still record the thread's options.
    let started_with =
        ReplayOptions::from_config(&settings(None, &[], ReasoningReplay::CurrentTurn));
    let mut rows = vec![
        TranscriptMessage::system("prompt"),
        TranscriptMessage::new("user", "hi"),
    ];
    assert!(started_with.record_on(&mut rows, &[true, true]));

    let now = settings(Some(6), &["a"], ReasoningReplay::All);
    let resolved = resolve(&now, &rows, true);
    assert_eq!(resolved.effective, started_with);
    assert_eq!(resolved.record, None);
}

#[test]
fn a_thread_that_recorded_nothing_keeps_todays_behaviour_whatever_the_config_says() {
    let rows = vec![TranscriptMessage::system("prompt")];
    let resumed = resolve(
        &settings(Some(2), &[], ReasoningReplay::CurrentTurn),
        &rows,
        false,
    );
    assert_eq!(resumed.effective, ReplayOptions::TODAY);
    assert_eq!(resumed.record, None);
}

#[test]
fn recording_touches_only_the_first_fresh_row_and_survives_serialization() {
    let options = ReplayOptions::from_config(&settings(Some(5), &[], ReasoningReplay::All));
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
    assert_eq!(ReplayOptions::recorded_in(&stored), Some(options.clone()));
    assert!(!options.record_on(&mut rows, &[false, false, false]));
}

#[test]
fn replay_and_discovery_records_share_a_row_without_clobbering_each_other() {
    use crate::agent::tinyagents::discovery::DiscoveryOptions;

    let discovery = DiscoveryOptions {
        promote_found_tools: false,
        ..DiscoveryOptions::TODAY
    };
    let replay = ReplayOptions::from_config(&settings(Some(5), &[], ReasoningReplay::All));
    let mut rows = vec![TranscriptMessage::system("prompt")];
    assert!(discovery.record_on(&mut rows, &[true]));
    assert!(replay.record_on(&mut rows, &[true]));
    assert_eq!(DiscoveryOptions::recorded_in(&rows), Some(discovery));
    assert_eq!(ReplayOptions::recorded_in(&rows), Some(replay));
}

fn with_keep_recent(tokens: Option<u32>) -> ContextSettings {
    let mut settings = ContextSettings::default();
    settings.keep_recent_tokens = tokens;
    settings
}

#[test]
fn a_new_thread_pins_the_compaction_tail_it_starts_with() {
    let created = resolve(&with_keep_recent(Some(12_000)), &[], true);
    assert_eq!(created.effective.keep_recent_tokens, Some(12_000));
    assert_eq!(
        created
            .record
            .as_ref()
            .map(|options| options.keep_recent_tokens),
        Some(Some(12_000)),
        "a tail that differs from the default is written down"
    );

    // Not asking for one records nothing, as before.
    let plain = resolve(&with_keep_recent(None), &[], true);
    assert_eq!(plain.effective, ReplayOptions::TODAY);
    assert_eq!(plain.record, None);
}

#[test]
fn a_resumed_thread_keeps_its_pinned_compaction_tail_whatever_the_config_says_now() {
    let started_with = ReplayOptions::from_config(&with_keep_recent(Some(12_000)));
    let mut rows = vec![
        TranscriptMessage::system("prompt"),
        TranscriptMessage::new("user", "hi"),
    ];
    assert!(started_with.record_on(&mut rows, &[true, true]));

    for now in [None, Some(3_000)] {
        let resumed = resolve(&with_keep_recent(now), &rows, false);
        assert_eq!(resumed.effective.keep_recent_tokens, Some(12_000));
        assert_eq!(resumed.record, None, "a turn override never re-pins it");
    }
}

#[test]
fn a_pinned_compaction_tail_beats_the_configured_one_and_an_unpinned_thread_follows_it() {
    let pinned = ReplayOptions::from_config(&with_keep_recent(Some(12_000)));
    assert_eq!(pinned.keep_recent_tokens_or(Some(3_000)), Some(12_000));
    assert_eq!(pinned.keep_recent_tokens_or(None), Some(12_000));

    assert_eq!(
        ReplayOptions::TODAY.keep_recent_tokens_or(Some(3_000)),
        Some(3_000)
    );
    assert_eq!(ReplayOptions::TODAY.keep_recent_tokens_or(None), None);
}

#[test]
fn a_record_without_a_compaction_tail_reads_back_and_writes_the_same_bytes() {
    // Written before the tail was pinned: no `keep_recent_tokens` key.
    let stored = serde_json::json!({
        "tool_result_max_age_turns": 6,
        "tool_result_exempt_tools": ["tool_search"],
        "replay_reasoning": "current_turn",
    });
    let read: ReplayOptions = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(read.keep_recent_tokens, None);
    assert_eq!(serde_json::to_value(&read).unwrap(), stored);
}
