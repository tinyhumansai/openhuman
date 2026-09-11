use super::*;
use serde_json::json;

#[test]
fn parse_hermes_entry_derives_bundled_download_url_from_docs_path() {
    let item = json!({
        "name": "apple-notes",
        "description": "Manage Apple Notes",
        "category": "apple",
        "source": "built-in",
        "docsPath": "bundled/apple/apple-apple-notes",
        "tags": ["Apple"],
        "platforms": ["macos"],
        "commands": ["memo"],
        "envVars": []
    });
    let entry = parse_hermes_entry(&item).expect("entry");
    assert_eq!(
        entry.download_url,
        "https://raw.githubusercontent.com/NousResearch/hermes-agent/main/skills/apple/apple-notes/SKILL.md"
    );
}

#[test]
fn parse_hermes_entry_derives_optional_download_url_from_docs_path() {
    let item = json!({
        "name": "docker-management",
        "description": "Manage Docker",
        "category": "devops",
        "source": "optional",
        "docsPath": "optional/devops/devops-docker-management"
    });
    let entry = parse_hermes_entry(&item).expect("entry");
    assert_eq!(
        entry.download_url,
        "https://raw.githubusercontent.com/NousResearch/hermes-agent/main/optional-skills/devops/docker-management/SKILL.md"
    );
}

#[test]
fn parse_hermes_entry_derives_github_tree_source_url() {
    // NVIDIA shape: sourceUrl is a GitHub *tree* (directory) view, no
    // docsPath. The raw SKILL.md lives inside that directory. (#3741)
    let item = json!({
        "name": "aiq-deploy",
        "description": "Deploy AIQ",
        "category": "agentic-ai",
        "source": "NVIDIA",
        "docsPath": "",
        "sourceUrl": "https://github.com/NVIDIA/skills/tree/main/skills/aiq-deploy"
    });
    let entry = parse_hermes_entry(&item).expect("entry");
    assert_eq!(
        entry.download_url,
        "https://raw.githubusercontent.com/NVIDIA/skills/main/skills/aiq-deploy/SKILL.md"
    );
    assert_eq!(
        entry.source_url.as_deref(),
        Some("https://github.com/NVIDIA/skills/tree/main/skills/aiq-deploy")
    );
}

#[test]
fn parse_hermes_entry_derives_github_blob_source_url() {
    // browse.sh shape: sourceUrl is a GitHub *blob* pointing straight at the
    // SKILL.md file — rewrite host to raw, keep the path. (#3741)
    let item = json!({
        "name": "account-management",
        "description": "Account mgmt",
        "category": "account-management",
        "source": "browse.sh",
        "sourceUrl": "https://github.com/browserbase/browse.sh/blob/main/skills/plugandpay.com/account-management-ic4kjh/SKILL.md"
    });
    let entry = parse_hermes_entry(&item).expect("entry");
    assert_eq!(
        entry.download_url,
        "https://raw.githubusercontent.com/browserbase/browse.sh/main/skills/plugandpay.com/account-management-ic4kjh/SKILL.md"
    );
}

#[test]
fn parse_hermes_entry_leaves_portal_source_url_undownloadable() {
    // ClawHub / LobeHub / skills.sh portals serve HTML, not raw markdown —
    // no direct download. download_url is empty; source_url is preserved so
    // install can point the user at the page. (#3741)
    for url in [
        "https://clawhub.ai/skills/agentkilox-code-audit",
        "https://lobehub.com/agent/9-somboon",
        "https://skills.sh/sickn33/antigravity-awesome-skills/00-andruia-consultant",
    ] {
        let item = json!({
            "name": "portal-skill",
            "description": "x",
            "category": "other",
            "source": "ClawHub",
            "sourceUrl": url
        });
        let entry = parse_hermes_entry(&item).expect("entry");
        assert_eq!(
            entry.download_url, "",
            "portal url must not be downloadable: {url}"
        );
        assert_eq!(entry.source_url.as_deref(), Some(url));
    }
}

#[test]
fn download_url_from_source_url_rejects_non_github_and_malformed() {
    assert_eq!(
        download_url_from_source_url("https://lobehub.com/agent/x"),
        None
    );
    // GitHub URL missing the branch/path tail.
    assert_eq!(
        download_url_from_source_url("https://github.com/owner/repo"),
        None
    );
    // Unknown ref kind.
    assert_eq!(
        download_url_from_source_url("https://github.com/o/r/raw/main/x"),
        None
    );
}

#[tokio::test]
async fn install_from_catalog_errors_for_portal_skill_without_download() {
    // A portal-only entry (empty download_url) must fail fast with an
    // actionable message naming the source + page — never fetch a 404. (#3741)
    let tmp = tempfile::tempdir().unwrap();
    let entry = parse_hermes_entry(&json!({
        "name": "code-audit",
        "description": "x",
        "category": "other",
        "source": "ClawHub",
        "sourceUrl": "https://clawhub.ai/skills/agentkilox-code-audit"
    }))
    .expect("entry");
    assert_eq!(entry.download_url, "");

    let err = install_from_catalog(tmp.path(), &entry)
        .await
        .expect_err("portal skill cannot install");
    assert!(err.contains("ClawHub"), "names the source: {err}");
    assert!(
        err.contains("https://clawhub.ai/skills/agentkilox-code-audit"),
        "links the source page: {err}"
    );
}

