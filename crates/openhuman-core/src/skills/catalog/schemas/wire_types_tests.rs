use super::*;

use serde_json::json;

use {crate::skills::catalog::types::RegistryCatalogEntry, tinyskills::CatalogEntry};

fn params(value: serde_json::Value) -> CatalogParams {
    serde_json::from_value(value).expect("params")
}

#[test]
fn legacy_and_multi_value_filters_merge_without_duplicates() {
    let query = params(json!({
        "query": "git",
        "source": "ClawHub",
        "sources": ["clawhub", "skills.sh", " "],
        "category": "dev",
    }))
    .into_query();
    assert_eq!(query.text, "git");
    assert_eq!(query.upstreams, ["ClawHub", "skills.sh"]);
    assert_eq!(query.categories, ["dev"]);
    assert!(!query.is_paged(), "no paging params reads every match");
}

#[test]
fn paging_params_make_the_read_paged() {
    let query = params(json!({ "page": 2 })).into_query();
    assert!(query.is_paged());
    assert_eq!(query.page, Some(2));
    assert_eq!(query.page_size, None);
}

#[test]
fn a_page_serializes_the_entry_shape_and_snake_case_freshness() {
    let entry = RegistryCatalogEntry {
        entry: CatalogEntry {
            id: "git-helper".into(),
            name: "git-helper".into(),
            description: "d".into(),
            source: "built-in".into(),
            category: "dev".into(),
            author: None,
            version: None,
            tags: Vec::new(),
            platforms: Vec::new(),
            download_url: "https://example.com/SKILL.md".into(),
            source_url: Some("https://example.com/git-helper".into()),
            docs_path: None,
            commands: Vec::new(),
            env_vars: Vec::new(),
            license: None,
        },
        registry: "hermes".into(),
        installable: true,
        category_label: None,
    };
    let page: CatalogResult = CatalogPage {
        entries: vec![entry],
        total: 1,
        page: 1,
        page_size: 25,
        total_pages: 1,
        freshness: Freshness::LocalFallback,
        fetched_at: Some(1),
        refreshing: true,
        last_error: Some(
            serde_json::from_value(json!({
                "kind": "rate_limited",
                "message": "rate limited: retry after 9s",
                "retry_after_secs": 9
            }))
            .unwrap(),
        ),
    };
    let value = serde_json::to_value(&page).unwrap();
    assert_eq!(value["freshness"], "local_fallback");
    assert_eq!(value["refreshing"], true);
    assert_eq!(value["last_error"]["kind"], "rate_limited");
    assert_eq!(value["last_error"]["retry_after_secs"], 9);
    let entry = &value["entries"][0];
    for field in [
        "id",
        "name",
        "download_url",
        "source_url",
        "docs_path",
        "registry",
        "installable",
    ] {
        assert!(entry.get(field).is_some(), "missing {field}: {entry}");
    }
}
