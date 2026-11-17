use super::*;
use tinyconnectors_bus::vocabulary::{ToolkitVocabulary, VocabularyAction};

fn reply() -> (Vec<CuratedTool>, VocabularyResponse) {
    let listed = vec![CuratedTool {
        toolkit: "gmail".into(),
        slug: "GMAIL_SEND_EMAIL".into(),
        scope: ToolScope::Write,
    }];
    let vocabulary = VocabularyResponse {
        toolkits: vec![
            ToolkitVocabulary {
                toolkit: "gmail".into(),
                description: "Mail".into(),
                native_provider: true,
                actions: Some(vec![
                    VocabularyAction {
                        slug: "GMAIL_FETCH_EMAILS".into(),
                        scope: ToolScope::Read,
                    },
                    VocabularyAction {
                        slug: "GMAIL_SEND_EMAIL".into(),
                        scope: ToolScope::Write,
                    },
                ]),
            },
            ToolkitVocabulary {
                toolkit: "Google_Calendar".into(),
                description: "Calendar".into(),
                native_provider: false,
                actions: Some(vec![VocabularyAction {
                    slug: "GOOGLECALENDAR_CREATE_EVENT".into(),
                    scope: ToolScope::Write,
                }]),
            },
            ToolkitVocabulary {
                toolkit: "unknownkit".into(),
                description: "generic".into(),
                native_provider: false,
                actions: None,
            },
        ],
        agent_ready_toolkits: vec!["notion".into(), "gmail".into(), "gmail".into()],
    };
    (listed, vocabulary)
}

#[test]
fn snapshot_serves_catalogs_aliases_descriptions_and_native_flags() {
    let (listed, vocabulary) = reply();
    let snapshot = build("1", &listed, &vocabulary);
    assert_eq!(snapshot.toolkits, vec!["gmail".to_string()]);
    assert_eq!(snapshot.catalogs["gmail"].len(), 2);
    assert_eq!(snapshot.catalogs["gmail"][1].scope, ToolScope::Write);
    assert!(snapshot.catalogs.contains_key("google_calendar"));
    assert!(!snapshot.catalogs.contains_key("unknownkit"));
    assert!(snapshot.native.contains("gmail"));
    assert_eq!(snapshot.descriptions["gmail"], "Mail");
}

#[test]
fn intern_returns_one_allocation_per_distinct_string() {
    let a = intern("SOME_SLUG_FOR_INTERN_TEST");
    let b = intern("SOME_SLUG_FOR_INTERN_TEST");
    assert!(std::ptr::eq(a, b));
}

#[test]
fn lookups_use_the_installed_snapshot_and_invalidate_clears_it() {
    let (listed, vocabulary) = reply();
    install_for_test(&listed, &vocabulary);
    assert!(catalog_for_toolkit(" GMAIL ").is_some());
    assert!(has_native_provider("gmail"));
    assert_eq!(toolkit_description("gmail"), "Mail");
    assert_eq!(toolkit_description("nope"), FALLBACK_DESCRIPTION);
    assert_eq!(
        curated_scope_for("GMAIL_SEND_EMAIL"),
        Some(ToolScope::Write)
    );
    invalidate();
    assert!(store().read().unwrap().is_none());
}

#[test]
fn fixture_serves_aliases_and_every_native_provider() {
    for alias in [
        "one",
        "one_drive",
        "onedrive",
        "google_calendar",
        "microsoft",
    ] {
        assert!(catalog_for_toolkit(alias).is_some(), "{alias}");
    }
    assert!(catalog_for_toolkit("flowstestkit").is_none());
    assert!(has_native_provider("linear"));
    assert!(!has_native_provider("discord"));
}

#[test]
fn agent_ready_list_is_the_modules_sorted_and_deduplicated() {
    let (listed, vocabulary) = reply();
    let snapshot = build("1", &listed, &vocabulary);
    assert_eq!(snapshot.agent_ready, vec!["gmail", "notion"]);
}

#[test]
fn fixture_agent_ready_list_names_the_native_providers() {
    let slugs = agent_ready_toolkits();
    let mut sorted = slugs.clone();
    sorted.sort_unstable();
    assert_eq!(slugs, sorted);
    for native in ["gmail", "notion", "github", "linear"] {
        assert!(slugs.iter().any(|s| s == native), "{native} missing");
    }
}
