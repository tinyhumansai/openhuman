use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use openhuman_core::openhuman::config::rpc as config_rpc;
use openhuman_core::openhuman::memory_sources::readers::SourceReader;
use openhuman_core::openhuman::memory_sources::{
    self, ContentType, MemorySourceEntry, MemorySourcePatch, SourceKind,
};
use tempfile::{Builder, TempDir};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }

    fn set(key: &'static str, value: impl Into<String>) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value.into()) };
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

struct Harness {
    _tmp: TempDir,
    root: PathBuf,
    _guards: Vec<EnvGuard>,
}

impl Harness {
    async fn config(&self) -> openhuman_core::openhuman::config::Config {
        config_rpc::load_config_with_timeout()
            .await
            .expect("isolated config should load")
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("target dir");
    Builder::new()
        .prefix("memory-sources-closure-round23-")
        .tempdir_in("target")
        .expect("tempdir")
}

fn setup() -> Harness {
    let tmp = tempdir();
    let root = tmp.path().join("openhuman");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::write(
        root.join("config.toml"),
        r#"api_url = "http://127.0.0.1:9"
default_model = "round23-memory-sources"
default_temperature = 0.2

[secrets]
encrypt = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0
auto_save = false

[memory_tree]
embedding_strict = false
"#,
    )
    .expect("config");
    let guards = vec![
        EnvGuard::set_path("OPENHUMAN_WORKSPACE", &root),
        EnvGuard::set_path("HOME", tmp.path()),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
        EnvGuard::unset("OPENHUMAN_API_URL"),
        EnvGuard::unset("BACKEND_URL"),
        EnvGuard::unset("VITE_BACKEND_URL"),
    ];
    Harness {
        _tmp: tmp,
        root,
        _guards: guards,
    }
}

fn source_entry(id: &str, kind: SourceKind) -> MemorySourceEntry {
    MemorySourceEntry {
        id: id.to_string(),
        kind,
        label: format!("{id} label"),
        enabled: true,
        toolkit: None,
        connection_id: None,
        path: None,
        glob: None,
        url: None,
        branch: None,
        paths: Vec::new(),
        query: None,
        since_days: None,
        max_items: None,
        max_commits: None,
        max_issues: None,
        max_prs: None,
        selector: None,
        max_tokens_per_sync: None,
        max_cost_per_sync_usd: None,
        sync_depth_days: None,
    }
}

#[tokio::test]
async fn round23_memory_sources_status_registry_and_readers_cover_remaining_edges() {
    let _lock = env_lock();
    let harness = setup();
    let config = harness.config().await;

    let folder_root = harness.root.join("docs");
    std::fs::create_dir_all(&folder_root).expect("docs");
    let folder = MemorySourceEntry {
        path: Some(folder_root.to_string_lossy().into_owned()),
        ..source_entry("round23-folder", SourceKind::Folder)
    };
    let added = memory_sources::add_source(folder.clone())
        .await
        .expect("add folder source");
    assert_eq!(added.id, "round23-folder");
    let duplicate = memory_sources::add_source(folder)
        .await
        .expect_err("duplicate source rejected");
    assert!(duplicate.contains("already exists"));

    let updated = memory_sources::update_source(
        "round23-folder",
        MemorySourcePatch {
            label: Some("Round23 Folder Updated".to_string()),
            enabled: Some(false),
            glob: Some("**/*.md".to_string()),
            ..MemorySourcePatch::default()
        },
    )
    .await
    .expect("update source");
    assert_eq!(updated.label, "Round23 Folder Updated");
    assert!(!updated.enabled);
    assert_eq!(updated.glob.as_deref(), Some("**/*.md"));

    let missing_update = memory_sources::update_source(
        "missing-round23",
        MemorySourcePatch {
            label: Some("Missing".to_string()),
            ..MemorySourcePatch::default()
        },
    )
    .await
    .expect_err("missing source rejected");
    assert!(missing_update.contains("source 'missing-round23' not found"));

    let invalid = memory_sources::add_source(MemorySourceEntry {
        url: None,
        ..source_entry("bad-rss", SourceKind::RssFeed)
    })
    .await
    .expect_err("invalid source rejected");
    assert!(invalid.contains("url is required"));

    let composio = memory_sources::upsert_composio_source(
        "gmail",
        "conn-round23-source-status",
        "Gmail Round23",
    )
    .await
    .expect("insert composio");
    let composio = memory_sources::update_source(
        &composio.id,
        MemorySourcePatch {
            enabled: Some(true),
            ..MemorySourcePatch::default()
        },
    )
    .await
    .expect("enable composio source");
    let status = memory_sources::status::source_status(&config, &composio)
        .await
        .expect("composio status");
    assert_eq!(status.source_id, composio.id);
    assert_eq!(status.chunks_synced, 0);
    assert_eq!(
        status.freshness,
        memory_sources::status::FreshnessLabel::Idle
    );

    let statuses = memory_sources::status::status_list(&config)
        .await
        .expect("status list");
    assert!(statuses
        .iter()
        .any(|status| status.source_id == composio.id));

    let enabled_composio = memory_sources::list_enabled_by_kind(SourceKind::Composio)
        .await
        .expect("enabled composio");
    assert_eq!(enabled_composio.len(), 1);
    assert_eq!(enabled_composio[0].id, composio.id);

    let composio_reader =
        openhuman_core::openhuman::memory_sources::readers::composio::ComposioReader;
    let items = composio_reader
        .list_items(&composio, &config)
        .await
        .expect("composio reader items");
    assert_eq!(items[0].id, "conn-round23-source-status");
    let content = composio_reader
        .read_item(&composio, "conn-round23-source-status", &config)
        .await
        .expect("composio reader content");
    assert_eq!(content.content_type, ContentType::Plaintext);
    assert!(content.body.contains("provider sync pipeline"));

    let twitter_reader = openhuman_core::openhuman::memory_sources::readers::twitter::TwitterReader;
    let missing_query = twitter_reader
        .list_items(
            &source_entry("tw-missing", SourceKind::TwitterQuery),
            &config,
        )
        .await
        .expect_err("missing twitter query rejected");
    assert!(missing_query.contains("non-empty query"));
    let configured_query = twitter_reader
        .list_items(
            &MemorySourceEntry {
                query: Some(" openhuman ".to_string()),
                since_days: Some(14),
                ..source_entry("tw-round23", SourceKind::TwitterQuery)
            },
            &config,
        )
        .await
        .expect_err("twitter credentials not configured");
    assert!(configured_query.contains("Query 'openhuman' is saved"));
    let read_err = twitter_reader
        .read_item(
            &source_entry("tw-round23", SourceKind::TwitterQuery),
            "tweet-1",
            &config,
        )
        .await
        .expect_err("twitter read not configured");
    assert!(read_err.contains("Individual tweet reading"));
}
