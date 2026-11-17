use super::*;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};
use tinyskills::{Clock, Freshness};

use crate::skills::catalog::test_fixtures::{hermes_item, Fixture};

#[derive(Clone)]
struct ManualClock(Arc<Mutex<SystemTime>>);

impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        *self.0.lock().unwrap()
    }
}

fn numbered(count: usize) -> Vec<Value> {
    (0..count)
        .map(|i| hermes_item(&format!("review-{i:02}"), "built-in"))
        .collect()
}

fn paged(page: usize, page_size: usize) -> CatalogQuery {
    CatalogQuery {
        page: Some(page),
        page_size: Some(page_size),
        ..CatalogQuery::default()
    }
}

#[test]
fn refresh_on_boot_enabled_defaults_on_and_accepts_common_false_values() {
    assert!(refresh_on_boot_enabled(None));
    assert!(refresh_on_boot_enabled(Some("1")));
    assert!(refresh_on_boot_enabled(Some("true")));

    assert!(!refresh_on_boot_enabled(Some("0")));
    assert!(!refresh_on_boot_enabled(Some("false")));
    assert!(!refresh_on_boot_enabled(Some(" no ")));
    assert!(!refresh_on_boot_enabled(Some("OFF")));
}

#[test]
fn every_error_kind_renders_a_typed_prefix() {
    let cases = [
        (
            RegistryError::Timeout {
                operation: "catalog",
                budget: Duration::from_secs(180),
            },
            "SKILL_REGISTRY_TIMEOUT: ",
        ),
        (
            RegistryError::Unavailable { status: 503 },
            "SKILL_REGISTRY_UNAVAILABLE: ",
        ),
        (
            RegistryError::RateLimited {
                retry_after: Some(Duration::from_secs(42)),
            },
            "SKILL_REGISTRY_RATE_LIMITED: rate limited: retry after 42s",
        ),
        (
            RegistryError::NotFound {
                id: "nope".into(),
                closest: vec!["clawhub/nope-ops".into()],
            },
            "SKILL_REGISTRY_NOT_FOUND: ",
        ),
        (
            RegistryError::UpstreamAmbiguous {
                name: "AI Code Review".into(),
            },
            "SKILL_REGISTRY_UPSTREAM_AMBIGUOUS: ",
        ),
        (
            RegistryError::Malformed {
                what: "catalog",
                detail: "expected an array".into(),
            },
            "SKILL_REGISTRY_MALFORMED: ",
        ),
    ];
    for (error, prefix) in cases {
        let message = registry_error_message(&error);
        assert!(message.starts_with(prefix), "{message} vs {prefix}");
    }
}

#[test]
fn a_skill_without_a_download_links_its_source_page() {
    let message = registry_error_message(&RegistryError::NoDirectDownload {
        name: "code-audit".into(),
        source_url: Some("https://clawhub.ai/skills/agentkilox-code-audit".into()),
    });
    assert!(message.starts_with("SKILL_REGISTRY_NO_DIRECT_DOWNLOAD: "));
    assert!(message.contains("https://clawhub.ai/skills/agentkilox-code-audit"));
}

#[test]
fn only_registry_defects_are_reportable() {
    assert!(is_reportable(RegistryErrorKind::TransportContract, true));
    assert!(is_reportable(RegistryErrorKind::Malformed, true));
    assert!(!is_reportable(RegistryErrorKind::TransportContract, false));
    assert!(!is_reportable(RegistryErrorKind::Malformed, false));
    for kind in [
        RegistryErrorKind::Timeout,
        RegistryErrorKind::Unavailable,
        RegistryErrorKind::RateLimited,
        RegistryErrorKind::Transport,
        RegistryErrorKind::NotFound,
        RegistryErrorKind::NoDirectDownload,
    ] {
        assert!(!is_reportable(kind, true), "{kind:?}");
    }
}

