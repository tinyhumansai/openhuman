use super::*;
use tinycomputer_bus::site_memory::SavedHint;

fn memory() -> SiteMemory {
    serde_json::from_value(serde_json::json!({
        "hints": [{"hint": {"app": "browser", "key": "k", "role": "button", "name": "Go"}, "ok_at": 7}]
    }))
    .unwrap()
}

fn store() -> (tempfile::TempDir, SiteStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = SiteStore::open(dir.path()).unwrap();
    (dir, store)
}

#[tokio::test]
async fn saved_memory_loads_back_and_empty_save_removes_the_site() {
    let (_dir, store) = store();
    assert!(store.load("example.com").await.unwrap().is_empty());
    assert!(store.save("example.com", &memory()).await.unwrap());
    let back = store.load("example.com").await.unwrap();
    assert_eq!(back, memory());
    let _: &SavedHint = &back.hints[0];
    assert!(store
        .save("example.com", &SiteMemory::default())
        .await
        .unwrap());
    assert!(store.load("example.com").await.unwrap().is_empty());
}

#[tokio::test]
async fn a_site_that_is_not_a_plain_host_is_refused() {
    let (_dir, store) = store();
    for bad in ["", "../x", "a/b", "A.com", ".hidden", "a..b"] {
        assert!(store.save(bad, &memory()).await.is_err(), "{bad}");
        assert!(store.load(bad).await.is_err(), "{bad}");
    }
}

#[tokio::test]
async fn an_unreadable_record_is_dropped_and_read_as_empty() {
    let (dir, store) = store();
    crate::storage::state_kv::StateKv::open(dir.path(), "computer", "site_memory")
        .unwrap()
        .put("x.com", &"not a site record")
        .unwrap();
    assert!(store.load("x.com").await.unwrap().is_empty());
    assert_eq!(store.forget(Some("x.com")).await.unwrap(), 0);
}

#[tokio::test]
async fn forgetting_every_site_counts_distinct_sites() {
    let (_dir, store) = store();
    store.save("a.com", &memory()).await.unwrap();
    store.save("b.com", &memory()).await.unwrap();
    assert_eq!(store.forget(None).await.unwrap(), 2);
    assert_eq!(store.forget(None).await.unwrap(), 0);
}

#[tokio::test]
async fn legacy_site_files_are_imported_once_dated_by_their_last_change_and_left_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let sites = sites_dir(dir.path());
    std::fs::create_dir_all(&sites).unwrap();
    let write = |name: &str, body: &[u8]| std::fs::write(sites.join(name), body).unwrap();
    write("fresh.com.json", &serde_json::to_vec(&memory()).unwrap());
    write("old.com.json", &serde_json::to_vec(&memory()).unwrap());
    write("broken.com.json", b"not json");
    write("aside.com.json.corrupt", b"not json");
    let aged = std::time::SystemTime::now() - std::time::Duration::from_secs(KEEP_SECS + 60);
    std::fs::File::options()
        .write(true)
        .open(sites.join("old.com.json"))
        .unwrap()
        .set_modified(aged)
        .unwrap();

    let store = SiteStore::open(dir.path()).unwrap();
    assert_eq!(store.load("fresh.com").await.unwrap(), memory());
    assert!(store.load("broken.com").await.unwrap().is_empty());
    assert!(
        sites.join("fresh.com.json").exists(),
        "the legacy file is not touched"
    );
    // The imported old site is swept like any other that nothing changed.
    store.sweep();
    assert!(store.load("old.com").await.unwrap().is_empty());
    assert_eq!(store.load("fresh.com").await.unwrap(), memory());
    // Forgetting is not undone by a later open importing the file again.
    assert_eq!(store.forget(Some("fresh.com")).await.unwrap(), 1);
    let reopened = SiteStore::open(dir.path()).unwrap();
    assert!(reopened.load("fresh.com").await.unwrap().is_empty());
}

#[test]
fn site_memory_is_never_published_to_agent_memory_or_prompts() {
    // The adapter must stay a pure file store: no event bus, memory or prompt
    // surface may be named in its source.
    let source = include_str!("site_memory_host.rs");
    let code = source.split("#[cfg(test)]").next().unwrap();
    for forbidden in ["BUS.", "crate::memory", "prompt", "DomainEvent"] {
        let hits = code
            .lines()
            .filter(|l| !l.trim_start().starts_with("//") && l.contains(forbidden))
            .count();
        assert_eq!(hits, 0, "{forbidden}");
    }
}

