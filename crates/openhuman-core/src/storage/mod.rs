//! The process's storage backend on the `tinystoragedrivers` ports.
//!
//! One URL picks where all durable state lives: `OPENHUMAN_STORAGE_URL`, else
//! `[storage] url` in `config.toml` ([`crate::config::StorageConfig`]), else
//! the default, `sqlite:<workspace>/storage` ([`config::default_url`]). There
//! is no second layout: the per-domain `.db` files, JSONL transcripts and JSON
//! stores older installs wrote are imported once, read-only, by [`import`].
//!
//! The host opens the URL once at startup ([`open`]) and installs it
//! ([`install_for_url`]); domains reach it through [`installed`] and bind
//! their records to an agent with [`scope_for_agent`]. The session store is
//! TinyAgents' `DriverSessionStores` over this backend, so transcripts, turn
//! states, records, the run ledger and journals live in it, one scope per
//! agent. The default follows the workspace, so a different workspace is a
//! different database: [`rebind`] re-resolves and reinstalls it when the
//! desktop switches user.
//!
//! With nothing installed (a unit test, an embedder that skipped the host
//! boot) a domain falls back to the same default for its own workspace,
//! opened lazily ([`local`]); a SaaS process never does and refuses instead.
//!
//! Drivers are Cargo features of this crate: `storage-sqlite`,
//! `storage-mongodb`, `storage-file` (memory is always available). A URL for a
//! driver the build does not carry fails at [`open`] naming the feature, so a
//! misconfigured deployment stops at boot instead of at its first write.

pub mod agents;
pub mod blob_sweep;
pub mod boot;
pub mod config;
mod declaring_ports;
pub mod documents;
pub mod fence;
pub mod fenced_backend;
mod fenced_ports;
mod host_importers;
pub mod import;
pub mod import_state;
mod inspection;
pub mod journal_maintenance;
pub mod layout;
pub mod lease;
mod lease_documents;
mod lease_local;
pub mod local;
pub mod rebind;
pub mod schema;
pub mod schema_support;
pub(crate) use inspection::probe_directory_writable;
pub mod scope;
#[cfg(feature = "legacy-import")]
pub mod scope_move;
pub mod secrets;
pub mod sessions;
pub mod state_kv;
#[cfg(test)]
pub mod test_support;

pub use scope::{
    scope_for_agent, scope_for_module, scope_for_profile, tenant_scope, ParsedScope, Principal,
    Scope, Tenant,
};

use std::future::Future;
use std::sync::{mpsc, Arc, LazyLock, Mutex, OnceLock, RwLock};

/// SQLite bindings re-exported by the storage driver crate, for the two
/// consumers that open a SQLite file that is not a storage backend: the
/// read-only legacy importers ([`import`], which open the old per-domain files
/// on a scratch copy) and the vault watcher's per-machine mtime table
/// (`config/workspace/state.rs`). `SqliteNative` cannot stand in for either: it
/// opens the driver's own database with the driver's schema, not an arbitrary
/// foreign file, and its `run_blocking` hands the closure a `rusqlite`
/// connection anyway. The core has no `rusqlite` dependency of its own,
/// tinystoragedrivers owns the one copy; the re-export goes when the importers
/// do (the one-time legacy import window closes). Re-checked for #7768: both
/// consumers are still present, so it stays.
pub use tinystoragedrivers::{
    CollectionSpec, Precondition, StorageBackend, StorageConfig as StorageUrl, StorageError,
};
pub use tinystoragedrivers_api::{DocumentStore, Storage};

/// Read-only legacy importers use SQLite to consume historical files. The
/// process storage API remains the `StorageBackend` facade above.
#[cfg(feature = "legacy-import")]
pub use tinystoragedrivers_connect::sqlite::rusqlite;

/// Typed conveniences for document consumers, including a bounded cursor
/// walk for callers that need every matching record.
#[async_trait::async_trait]
pub trait DocumentStoreExt: tinystoragedrivers_api::DocumentStoreExt {
    /// Read every record matching `query`, following the driver's cursor until
    /// the final page. Each request is bounded to 256 documents.
    async fn query_all(
        &self,
        collection: &str,
        query: &tinystoragedrivers_types::Query,
    ) -> Result<Vec<tinystoragedrivers_types::Versioned<serde_json::Value>>, StorageError> {
        let mut page_query = query.clone();
        page_query.limit = Some(256);
        page_query.cursor = None;
        let mut records = Vec::new();
        loop {
            let page = self.query(collection, &page_query).await?;
            records.extend(page.items);
            let Some(cursor) = page.next else {
                break;
            };
            page_query.cursor = Some(cursor);
        }
        Ok(records)
    }
}

impl<T: tinystoragedrivers_api::DocumentStoreExt + ?Sized> DocumentStoreExt for T {}