#[test]
fn queries_page_only_when_asked_and_clamp_the_page_size() {
    let unpaged = skill_query(&CatalogQuery::default());
    assert_eq!(unpaged.page, 1);
    assert_eq!(unpaged.page_size, usize::MAX);

    let defaulted = skill_query(&CatalogQuery {
        page: Some(0),
        ..CatalogQuery::default()
    });
    assert_eq!(defaulted.page, 1);
    assert_eq!(defaulted.page_size, DEFAULT_PAGE_SIZE);

    let clamped = skill_query(&paged(3, 10_000));
    assert_eq!(clamped.page, 3);
    assert_eq!(clamped.page_size, MAX_PAGE_SIZE);
    assert_eq!(clamped.read, ReadPolicy::AllowStale);
}

#[tokio::test]
async fn pages_split_the_matches_and_carry_full_entries() {
    let fixture = Fixture::start(numbered(30)).await;
    let registry = fixture.registry();

    let first = catalog_page_in(&registry, &paged(1, 25)).await.unwrap();
    assert_eq!(first.total, 30);
    assert_eq!(first.total_pages, 2);
    assert_eq!(first.entries.len(), 25);
    assert_eq!(first.freshness, Freshness::Live);
    assert!(first.fetched_at.is_some());
    assert!(first.last_error.is_none());
    let entry = &first.entries[0];
    assert_eq!(entry.registry, "hermes");
    assert!(entry.installable);
    assert_eq!(
        entry.entry.download_url,
        format!("{}/skills/{}/SKILL.md", fixture.base, entry.entry.name)
    );
    assert!(entry.entry.docs_path.is_some());

    let second = catalog_page_in(&registry, &paged(2, 25)).await.unwrap();
    assert_eq!(second.page, 2);
    assert_eq!(second.entries.len(), 5);
    assert_eq!(fixture.catalog_hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_unpaged_read_returns_every_match_on_one_page() {
    let fixture = Fixture::start(numbered(30)).await;
    let page = catalog_page_in(&fixture.registry(), &CatalogQuery::default())
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 30);
    assert_eq!(page.total_pages, 1);
    assert!(page.entries.iter().all(|entry| entry.installable));
}

#[tokio::test]
async fn several_sources_filter_together() {
    let fixture = Fixture::start(vec![
        hermes_item("alpha", "ClawHub"),
        hermes_item("beta", "skills.sh"),
        hermes_item("gamma", "built-in"),
    ])
    .await;
    let query = CatalogQuery {
        upstreams: vec!["clawhub".into(), "built-in".into()],
        ..paged(1, 25)
    };
    let page = catalog_page_in(&fixture.registry(), &query).await.unwrap();
    let names: Vec<&str> = page.entries.iter().map(|e| e.entry.name.as_str()).collect();
    assert_eq!(page.total, 2);
    assert!(
        names.contains(&"alpha") && names.contains(&"gamma"),
        "{names:?}"
    );
}

#[tokio::test]
async fn search_ranks_installable_entries_before_uninstallable_ones() {
    let fixture = Fixture::start(vec![
        json!({
            "name": "review-agent",
            "description": "x",
            "source": "LobeHub",
            "identifier": "lobehub/review-agent",
            "sourceUrl": "https://lobehub.com/agent/review-agent"
        }),
        json!({
            "name": "review-skill",
            "description": "x",
            "source": "ClawHub",
            "identifier": "review-skill"
        }),
    ])
    .await;
    let query = CatalogQuery {
        text: "review".into(),
        ..paged(1, 25)
    };
    let page = catalog_page_in(&fixture.registry_without_download_base(), &query)
        .await
        .unwrap();
    let ids: Vec<&str> = page.entries.iter().map(|e| e.entry.id.as_str()).collect();
    assert_eq!(ids, ["clawhub/review-skill", "lobehub/review-agent"]);
    assert!(!page.entries[1].installable);
    assert_eq!(
        page.entries[1].entry.source_url.as_deref(),
        Some("https://lobehub.com/agent/review-agent")
    );
}

#[tokio::test]
async fn concurrent_cold_reads_share_one_upstream_fetch() {
    let fixture = Fixture::start(numbered(5)).await;
    let registry = fixture.registry();
    let mut handles = Vec::new();
    for _ in 0..6 {
        let registry = Arc::clone(&registry);
        handles.push(tokio::spawn(async move {
            catalog_page_in(&registry, &paged(1, 25)).await
        }));
    }
    for handle in handles {
        assert_eq!(handle.await.unwrap().unwrap().total, 5);
    }
    assert_eq!(fixture.catalog_hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failed_refresh_keeps_serving_the_held_catalog_with_its_error() {
    let fixture = Fixture::start(numbered(3)).await;
    let clock = ManualClock(Arc::new(Mutex::new(SystemTime::now())));
    let registry = fixture.registry_with_clock(clock.clone());
    assert_eq!(
        catalog_page_in(&registry, &paged(1, 25))
            .await
            .unwrap()
            .freshness,
        Freshness::Live
    );

    fixture.catalog_status.store(503, Ordering::SeqCst);
    let refreshed = catalog_page_in(
        &registry,
        &CatalogQuery {
            force_refresh: true,
            ..paged(1, 25)
        },
    )
    .await
    .expect("the held catalog still answers");
    assert_eq!(refreshed.total, 3);
    let error = refreshed.last_error.expect("the failure is reported");
    assert_eq!(error.kind, RegistryErrorKind::Unavailable);

    *clock.0.lock().unwrap() += Duration::from_secs(2 * 3600);
    let stale = catalog_page_in(&registry, &paged(1, 25)).await.unwrap();
    assert_eq!(stale.freshness, Freshness::Cached);
    assert!(stale.last_error.is_some());
}

#[tokio::test]
async fn a_cold_registry_with_an_unreachable_upstream_returns_the_typed_error() {
    let fixture = Fixture::start(Vec::new()).await;
    fixture.catalog_status.store(502, Ordering::SeqCst);
    let error = catalog_page_in(&fixture.registry(), &paged(1, 25))
        .await
        .expect_err("nothing is held");
    assert_eq!(error.kind(), RegistryErrorKind::Unavailable);
    assert!(registry_error_message(&error).starts_with("SKILL_REGISTRY_UNAVAILABLE: "));
}

#[tokio::test]
async fn facets_list_upstreams_and_categories_most_entries_first() {
    let fixture = Fixture::start(vec![
        hermes_item("a", "ClawHub"),
        hermes_item("b", "ClawHub"),
        hermes_item("c", "built-in"),
    ])
    .await;
    let facets = catalog_facets_in(&fixture.registry()).await.unwrap();
    let upstreams: Vec<(&str, usize)> = facets
        .upstreams
        .iter()
        .map(|facet| (facet.value.as_str(), facet.count))
        .collect();
    assert_eq!(upstreams, [("ClawHub", 2), ("built-in", 1)]);
    assert_eq!(facets.categories[0].value, "productivity");
}

#[tokio::test]
async fn detail_carries_the_overview_and_an_unknown_id_suggests_real_ones() {
    let mut item = hermes_item("git-helper", "built-in");
    item["overview"] = json!("Longer text about git-helper.");
    let fixture = Fixture::start(vec![item, hermes_item("notes", "built-in")]).await;
    let registry = fixture.registry();

    let detail = catalog_detail_in(&registry, "git-helper").await.unwrap();
    assert_eq!(detail.overview, "Longer text about git-helper.");
    assert_eq!(detail.entry.entry.name, "git-helper");

    let error = catalog_detail_in(&registry, "git-helpr")
        .await
        .expect_err("unknown id");
    assert_eq!(error.kind(), RegistryErrorKind::NotFound);
    assert!(error.to_string().contains("git-helper"), "{error}");
}
