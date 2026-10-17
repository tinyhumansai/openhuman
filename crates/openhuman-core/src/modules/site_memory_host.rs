//! Host side of `ai.tinyhumans.tinycomputer.SiteMemoryHost`: where the
//! TinyComputer module keeps what finished browser tasks learned about a site.
//!
//! The module owns the rules (reuse, fact-value filtering, expiry, size
//! limits); the host owns only the bytes: one storage value per site (`tsd.kv`
//! owner `computer`, namespace `site_memory`; once
//! `<workspace>/state/computer/sites/<site>.json`). Site memory goes to the module and nowhere else: this file never
//! publishes it to the event bus, the agent's memory or a prompt.
//!
//! The host also owns three policies the module cannot know: whether tasks
//! learn at all (`browser.learn_from_tasks`, switched live from Browser
//! settings), the 30-day sweep of site files nothing has changed, and the
//! forget epoch that stops a task running when its site was forgotten from
//! writing it back.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::UNIX_EPOCH;

use tinybus::ObjectPath;
use tinycomputer_bus::site_memory::{
    ForgetSiteRequest, ForgetSiteResponse, LoadSiteRequest, LoadSiteResponse, SaveSiteRequest,
    SaveSiteResponse, SiteMemory, INTERFACE, OBJECT_PATH,
};

/// Largest site file read or written: anything bigger was not written here.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// How long a site file is kept without a change.
const KEEP_SECS: u64 = 30 * 24 * 60 * 60;

/// The legacy directory site files lived in (imported once, then unused).
pub(super) fn sites_dir(workspace: &Path) -> PathBuf {
    workspace.join("state").join("computer").join("sites")
}

/// Whether `site` is a plain lower-cased host name and so a safe file stem.
fn plain_site(site: &str) -> bool {
    !site.is_empty()
        && site.len() <= 253
        && !site.starts_with('.')
        && !site.contains("..")
        && site
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// What is stored for one site: the module's memory and when it was saved.
#[derive(serde::Serialize, serde::Deserialize)]
struct SiteRecord {
    saved_ms: i64,
    memory: SiteMemory,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn storage_error(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(format!("{error:#}"))
}

/// The site memories of one workspace, one `tsd.kv` value per site (owner
/// `computer`, namespace `site_memory`, key the site) in the acting scope.
/// Before schema v2 each was `<workspace>/state/computer/sites/<site>.json`;
/// [`SiteStore::open`] imports those once and leaves them in place.
pub(super) struct SiteStore {
    kv: crate::storage::state_kv::StateKv,
}

impl SiteStore {
    pub(super) fn open(workspace: &Path) -> std::io::Result<Self> {
        let kv = crate::storage::state_kv::StateKv::open(workspace, "computer", "site_memory")
            .map_err(storage_error)?;
        let legacy = sites_dir(workspace);
        kv.import_once("sites_dir", || import_legacy(&kv, &legacy))
            .map_err(storage_error)?;
        Ok(Self { kv })
    }

    fn name(site: &str) -> std::io::Result<&str> {
        if plain_site(site) {
            Ok(site)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "site is not a plain host name",
            ))
        }
    }

    /// `site`'s memory; nothing stored is empty, and an unreadable value is
    /// dropped and read as empty.
    pub(super) async fn load(&self, site: &str) -> std::io::Result<SiteMemory> {
        let name = Self::name(site)?;
        match self.kv.get::<SiteRecord>(name) {
            Ok(record) => Ok(record.map(|record| record.memory).unwrap_or_default()),
            Err(error) => {
                tracing::warn!(%error, "[site-memory-host] unreadable site record dropped");
                self.kv.delete(name).map_err(storage_error)?;
                Ok(SiteMemory::default())
            }
        }
    }

    /// Replaces `site`'s memory; an empty one removes the record. Returns
    /// whether the store holds the memory as sent.
    pub(super) async fn save(&self, site: &str, memory: &SiteMemory) -> std::io::Result<bool> {
        let name = Self::name(site)?;
        if memory.is_empty() {
            self.kv.delete(name).map_err(storage_error)?;
            return Ok(true);
        }
        let record = SiteRecord {
            saved_ms: now_ms(),
            memory: memory.clone(),
        };
        let bytes = serde_json::to_vec(&record).map_err(std::io::Error::other)?;
        if u64::try_from(bytes.len()).map_or(true, |n| n > MAX_FILE_BYTES) {
            tracing::warn!("[site-memory-host] site memory over the size limit; not saved");
            return Ok(false);
        }
        self.kv.put(name, &record).map_err(storage_error)?;
        Ok(true)
    }

    /// Forgets `site`, or every site; returns how many were forgotten.
    pub(super) async fn forget(&self, site: Option<&str>) -> std::io::Result<u64> {
        if let Some(site) = site {
            let kept = self.kv.delete(Self::name(site)?).map_err(storage_error)?;
            return Ok(u64::from(kept));
        }
        let mut forgotten = 0;
        for (name, _) in self
            .kv
            .list::<serde_json::Value>("")
            .map_err(storage_error)?
        {
            if self.kv.delete(&name).map_err(storage_error)? {
                forgotten += 1;
            }
        }
        Ok(forgotten)
    }

    /// Removes the sites nothing changed for [`KEEP_SECS`]: every entry in
    /// them has expired.
    pub(super) fn sweep(&self) {
        let cutoff = now_ms() - i64::try_from(KEEP_SECS).unwrap_or(i64::MAX) * 1000;
        let listed = match self.kv.list::<serde_json::Value>("") {
            Ok(listed) => listed,
            Err(error) => {
                tracing::warn!(%error, "[site-memory-host] sweep could not list sites");
                return;
            }
        };
        for (name, value) in listed {
            let saved = value.get("saved_ms").and_then(serde_json::Value::as_i64);
            if saved.is_some_and(|saved| saved <= cutoff) {
                match self.kv.delete(&name) {
                    Ok(_) => tracing::debug!(
                        "[site-memory-host] removed a site record unchanged for 30 days"
                    ),
                    Err(error) => {
                        tracing::warn!(%error, "[site-memory-host] old site record not removed");
                    }
                }
            }
        }
    }
}

