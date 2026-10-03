use super::*;
use serde_json::json;

#[test]
fn defaults_select_tinyhumans_with_ingest_and_context_on() {
    let config = MemoryConfig::default();
    assert_eq!(config.engine, DEFAULT_MEMORY_ENGINE);
    assert!(config.engines.is_empty());
    assert!(config.sources.is_empty());
    assert!(config.conversations.enabled);
    assert_eq!(
        config.conversations.batch_turns,
        DEFAULT_CONVERSATION_BATCH_TURNS
    );
    assert_eq!(
        config.conversations.idle_secs,
        DEFAULT_CONVERSATION_IDLE_SECS
    );
    assert!(config.context.enabled);
    assert_eq!(config.context.interval_mins, DEFAULT_CONTEXT_INTERVAL_MINS);
    assert_eq!(config.context.budget_tokens, DEFAULT_CONTEXT_BUDGET_TOKENS);
}

#[test]
fn parses_a_full_v2_section() {
    let config: MemoryConfig = toml::from_str(
        r#"
engine = "cortexdb"

[engines.cortexdb]
endpoint = "https://cortex.example"

[conversations]
enabled = false
batch_turns = 8
idle_secs = 30

[context]
interval_mins = 60
budget_tokens = 500

[[sources]]
id = "src-1"
kind = "folder"
target = "/notes"
label = "Notes"
schedule_mins = 15
"#,
    )
    .expect("v2 section parses");
    assert_eq!(config.engine, "cortexdb");
    assert_eq!(
        config.endpoint_for("cortexdb").as_deref(),
        Some("https://cortex.example")
    );
    assert!(!config.conversations.enabled);
    assert_eq!(config.conversations.batch_turns, 8);
    assert_eq!(config.context.interval_mins, 60);
    assert!(config.context.enabled, "unset fields keep their default");
    assert_eq!(config.sources.len(), 1);
    assert_eq!(config.sources[0].kind, MemorySourceKind::Folder);
    assert_eq!(config.sources[0].schedule_mins, Some(15));
}

#[test]
fn ignores_v1_keys_instead_of_failing() {
    let config: MemoryConfig = toml::from_str(
        r#"
backend = "sqlite"
auto_save = true
embedding_model = "embedding-v1"
"#,
    )
    .expect("a v1 [memory] section still parses");
    assert_eq!(config.engine, DEFAULT_MEMORY_ENGINE);
    assert_eq!(config.embedding_model, "embedding-v1");
}

#[test]
fn drops_an_unreadable_source_and_keeps_the_rest() {
    let config: MemoryConfig = toml::from_str(
        r#"
[[sources]]
id = "bad"
kind = "twitter_query"
target = "rust"

[[sources]]
id = "good"
kind = "rss"
target = "https://example.com/feed.xml"
"#,
    )
    .expect("one bad entry does not fail the section");
    assert_eq!(config.sources.len(), 1);
    assert_eq!(config.sources[0].id, "good");
    assert_eq!(config.sources[0].label, "", "label defaults to empty");
}

#[test]
fn endpoint_for_ignores_blank_and_missing_endpoints() {
    let mut config = MemoryConfig::default();
    assert_eq!(config.endpoint_for("cortexdb"), None);
    config.engines.insert(
        "cortexdb".into(),
        MemoryEngineSettings {
            endpoint: Some("   ".into()),
        },
    );
    assert_eq!(config.endpoint_for("cortexdb"), None);
    config.engines.insert(
        "cortexdb".into(),
        MemoryEngineSettings {
            endpoint: Some(" https://c.example ".into()),
        },
    );
    assert_eq!(
        config.endpoint_for("cortexdb").as_deref(),
        Some("https://c.example")
    );
}

#[test]
fn source_kind_round_trips_its_wire_names() {
    for kind in MemorySourceKind::ALL {
        assert_eq!(MemorySourceKind::parse(kind.as_str()), Some(kind));
        assert_eq!(
            serde_json::to_value(kind).expect("serialises"),
            json!(kind.as_str())
        );
    }
    assert_eq!(
        MemorySourceKind::parse(" FOLDER "),
        Some(MemorySourceKind::Folder)
    );
    assert_eq!(MemorySourceKind::parse("twitter_query"), None);
}

#[test]
fn migrates_every_mappable_legacy_kind() {
    let cases = [
        (
            json!({"id":"a","kind":"folder","path":"/p"}),
            MemorySourceKind::Folder,
            "/p",
        ),
        (
            json!({"id":"b","kind":"file","path":"/f.md"}),
            MemorySourceKind::File,
            "/f.md",
        ),
        (
            json!({"id":"c","kind":"web_page","url":"https://w"}),
            MemorySourceKind::Link,
            "https://w",
        ),
        (
            json!({"id":"d","kind":"github_repo","url":"o/r"}),
            MemorySourceKind::Github,
            "o/r",
        ),
        (
            json!({"id":"e","kind":"rss_feed","url":"https://f"}),
            MemorySourceKind::Rss,
            "https://f",
        ),
        (
            json!({"id":"f","kind":"composio","toolkit":"gmail"}),
            MemorySourceKind::Composio,
            "gmail",
        ),
    ];
    for (legacy, kind, target) in cases {
        let migrated = migrate_legacy_source(&legacy).expect("mappable kind migrates");
        assert_eq!(migrated.kind, kind);
        assert_eq!(migrated.target, target);
        assert_eq!(migrated.label, target, "label falls back to the target");
        assert_eq!(migrated.schedule_mins, None);
    }
}

#[test]
fn migration_keeps_the_legacy_label() {
    let migrated = migrate_legacy_source(&json!({
        "id": "a", "kind": "folder", "path": "/p", "label": "Work"
    }))
    .expect("migrates");
    assert_eq!(migrated.label, "Work");
}

#[test]
fn migration_drops_unmappable_disabled_and_incomplete_entries() {
    for legacy in [
        json!({"id":"t","kind":"twitter_query","query":"rust"}),
        json!({"id":"c","kind":"conversation"}),
        json!({"id":"x","kind":"folder","path":"/p","enabled":false}),
        json!({"id":"y","kind":"folder"}),
        json!({"kind":"folder","path":"/p"}),
        json!({"id":"z","kind":"folder","path":"   "}),
        json!("not an object"),
    ] {
        assert_eq!(migrate_legacy_source(&legacy), None, "{legacy}");
    }
}
