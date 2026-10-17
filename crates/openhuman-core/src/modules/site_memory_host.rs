//! Host side of `ai.tinyhumans.tinycomputer.SiteMemoryHost`: where the
//! TinyComputer module keeps what finished browser tasks learned about a site.
//!
//! The module owns the rules (reuse, fact-value filtering, expiry, size
//! limits); the host owns only the bytes: one file per site in
//! `<workspace>/state/computer/sites/<site>.json`, readable by its owner
//! alone. Site memory goes to the module and nowhere else: this file never
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

/// The directory site files live in.
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

/// The site files under one directory.
pub(super) struct SiteStore {
    dir: PathBuf,
}

impl SiteStore {
    pub(super) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, site: &str) -> std::io::Result<PathBuf> {
        if plain_site(site) {
            Ok(self.dir.join(format!("{site}.json")))
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "site is not a plain host name",
            ))
        }
    }

    /// `site`'s memory; a missing file is empty, and an unreadable one is set
    /// aside as `<site>.json.corrupt` and read as empty.
    pub(super) async fn load(&self, site: &str) -> std::io::Result<SiteMemory> {
        let path = self.path(site)?;
        let read = match tokio::fs::metadata(&path).await {
            Ok(m) if m.len() <= MAX_FILE_BYTES => tokio::fs::read(&path).await,
            Ok(_) => Err(std::io::Error::other("larger than a site file can be")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SiteMemory::default()),
            Err(e) => Err(e),
        };
        match read.and_then(|b| serde_json::from_slice(&b).map_err(std::io::Error::other)) {
            Ok(memory) => Ok(memory),
            Err(error) => {
                tracing::warn!(%error, "[site-memory-host] unreadable site file set aside");
                let _ = tokio::fs::rename(&path, path.with_extension("json.corrupt")).await;
                Ok(SiteMemory::default())
            }
        }
    }

    /// Replaces `site`'s memory; an empty one removes the file. Returns
    /// whether the file holds the memory as sent.
    pub(super) async fn save(&self, site: &str, memory: &SiteMemory) -> std::io::Result<bool> {
        let path = self.path(site)?;
        if memory.is_empty() {
            remove(&path).await?;
            return Ok(true);
        }
        let bytes = serde_json::to_vec_pretty(memory).map_err(std::io::Error::other)?;
        if u64::try_from(bytes.len()).map_or(true, |n| n > MAX_FILE_BYTES) {
            tracing::warn!("[site-memory-host] site memory over the file limit; not saved");
            return Ok(false);
        }
        write_private(&path, &bytes).await?;
        Ok(true)
    }

    /// Forgets `site`, or every site; returns how many were forgotten.
    pub(super) async fn forget(&self, site: Option<&str>) -> std::io::Result<u64> {
        if let Some(site) = site {
            let path = self.path(site)?;
            let aside = remove(&path.with_extension("json.corrupt")).await?;
            remove(&path.with_extension("json.tmp")).await?;
            let kept = remove(&path).await?;
            return Ok(u64::from(kept || aside));
        }
        let mut entries = match tokio::fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let mut sites = std::collections::BTreeSet::new();
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_file() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some(site) = name
                    .strip_suffix(".json")
                    .or_else(|| name.strip_suffix(".json.corrupt"))
                {
                    sites.insert(site.to_owned());
                }
                remove(&entry.path()).await?;
            }
        }
        Ok(sites.len() as u64)
    }
}

async fn remove(path: &Path) -> std::io::Result<bool> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Writes through a temporary file, in a directory and file only their owner
/// can read: plans and elements tell where a person browses.
async fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).await?;
        }
    }
    let staged = path.with_extension("json.tmp");
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&staged).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .await?;
    }
    file.write_all(bytes).await?;
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&staged, path).await
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
    let store = SiteStore::new(sites_dir(&config.workspace_dir));
    let forgotten = store.forget(site.as_deref()).await?;
    tracing::debug!(forgotten, epoch, "[site-memory-host] forgot sites");
    Ok(usize::try_from(forgotten).unwrap_or(usize::MAX))
}

/// Removes the site files nothing changed for [`KEEP_SECS`]: every entry in
/// them has expired, as has a copy set aside or a write left behind.
async fn sweep(dir: &Path) {
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        let changed = metadata
            .modified()
            .ok()
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or(now, |since| since.as_secs());
        if metadata.is_file() && changed.saturating_add(KEEP_SECS) <= now {
            match remove(&entry.path()).await {
                Ok(_) => {
                    tracing::debug!("[site-memory-host] removed a site file unchanged for 30 days");
                }
                Err(error) => {
                    tracing::warn!(%error, "[site-memory-host] old site file not removed");
                }
            }
        }
    }
}

/// `Load`, under `config`.
async fn load_site(
    config: &crate::config::Config,
    request: LoadSiteRequest,
) -> std::io::Result<LoadSiteResponse> {
    let epochs = guard().lock().await;
    let epoch = epochs.get(&config.workspace_dir).copied().unwrap_or(0);
    let dir = sites_dir(&config.workspace_dir);
    sweep(&dir).await;
    let memory = SiteStore::new(dir).load(&request.site).await?;
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
    let saved = SiteStore::new(sites_dir(&config.workspace_dir))
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