type StorageTask = Box<dyn FnOnce(&tokio::runtime::Runtime) + Send>;

/// Dedicated runtime for synchronous domain surfaces that bridge to the
/// asynchronous storage ports. Keeping it on its own thread also permits a
/// synchronous facade to be called while the caller is on a current-thread
/// Tokio runtime.
struct Blocking {
    tasks: mpsc::Sender<StorageTask>,
}

thread_local! {
    static ON_STORAGE_RUNTIME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl Blocking {
    fn new() -> anyhow::Result<Self> {
        let (tasks, receiver) = mpsc::channel::<StorageTask>();
        let (started, ready) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("openhuman-storage".to_owned())
            .spawn(move || {
                // The bridge thread can be idle in `receiver.recv()` between
                // sync calls. A current-thread runtime would stop polling its
                // background tasks during that interval, so use a small
                // multithread runtime dedicated to storage work.
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .on_thread_start(|| {
                        // Futures spawned from a bridge request may be
                        // polled on runtime workers rather than the owner
                        // thread, and must obey the same re-entrancy guard.
                        ON_STORAGE_RUNTIME.with(|active| active.set(true));
                    })
                    .build();
                match runtime {
                    Ok(runtime) => {
                        if started.send(Ok(())).is_err() {
                            return;
                        }
                        while let Ok(task) = receiver.recv() {
                            task(&runtime);
                        }
                    }
                    Err(error) => {
                        let _ = started.send(Err(error.to_string()));
                    }
                }
            })?;
        ready
            .recv()
            .map_err(|error| anyhow::anyhow!("storage runtime failed to start: {error}"))?
            .map_err(anyhow::Error::msg)?;
        Ok(Self { tasks })
    }

    fn run<T, F>(&self, future: F) -> Result<T, StorageError>
    where
        F: Future<Output = Result<T, StorageError>> + Send + 'static,
        T: Send + 'static,
    {
        if ON_STORAGE_RUNTIME.with(std::cell::Cell::get) {
            return Err(StorageError::backend(
                "synchronous storage bridge cannot be re-entered from its own runtime",
            ));
        }
        let (reply, result) = mpsc::sync_channel(1);
        self.tasks
            .send(Box::new(move |runtime| {
                ON_STORAGE_RUNTIME.with(|active| active.set(true));
                let outcome = runtime.block_on(future);
                ON_STORAGE_RUNTIME.with(|active| active.set(false));
                let _ = reply.send(outcome);
            }))
            .map_err(|error| StorageError::backend(format!("storage bridge stopped: {error}")))?;
        result
            .recv()
            .map_err(|error| StorageError::backend(format!("storage bridge stopped: {error}")))?
    }
}

use crate::config::schema::storage::redact_url;
use crate::config::Config;

/// The environment variable that overrides `[storage] url`.
pub const STORAGE_URL_VAR: &str = "OPENHUMAN_STORAGE_URL";

/// A holder for one backend. The process has exactly one ([`BACKEND`]);
/// tests make their own, so they never change what other tests in the same
/// process see.
#[derive(Default)]
struct Slot(RwLock<Option<Arc<dyn StorageBackend>>>);

impl Slot {
    fn install(&self, backend: Arc<dyn StorageBackend>) -> Option<Arc<dyn StorageBackend>> {
        self.0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(backend)
    }

    fn installed(&self) -> Option<Arc<dyn StorageBackend>> {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn clear(&self) -> bool {
        self.0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .is_some()
    }
}

static BACKEND: LazyLock<Slot> = LazyLock::new(Slot::default);

/// The URL the installed backend was opened from, when the host installed it
/// by URL ([`install_for_url`]). [`rebind`] compares against it; a backend
/// installed without one (a test's, an embedder's own) is never rebound.
static INSTALLED_URL: Mutex<Option<String>> = Mutex::new(None);

fn set_installed_url(url: Option<String>) {
    *INSTALLED_URL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = url;
}

/// The URL the installed backend was opened from, if the host installed it by
/// URL.
pub fn installed_url() -> Option<String> {
    INSTALLED_URL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// The storage URL a host opens at startup: [`STORAGE_URL_VAR`], else
/// `config`'s `[storage] url`, else the default for `config`'s workspace
/// ([`config::default_url`]). `None` only in SaaS mode with nothing
/// configured: a SaaS process has no workspace-local state to default to.
pub fn configured_url(config: &Config) -> Option<String> {
    url_from(std::env::var(STORAGE_URL_VAR).ok(), config)
}

/// [`configured_url`] with the environment read made explicit, so the rule
/// is testable without mutating process-wide state.
pub fn url_from(env: Option<String>, config: &Config) -> Option<String> {
    match config::mode_from(env, config) {
        config::StorageMode::Url(url) => Some(url),
        config::StorageMode::Default if crate::core::runtime::mode::is_saas() => None,
        config::StorageMode::Default => Some(config::default_url(&config.workspace_dir)),
    }
}

/// The URL the operator named (`OPENHUMAN_STORAGE_URL`, else `[storage] url`),
/// with no default applied. For hosts with no workspace (a stateless embed)
/// that must be told where to store things.
pub fn explicit_url(config: &Config) -> Option<String> {
    match config::mode(config) {
        config::StorageMode::Url(url) => Some(url),
        config::StorageMode::Default => None,
    }
}

/// Parses and opens the backend `url` names, its writes fenced by the
/// process's lease fences ([`fence`]).
///
/// # Errors
///
/// An unparseable URL, a driver this build was compiled without (the error
/// names the Cargo feature), or a backend that cannot be reached.
pub async fn open(url: &str) -> Result<Arc<dyn StorageBackend>, StorageError> {
    let parsed = StorageUrl::parse(url)?;
    tracing::info!(
        target: "openhuman::storage",
        driver = parsed.driver(),
        url = %redact_url(url),
        "[storage] opening the configured backend"
    );
    // Every opened backend checks the process's lease fences on write; with
    // no fence registered (every single-user host) that is a pass-through.
    let backend =
        connect_on_storage_runtime(move || async move { tinystoragedrivers::open(&parsed).await })
            .await??;
    boot::prepare_and_migrate(&backend).await?;
    Ok(fenced_backend::wrap(backend))
}

/// The process's storage runtime: two worker threads that live as long as the
/// process. A driver connected here starts its connection-time background tasks
/// (MongoDB's connection pool and monitors) here, off the host runtime, so
/// callers parked on [`block_on`] cannot starve them. Cleanup tasks registered
/// by later object drops can run on the internal [`block_on`] runtime instead.
fn storage_runtime() -> Result<&'static tokio::runtime::Runtime, StorageError> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("openhuman-storage")
                .enable_all()
                .build()
                .map_err(|error| format!("cannot build the storage runtime: {error}"))
        })
        .as_ref()
        .map_err(|error| StorageError::backend(error.clone()))
}