/// Copies the legacy `<site>.json` files of `dir`, dated by their last change
/// so the 30-day sweep treats them as before. An existing record wins; a file
/// that does not parse, or a copy set aside, is skipped.
fn import_legacy(
    kv: &crate::storage::state_kv::StateKv,
    dir: &Path,
) -> anyhow::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(site) = name.strip_suffix(".json").filter(|site| plain_site(site)) else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            continue;
        }
        let memory = match std::fs::read(entry.path())
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(serde_json::from_slice::<SiteMemory>(&bytes)?))
        {
            Ok(memory) => memory,
            Err(error) => {
                tracing::warn!(%error, "[site-memory-host] unreadable legacy site file skipped");
                continue;
            }
        };
        let saved_ms = metadata
            .modified()
            .ok()
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or_else(now_ms, |since| {
                i64::try_from(since.as_millis()).unwrap_or_else(|_| now_ms())
            });
        kv.insert_if_absent(site, &SiteRecord { saved_ms, memory })?;
    }
    Ok(())
}

#[derive(Clone)]
struct SiteMemoryHost;

async fn config() -> tinybus::Result<crate::config::Config> {
    crate::core::runtime::DispatchCtx::host_default()
        .load_config()
        .await
        .map_err(|e| method_error(format!("config unavailable: {e}")))
}

/// Serializes changes to site files and holds the forget epochs: for each
/// workspace, the number of times a site was forgotten in it.
fn guard() -> &'static tokio::sync::Mutex<HashMap<PathBuf, u64>> {
    static GUARD: OnceLock<tokio::sync::Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
    GUARD.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// Learning switched in Browser settings since the process started, by
/// workspace.
fn switched() -> MutexGuard<'static, HashMap<PathBuf, bool>> {
    static SWITCHED: OnceLock<Mutex<HashMap<PathBuf, bool>>> = OnceLock::new();
    SWITCHED
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Records that Browser settings switched learning on or off for
/// `workspace`. A tool keeps the config it was built with, and a channel's
/// agent is built once, so the switch outranks that config.
pub(crate) fn note_switch(workspace: &Path, on: bool) {
    switched().insert(workspace.to_path_buf(), on);
}

/// Whether tasks under `config` learn: the latest switch in Browser
/// settings, else `browser.learn_from_tasks`.
pub(crate) fn learning(config: &crate::config::Config) -> bool {
    switched()
        .get(&config.workspace_dir)
        .copied()
        .unwrap_or(config.browser.learn_from_tasks)
}