#[test]
fn parse_catalog_json_rejects_invalid_payloads() {
    let error = parse_catalog_json("{").expect_err("invalid json");
    assert!(error.contains("invalid catalog json"));
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

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;

const CACHE_DIR_ENV: &str = "OPENHUMAN_SKILL_REGISTRY_CACHE_DIR";

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::openhuman::skills::catalog::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn sample_entry() -> CatalogEntry {
    parse_hermes_entry(&json!({
        "name": "apple-notes",
        "description": "Manage Apple Notes",
        "category": "apple",
        "source": "built-in",
        "docsPath": "bundled/apple/apple-apple-notes"
    }))
    .expect("entry")
}

#[tokio::test]
async fn fresh_cache_skips_fetch() {
    let _env = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var(CACHE_DIR_ENV, tmp.path());
    store::save_catalog_cache(&[sample_entry()]);

    let called = Arc::new(AtomicBool::new(false));
    let called_in = called.clone();
    let entries = browse_catalog_with(false, StaleMode::Allow, move || async move {
        called_in.store(true, AtomicOrdering::SeqCst);
        Ok(Vec::new())
    })
    .await
    .unwrap();

    assert_eq!(entries.len(), 1);
    assert!(
        !called.load(AtomicOrdering::SeqCst),
        "fetcher must not run when the cache is fresh"
    );

    store::clear_cache();
    std::env::remove_var(CACHE_DIR_ENV);
}

#[tokio::test]
async fn concurrent_cache_miss_coalesces_to_single_fetch() {
    let _env = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var(CACHE_DIR_ENV, tmp.path());
    store::clear_cache();

    let calls = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let calls = calls.clone();
        handles.push(tokio::spawn(async move {
            browse_catalog_with(false, StaleMode::Allow, move || async move {
                calls.fetch_add(1, AtomicOrdering::SeqCst);
                // Mimic the slow upstream so the other callers queue on the
                // single-flight lock instead of each starting a fetch.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let entries = vec![sample_entry()];
                store::save_catalog_cache(&entries);
                Ok(entries)
            })
            .await
        }));
    }

    for handle in handles {
        let entries = handle.await.unwrap().unwrap();
        assert_eq!(entries.len(), 1, "every caller receives the catalog");
    }
    assert_eq!(
        calls.load(AtomicOrdering::SeqCst),
        1,
        "four concurrent cache-miss callers must trigger exactly one fetch"
    );

    store::clear_cache();
    std::env::remove_var(CACHE_DIR_ENV);
}

/// Write a cache file with an explicit `fetched_at_epoch` (epoch 1 => stale).
fn write_cache_at(dir: &std::path::Path, entries: Vec<CatalogEntry>, epoch: u64) {
    let cache = store::CatalogCache {
        entries,
        fetched_at_epoch: epoch,
    };
    std::fs::write(
        dir.join("cache.json"),
        serde_json::to_string(&cache).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn browse_serves_stale_without_a_foreground_fetch() {
    let _env = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var(CACHE_DIR_ENV, tmp.path());
    write_cache_at(tmp.path(), vec![sample_entry()], 1); // epoch 1 => stale

    // Pin REFRESHING so the background revalidation no-ops (no real network).
    REFRESHING.store(true, AtomicOrdering::SeqCst);
    let called = Arc::new(AtomicBool::new(false));
    let called_in = called.clone();
    let entries = browse_catalog_with(false, StaleMode::Allow, move || async move {
        called_in.store(true, AtomicOrdering::SeqCst);
        Ok(Vec::new())
    })
    .await
    .unwrap();
    REFRESHING.store(false, AtomicOrdering::SeqCst);

    assert_eq!(entries.len(), 1, "browse returns the stale entry");
    assert!(
        !called.load(AtomicOrdering::SeqCst),
        "browse must serve stale without a foreground fetch"
    );

    store::clear_cache();
    std::env::remove_var(CACHE_DIR_ENV);
}

#[tokio::test]
async fn search_rejects_stale_and_fetches_fresh() {
    let _env = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var(CACHE_DIR_ENV, tmp.path());
    write_cache_at(tmp.path(), vec![sample_entry()], 1); // stale: 1 entry

    let called = Arc::new(AtomicBool::new(false));
    let called_in = called.clone();
    let entries = browse_catalog_with(false, StaleMode::Reject, move || async move {
        called_in.store(true, AtomicOrdering::SeqCst);
        let fresh = vec![sample_entry(), sample_entry()];
        store::save_catalog_cache(&fresh);
        Ok(fresh)
    })
    .await
    .unwrap();

    assert!(
        called.load(AtomicOrdering::SeqCst),
        "a fresh (search) read must not be satisfied by a stale cache"
    );
    assert_eq!(
        entries.len(),
        2,
        "returns the freshly fetched catalog, not the stale one"
    );

    store::clear_cache();
    std::env::remove_var(CACHE_DIR_ENV);
}