/// Runs `connect` on the storage runtime and returns what it built.
///
/// # Errors
///
/// When the storage runtime cannot start or `connect` panics.
pub(crate) async fn connect_on_storage_runtime<F, Fut, T>(connect: F) -> Result<T, StorageError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let runtime = storage_runtime()?;
    tracing::debug!(
        target: "openhuman::storage",
        "[storage] connecting on the storage runtime"
    );
    runtime
        .spawn(async move { connect().await })
        .await
        .map_err(|error| {
            StorageError::backend(format!("connecting on the storage runtime failed: {error}"))
        })
}

/// Makes `backend` the process's storage backend; returns the previous one.
pub fn install(backend: Arc<dyn StorageBackend>) -> Option<Arc<dyn StorageBackend>> {
    set_installed_url(None);
    let previous = BACKEND.install(backend);
    // What was recorded described the previous backend.
    agents::reset_recorded();
    // Agents derived before the backend existed still need recording.
    agents::record_live();
    previous
}

/// [`install`] for a backend [`open`]ed from `url`, remembering the URL so
/// [`rebind`] can tell when the workspace's default has moved.
pub fn install_for_url(
    url: &str,
    backend: Arc<dyn StorageBackend>,
) -> Option<Arc<dyn StorageBackend>> {
    let previous = install(backend);
    set_installed_url(Some(url.to_string()));
    previous
}

/// The installed backend, when the host configured one.
pub fn installed() -> Option<Arc<dyn StorageBackend>> {
    BACKEND.installed()
}

/// Removes the installed backend; returns whether there was one.
pub fn clear() -> bool {
    set_installed_url(None);
    agents::reset_recorded();
    BACKEND.clear()
}

/// The storage scope of the current call: the tenant's profile
/// ([`scope_for_profile`]) when the work runs for one, else the acting
/// agent's ([`scope_for_agent`], `CoreContext::session_agent`), else
/// [`Scope::local`] — except in SaaS mode, where only a profile scope is
/// handed out: a call with no profile (or no task scope at all) is refused
/// rather than given a bucket other users could share.
///
/// # Errors
///
/// In SaaS mode, when the current task serves no profile.
pub fn current_scope() -> Result<Scope, StorageError> {
    let saas = crate::core::runtime::mode::is_saas();
    match crate::core::runtime::DispatchCtx::ambient().tenant() {
        Ok(tenant) => scope_from(tenant.profile.as_deref(), tenant.agent.as_deref(), saas),
        Err(no_tenant) => Err(StorageError::invalid_input(no_tenant.to_string())),
    }
}