fn workspace_config(dir: &tempfile::TempDir, learn: bool) -> crate::config::Config {
    let mut config = crate::config::Config {
        workspace_dir: dir.path().to_path_buf(),
        ..crate::config::Config::default()
    };
    config.browser.learn_from_tasks = learn;
    config
}

async fn load(config: &crate::config::Config, site: &str) -> LoadSiteResponse {
    load_site(config, LoadSiteRequest { site: site.into() })
        .await
        .unwrap()
}

async fn save(config: &crate::config::Config, site: &str, epoch: u64) -> bool {
    save_site(
        config,
        SaveSiteRequest {
            site: site.into(),
            memory: memory(),
            epoch,
        },
    )
    .await
    .unwrap()
    .saved
}

#[tokio::test]
async fn a_save_under_the_loaded_epoch_is_kept_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let config = workspace_config(&dir, true);
    let loaded = load(&config, "keep.example").await;
    assert!(save(&config, "keep.example", loaded.epoch).await);
    assert_eq!(load(&config, "keep.example").await.memory, memory());
}

#[tokio::test]
async fn a_site_forgotten_after_it_was_loaded_is_not_written_back() {
    let dir = tempfile::tempdir().unwrap();
    let config = workspace_config(&dir, true);
    let loaded = load(&config, "gone.example").await;
    assert!(save(&config, "gone.example", loaded.epoch).await);

    assert_eq!(
        forget(&config, Some("https://www.Gone.example/x"))
            .await
            .unwrap(),
        1
    );
    // The task that loaded before the forget still holds the old epoch.
    assert!(!save(&config, "gone.example", loaded.epoch).await);
    assert!(load(&config, "gone.example").await.memory.is_empty());
    // One that loads after it saves as usual.
    let fresh = load(&config, "gone.example").await;
    assert!(fresh.epoch > loaded.epoch);
    assert!(save(&config, "gone.example", fresh.epoch).await);
}

#[tokio::test]
async fn forgetting_every_site_counts_them_and_an_address_that_names_no_site_forgets_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let config = workspace_config(&dir, true);
    for site in ["a.example", "b.example"] {
        let epoch = load(&config, site).await.epoch;
        assert!(save(&config, site, epoch).await);
    }
    assert_eq!(
        forget(&config, Some("file:///etc/passwd")).await.unwrap(),
        0
    );
    assert_eq!(forget(&config, None).await.unwrap(), 2);
}

#[tokio::test]
async fn nothing_is_kept_while_learning_is_off_and_a_settings_switch_outranks_the_config() {
    let dir = tempfile::tempdir().unwrap();
    let off = workspace_config(&dir, false);
    let epoch = load(&off, "off.example").await.epoch;
    assert!(!save(&off, "off.example", epoch).await);

    note_switch(dir.path(), true);
    assert!(learning(&off));
    let epoch = load(&off, "off.example").await.epoch;
    assert!(save(&off, "off.example", epoch).await);
    note_switch(dir.path(), false);
    let epoch = load(&off, "off.example").await.epoch;
    assert!(!save(&off, "off.example", epoch).await);
}

#[tokio::test]
async fn a_load_sweeps_sites_nothing_changed_for_thirty_days() {
    let dir = tempfile::tempdir().unwrap();
    let config = workspace_config(&dir, true);
    let epoch = load(&config, "old.example").await.epoch;
    assert!(save(&config, "old.example", epoch).await);
    assert!(save(&config, "new.example", epoch).await);
    crate::storage::state_kv::StateKv::open(dir.path(), "computer", "site_memory")
        .unwrap()
        .put(
            "old.example",
            &SiteRecord {
                saved_ms: now_ms() - i64::try_from(KEEP_SECS + 60).unwrap() * 1000,
                memory: memory(),
            },
        )
        .unwrap();

    assert_eq!(load(&config, "new.example").await.memory, memory());
    assert!(load(&config, "old.example").await.memory.is_empty());
    assert_eq!(load(&config, "new.example").await.memory, memory());
}

#[test]
fn a_site_is_its_host_without_www() {
    assert_eq!(
        site_of("https://www.Amazon.in/s?k=x").as_deref(),
        Some("amazon.in")
    );
    assert_eq!(
        site_of(" http://shop.test:8080/cart ").as_deref(),
        Some("shop.test")
    );
    for none in [
        "ftp://x.test/",
        "file:///etc/passwd",
        "not a url",
        "http://[::1]/",
    ] {
        assert_eq!(site_of(none), None, "{none}");
    }
}
