use super::*;

const SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../vendor/tinychannels/crates/tinychannels-bus/schema/web-channel-event.schema.json"
);

fn contract_tags() -> Vec<String> {
    let raw = std::fs::read_to_string(SCHEMA).expect("web channel event schema (init submodules)");
    let schema: serde_json::Value = serde_json::from_str(&raw).expect("schema is JSON");
    schema["oneOf"]
        .as_array()
        .expect("oneOf variants")
        .iter()
        .map(|variant| {
            variant["properties"]["event"]["const"]
                .as_str()
                .expect("variant has an event tag")
                .to_string()
        })
        .collect()
}

#[test]
fn every_contract_event_is_classified() {
    let unclassified: Vec<String> = contract_tags()
        .into_iter()
        .filter(|tag| disposition(tag) == Disposition::Unknown)
        .collect();
    assert!(
        unclassified.is_empty(),
        "web channel events with no TUI disposition: {unclassified:?}"
    );
}

#[test]
fn classified_tags_exist_in_the_contract() {
    let tags = contract_tags();
    for tag in REDUCED.iter().chain(IGNORED) {
        assert!(
            tags.iter().any(|t| t == tag),
            "{tag} is not in the contract"
        );
    }
}

#[test]
fn no_tag_is_both_reduced_and_ignored() {
    for tag in IGNORED {
        assert!(!REDUCED.contains(tag), "{tag} is in both lists");
        assert!(!tag.starts_with("subagent_"), "{tag} is reduced by prefix");
    }
}

#[test]
fn dispositions_cover_the_three_cases() {
    assert_eq!(disposition("chat_done"), Disposition::Reduced);
    assert_eq!(disposition("subagent_spawned"), Disposition::Reduced);
    assert_eq!(disposition("turn_cost"), Disposition::Ignored);
    assert_eq!(disposition("channel_event"), Disposition::Ignored);
    assert_eq!(disposition("from_the_future"), Disposition::Unknown);
}