/// [`current_scope`] with its inputs made explicit, so the rule is testable
/// without a booted context or a locked mode.
///
/// # Errors
///
/// When `saas` and there is no `profile`.
pub fn scope_from(
    profile: Option<&str>,
    agent: Option<&str>,
    saas: bool,
) -> Result<Scope, StorageError> {
    match (profile, agent) {
        (Some(profile), _) => Ok(scope_for_profile(profile)),
        (None, _) if saas => Err(StorageError::invalid_input(
            "no profile in SaaS mode; refusing a shared storage scope",
        )),
        (None, Some(agent)) => Ok(scope_for_agent(agent)),
        (None, None) => Ok(Scope::local()),
    }
}

/// The installed backend bound to [`current_scope`], or `None` when no backend
/// is installed.
///
/// # Errors
///
/// When the scope cannot be resolved ([`current_scope`]) or the backend
/// refuses it.
pub fn current_scoped() -> Result<Option<Storage>, StorageError> {
    let Some(backend) = installed() else {
        return Ok(None);
    };
    let scope = current_scope()?;
    backend.for_scope(&scope.to_storage_scope()).map(Some)
}

/// Runs `future` to completion from synchronous code, on one dedicated
/// runtime thread shared by every caller in the process.
///
/// For domain stores whose API is synchronous (most of the core's), so they
/// can call the async storage ports without `block_in_place` — which would
/// panic on a current-thread runtime. The backend [`open`] returns runs on the
/// storage runtime, so `future` depends only on that thread and the storage
/// runtime, never on the caller's: however many callers are parked here, the
/// host runtime cannot be left without the threads the backend needs.
///
/// # Errors
///
/// When the bridge cannot start, or `future` called back into it.
pub fn block_on<T, F>(future: F) -> Result<T, StorageError>
where
    F: Future<Output = Result<T, StorageError>> + Send + 'static,
    T: Send + 'static,
{
    static BRIDGE: OnceLock<Result<Blocking, String>> = OnceLock::new();
    let bridge = BRIDGE
        .get_or_init(|| Blocking::new().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| StorageError::backend(error.clone()))?;
    bridge.run(future)
}

/// [`block_on`] for a future whose error is an `anyhow::Error` — the stores
/// tinyflows puts on the ports return those, with typed errors (such as
/// `FlowUpdateError`) a caller may downcast, so they pass through unchanged.
///
/// # Errors
///
/// The future's own error, or the bridge's when it cannot start.
pub fn block_on_anyhow<T, F>(future: F) -> anyhow::Result<T>
where
    F: Future<Output = anyhow::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    block_on(async move { Ok(future.await) })?
}

/// Whether a backend with this driver name may be shared by several
/// processes at once. A MongoDB database can be; SQLite files, the memory
/// driver and plain files belong to the one process that opened them.
///
/// Boot-time recovery (interrupting in-flight turns, reaping orphaned runs)
/// is only sound on a backend no other process can be writing to.
pub fn driver_is_shared(driver: &str) -> bool {
    driver == "mongodb"
}

/// Whether an opened backend makes a compare-and-swap
/// (`Precondition::Version` / `Absent`) atomic across processes, which a
/// [`lease::DocumentLeases`] needs to exclude other nodes. The driver says so
/// itself with [`tinystoragedrivers::Capability::SharedAcrossProcesses`], and
/// tinystoragedrivers' `conformance::run_shared` proves the claim per driver.
pub fn backend_has_cross_process_cas(backend: &dyn StorageBackend) -> bool {
    backend
        .for_scope(&Scope::cluster())
        .ok()
        .is_some_and(|storage| {
            storage
                .documents()
                .capabilities()
                .contains(tinystoragedrivers::Capability::CrossProcessCas)
        })
}

/// The same answer from a driver name alone, for the boot guard, which judges
/// a configured URL before anything is opened (so it has no backend to ask).
/// It must agree with [`backend_has_cross_process_cas`]: MongoDB and SQLite
/// claim `SharedAcrossProcesses`; the memory driver lives in one process and
/// the file driver checks versions in-process only. A test opens a real
/// backend per driver and pins the two together.
pub fn driver_has_cross_process_cas(driver: &str) -> bool {
    matches!(driver, "mongodb" | "sqlite")
}

/// Whether the installed backend may be shared with other processes; `false`
/// when none is installed. See [`driver_is_shared`].
pub fn installed_is_shared() -> bool {
    installed_url()
        .as_deref()
        .and_then(|url| StorageUrl::parse(url).ok())
        .is_some_and(|config| driver_is_shared(config.driver()))
}

/// A stable key for a legacy import source (a workspace directory), for the
/// marker document and once-per-process key of an importer. Two workspaces
/// that share one explicit backend and scope must each import their own legacy
/// files, so a marker names its source.
pub fn source_key(source: &std::path::Path) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(source.to_string_lossy().as_bytes());
    hex::encode(&digest[..8])
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "host_migration_tests.rs"]
mod host_migration_tests;
