use super::*;
use serde_json::json;

#[test]
fn defaults_select_tinyhumans_with_logging_and_recall_on() {
    let config = MemoryConfig::default();
    assert_eq!(config.engine, DEFAULT_MEMORY_ENGINE);
    assert!(config.engines.is_empty());
    assert!(config.sources.is_empty());
    assert!(config.conversations.enabled);
    assert!(config.recall.enabled);
    assert_eq!(config.recall.budget_tokens, DEFAULT_RECALL_BUDGET_TOKENS);
    assert_eq!(
        config.recall.pre_turn_timeout_ms,
        DEFAULT_PRE_TURN_TIMEOUT_MS
    );
    assert_eq!(config.recall.build_delay_secs, DEFAULT_BUILD_DELAY_SECS);
    assert_eq!(config.agent_id, None);
    assert_eq!(config.root, None);
    assert_eq!(
        config.recall.team_limit, 3,
        "three other-agent turns per pack"
    );
    assert!(
        config.split_github_by_repo,
        "one scope per repository by default"
    );
}

#[test]
fn default_pre_turn_waits_five_seconds_for_memory() {
    assert_eq!(MemoryConfig::default().recall.pre_turn_timeout_ms, 5_000);
}

#[test]
fn turning_the_github_split_off_survives_a_save() {
    // On is the default and is not written; off is, so it reads back off.
    let on = toml::to_string(&MemoryConfig::default()).unwrap();
    assert!(!on.contains("split_github_by_repo"), "{on}");
    let off = MemoryConfig {
        split_github_by_repo: false,
        ..MemoryConfig::default()
    };
    let saved = toml::to_string(&off).unwrap();
    assert!(saved.contains("split_github_by_repo = false"), "{saved}");
    let read: MemoryConfig = toml::from_str(&saved).unwrap();
    assert!(!read.split_github_by_repo);
}

#[test]
fn parses_a_full_section() {
    let config: MemoryConfig = toml::from_str(
        r#"
engine = "cortexdb"
agent_id = "employee-7"
root = "project:acme"

[engines.cortexdb]
endpoint = "https://cortex.example"

[conversations]
enabled = false

[recall]
budget_tokens = 500
team_limit = 0

[agents.researcher]
agent_id = "desk"
recall = false

[[sources]]
id = "src-1"
kind = "folder"
target = "/notes"
label = "Notes"
schedule_mins = 15
"#,
    )
    .expect("section parses");
    assert_eq!(config.engine, "cortexdb");
    assert_eq!(
        config.endpoint_for("cortexdb").as_deref(),
        Some("https://cortex.example")
    );
    assert_eq!(config.agent_id.as_deref(), Some("employee-7"));
    assert_eq!(config.root.as_deref(), Some("project:acme"));
    assert!(!config.conversations.enabled);
    assert_eq!(config.recall.budget_tokens, 500);
    assert_eq!(config.recall.team_limit, 0);
    assert!(config.recall.enabled, "unset fields keep their default");
    assert_eq!(
        config.agents["researcher"].agent_id.as_deref(),
        Some("desk")
    );
    assert_eq!(config.agents["researcher"].recall, Some(false));
    assert_eq!(config.sources.len(), 1);
    assert_eq!(config.sources[0].kind, MemorySourceKind::Folder);
    assert_eq!(config.sources[0].schedule_mins, Some(15));
}

#[test]
fn ignores_retired_lifecycle_keys() {
    let config: MemoryConfig = toml::from_str(
        r#"
root_agents = ["orchestrator"]

[conversations]
enabled = true
batch_turns = 8
idle_secs = 30

[context]
interval_mins = 60

[agents.analyst]
namespace = "project:q4"
inherit = false
context = true
"#,
    )
    .expect("retired keys are ignored");
    assert!(config.conversations.enabled);
    assert_eq!(config.agents["analyst"], MemoryAgentConfig::default());
}

#[test]
fn parses_v1_keys_for_loader_migration() {
    let config: MemoryConfig = toml::from_str(
        r#"
backend = "sqlite"
auto_save = true
embedding_model = "embedding-v1"
"#,
    )
    .expect("a v1 [memory] section still parses");
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
fn drops_a_stale_composio_source_and_keeps_every_other_kind() {
    let config: MemoryConfig = toml::from_str(
        r#"
[[sources]]
id = "src-composio"
kind = "composio"
target = "gmail"

[[sources]]
id = "src-link"
kind = "link"
target = "https://example.com"

[[sources]]
id = "src-github"
kind = "github"
target = "o/r"

[[sources]]
id = "src-rss"
kind = "rss"
target = "https://example.com/feed.xml"

[[sources]]
id = "src-folder"
kind = "folder"
target = "/notes"

[[sources]]
id = "src-file"
kind = "file"
target = "/notes/a.md"
"#,
    )
    .expect("a removed kind must not fail the section");
    let ids: Vec<&str> = config.sources.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "src-link",
            "src-github",
            "src-rss",
            "src-folder",
            "src-file"
        ]
    );
    assert_eq!(MemorySourceKind::parse("composio"), None);
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
        json!({"id":"k","kind":"composio","toolkit":"gmail"}),
        json!({"id":"x","kind":"folder","path":"/p","enabled":false}),
        json!({"id":"y","kind":"folder"}),
        json!({"kind":"folder","path":"/p"}),
        json!({"id":"z","kind":"folder","path":"   "}),
        json!("not an object"),
    ] {
        assert_eq!(migrate_legacy_source(&legacy), None, "{legacy}");
    }
}

#[test]
fn observed_actor_is_off_by_default_and_unwritten_until_set() {
    let config = MemoryConfig::default();
    assert!(!config.observed_actor);
    let written = serde_json::to_value(&config).unwrap();
    assert!(
        written.get("observed_actor").is_none(),
        "off is not written"
    );

    let on: MemoryConfig = toml::from_str("observed_actor = true").unwrap();
    assert!(on.observed_actor);
}
