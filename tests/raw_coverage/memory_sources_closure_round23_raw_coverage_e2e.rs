use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::config::rpc as config_rpc;
use openhuman_core::openhuman::memory::sources::readers::SourceReader;
use openhuman_core::openhuman::memory::sources::{
    self as memory_sources, ContentType, MemorySourceEntry, MemorySourcePatch, SourceKind,
};
use tempfile::{Builder, TempDir};

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;
static MEMORY_SEAMS_INIT: OnceLock<()> = OnceLock::new();

fn ensure_memory_seams() {
    MEMORY_SEAMS_INIT.get_or_init(|| {
        std::thread::Builder::new()
            .name("round23-memory-source-seams".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
            })
            .expect("spawn round23 memory source seam installer")
            .join()
            .expect("round23 memory source seam installer panicked");
    });
}

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