/// The site `url` belongs to: its host, lower-cased and without a leading
/// `www.`; `None` for an address that is not `http(s)` or names no plain
/// host. The name a task's `StartTask.site` carries.
#[must_use]
pub fn site_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    site_named(parsed.host_str()?)
}

/// `host` as a site name, or `None` when it is not a plain host name.
fn site_named(host: &str) -> Option<String> {
    let host = host.trim().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    plain_site(host).then(|| host.to_owned())
}

/// Forgets what was learned about `site` (a host or an address), or about
/// every site when `None`, also by the tasks still running there. Returns how
/// many sites were forgotten.
///
/// # Errors
///
/// Returns an I/O error when a site's file exists and cannot be removed.
pub async fn forget(config: &crate::config::Config, site: Option<&str>) -> std::io::Result<usize> {
    let site = match site {
        Some(named) => match site_of(named).or_else(|| site_named(named)) {
            Some(site) => Some(site),
            None => return Ok(0),
        },
        None => None,
    };
    let mut epochs = guard().lock().await;
    // A task still running there would write it all back when it ends; its
    // save now carries a stale epoch.
    let epoch = epochs.entry(config.workspace_dir.clone()).or_insert(0);
    *epoch += 1;
    let epoch = *epoch;
    let store = SiteStore::open(&config.workspace_dir)?;
    let forgotten = store.forget(site.as_deref()).await?;
    tracing::debug!(forgotten, epoch, "[site-memory-host] forgot sites");
    Ok(usize::try_from(forgotten).unwrap_or(usize::MAX))
}

/// `Load`, under `config`.
async fn load_site(
    config: &crate::config::Config,
    request: LoadSiteRequest,
) -> std::io::Result<LoadSiteResponse> {
    let epochs = guard().lock().await;
    let epoch = epochs.get(&config.workspace_dir).copied().unwrap_or(0);
    let store = SiteStore::open(&config.workspace_dir)?;
    store.sweep();
    let memory = store.load(&request.site).await?;
    Ok(LoadSiteResponse { memory, epoch })
}

/// `Save`, under `config`: refused when learning is off or the site was
/// forgotten since it was loaded.
async fn save_site(
    config: &crate::config::Config,
    request: SaveSiteRequest,
) -> std::io::Result<SaveSiteResponse> {
    let epochs = guard().lock().await;
    let epoch = epochs.get(&config.workspace_dir).copied().unwrap_or(0);
    if request.epoch != epoch || !learning(config) {
        tracing::debug!(
            stale = request.epoch != epoch,
            "[site-memory-host] save refused"
        );
        return Ok(SaveSiteResponse { saved: false });
    }
    let saved = SiteStore::open(&config.workspace_dir)?
        .save(&request.site, &request.memory)
        .await?;
    Ok(SaveSiteResponse { saved })
}

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.SiteMemoryHost")]
impl SiteMemoryHost {
    /// One site's memory; empty for a site with nothing kept.
    async fn load(&self, request: LoadSiteRequest) -> tinybus::Result<LoadSiteResponse> {
        load_site(&config().await?, request)
            .await
            .map_err(method_error)
    }

    /// Replaces one site's memory, unless learning is off or the site was
    /// forgotten since it was loaded.
    async fn save(&self, request: SaveSiteRequest) -> tinybus::Result<SaveSiteResponse> {
        save_site(&config().await?, request)
            .await
            .map_err(method_error)
    }

    /// Forgets one site, or every site.
    async fn forget(&self, request: ForgetSiteRequest) -> tinybus::Result<ForgetSiteResponse> {
        let forgotten = forget(&config().await?, request.site.as_deref())
            .await
            .map_err(method_error)?;
        Ok(ForgetSiteResponse {
            forgotten: forgotten as u64,
        })
    }
}

fn method_error(error: impl std::fmt::Display) -> tinybus::Error {
    tinybus::Error::MethodFailed {
        name: "ai.tinyhumans.tinycomputer.Error.SiteMemoryHost".to_string(),
        message: error.to_string(),
    }
}

pub(super) async fn install(connection: &tinybus::Connection) -> tinybus::Result<()> {
    connection
        .serve_at(ObjectPath::new(OBJECT_PATH)?, SiteMemoryHost)
        .await?;
    // The module reaches the host by this name (`optional` in its manifest).
    connection.request_name(INTERFACE).await?;
    tracing::debug!(interface = INTERFACE, "[site-memory-host] served");
    Ok(())
}

#[cfg(test)]
#[path = "site_memory_host_tests.rs"]
mod tests;
