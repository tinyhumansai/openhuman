//! Policy-backend failures at canonical scheduler and manual-run boundaries.

use super::*;
use async_trait::async_trait;
use openhuman_core::cron::{CronJobPatch, JobPolicy};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use tinystoragedrivers_core::{
    Capabilities, CollectionSpec, DocumentStore, Filter, Page, Precondition, Query, ReadCheck,
    ScopedStorage, Sort, StorageBackend, StorageError, Version, Versioned, WriteOp, WriteResult,
};
use tinytools::Tool;

type StoreResult<T> = Result<T, StorageError>;

/// A job's policy is a field of its job document: these read and write it
/// through the job, failing closed when the job cannot be read.
mod policy {
    use super::*;

    pub fn get_policy(config: &Config, job_id: &str) -> anyhow::Result<JobPolicy> {
        Ok(cron::get_job(config, job_id)?.policy)
    }

    pub fn set_policy(config: &Config, job_id: &str, policy: JobPolicy) -> anyhow::Result<()> {
        cron::update_job(
            config,
            job_id,
            CronJobPatch {
                policy: Some(policy),
                ..CronJobPatch::default()
            },
        )
        .map(|_| ())
    }

    pub fn clear_policy(config: &Config, job_id: &str) -> anyhow::Result<()> {
        set_policy(config, job_id, JobPolicy::default())
    }
}
/// A job's policy is a field of its job document.
const JOBS: &str = "cron_jobs";
const BACKEND_DETAIL: &str = "mongodb://policy-user:secret@backend/private-payload";

#[derive(Debug, Default)]
struct Faults {
    read: AtomicBool,
    write: AtomicBool,
    delete: AtomicBool,
    calls: Mutex<Vec<(String, String)>>,
    writes: AtomicUsize,
}

#[derive(Debug)]
struct Backend {
    inner: Arc<dyn StorageBackend>,
    faults: Arc<Faults>,
}

impl StorageBackend for Backend {
    fn driver(&self) -> &'static str {
        self.inner.driver()
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn for_scope(&self, scope: &Scope) -> StoreResult<ScopedStorage> {
        let inner = self.inner.for_scope(scope)?;
        Ok(ScopedStorage::new(
            scope.clone(),
            inner.driver(),
            Arc::new(Documents {
                inner: Arc::clone(inner.documents()),
                scope: scope.to_string(),
                faults: Arc::clone(&self.faults),
            }),
            Arc::clone(inner.streams()),
            Arc::clone(inner.blobs()),
        ))
    }
    fn database(&self, name: &str) -> StoreResult<Arc<dyn StorageBackend>> {
        Ok(Arc::new(Self {
            inner: self.inner.database(name)?,
            faults: Arc::clone(&self.faults),
        }))
    }
}

#[derive(Debug)]
struct Documents {
    inner: Arc<dyn DocumentStore>,
    scope: String,
    faults: Arc<Faults>,
}

#[async_trait]
impl DocumentStore for Documents {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn ensure_collection(&self, spec: &CollectionSpec) -> StoreResult<()> {
        self.inner.ensure_collection(spec).await
    }
    async fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> StoreResult<Option<Versioned<serde_json::Value>>> {
        if collection == JOBS {
            self.faults
                .calls
                .lock()
                .unwrap()
                .push((self.scope.clone(), id.to_string()));
            if self.faults.read.load(Ordering::SeqCst) {
                return Err(StorageError::backend(BACKEND_DETAIL));
            }
        }
        self.inner.get(collection, id).await
    }
    async fn put(
        &self,
        collection: &str,
        id: &str,
        doc: serde_json::Value,
        precondition: Precondition,
    ) -> StoreResult<Version> {
        if self.faults.read.load(Ordering::SeqCst) {
            self.faults.writes.fetch_add(1, Ordering::SeqCst);
        }
        if collection == JOBS && self.faults.write.load(Ordering::SeqCst) {
            return Err(StorageError::backend(BACKEND_DETAIL));
        }
        self.inner.put(collection, id, doc, precondition).await
    }
    async fn delete(
        &self,
        collection: &str,
        id: &str,
        precondition: Precondition,
    ) -> StoreResult<bool> {
        if self.faults.read.load(Ordering::SeqCst) {
            self.faults.writes.fetch_add(1, Ordering::SeqCst);
        }
        if collection == JOBS && self.faults.delete.load(Ordering::SeqCst) {
            return Err(StorageError::backend(BACKEND_DETAIL));
        }
        self.inner.delete(collection, id, precondition).await
    }
    async fn query(
        &self,
        collection: &str,
        query: &Query,
    ) -> StoreResult<Page<Versioned<serde_json::Value>>> {
        self.inner.query(collection, query).await
    }
    async fn count(&self, collection: &str, filter: &Filter) -> StoreResult<u64> {
        self.inner.count(collection, filter).await
    }
    async fn delete_where(&self, collection: &str, filter: &Filter) -> StoreResult<u64> {
        self.inner.delete_where(collection, filter).await
    }
    async fn claim(
        &self,
        collection: &str,
        filter: &Filter,
        sort: &[Sort],
        patch: &serde_json::Value,
    ) -> StoreResult<Option<Versioned<serde_json::Value>>> {
        self.inner.claim(collection, filter, sort, patch).await
    }
    async fn drop_collection(&self, collection: &str) -> StoreResult<()> {
        self.inner.drop_collection(collection).await
    }
    // The cron store applies its multi-document writes atomically, so the
    // wrapper forwards batches (and applies the same faults to them).
    async fn atomic_batch(&self, ops: Vec<WriteOp>) -> StoreResult<Vec<WriteResult>> {
        self.check_batch(&ops)?;
        self.inner.atomic_batch(ops).await
    }
    async fn atomic_batch_checked(
        &self,
        checks: Vec<ReadCheck>,
        ops: Vec<WriteOp>,
    ) -> StoreResult<Vec<WriteResult>> {
        self.check_batch(&ops)?;
        self.inner.atomic_batch_checked(checks, ops).await
    }
}

impl Documents {
    /// Counts a batch's writes while the read fault is armed (nothing may
    /// write then) and fails it as its single writes would fail.
    fn check_batch(&self, ops: &[WriteOp]) -> StoreResult<()> {
        if self.faults.read.load(Ordering::SeqCst) {
            self.faults.writes.fetch_add(ops.len(), Ordering::SeqCst);
        }
        for op in ops {
            let fails = match op {
                WriteOp::Put { collection, .. } => {
                    collection == JOBS && self.faults.write.load(Ordering::SeqCst)
                }
                WriteOp::Delete { collection, .. } => {
                    collection == JOBS && self.faults.delete.load(Ordering::SeqCst)
                }
            };
            if fails {
                return Err(StorageError::backend(BACKEND_DETAIL));
            }
        }
        Ok(())
    }
}

/// Size and mtime of the workspace's own job store, if it exists: unchanged
/// (absent) means nothing fell back to it.
fn sqlite_stamp(config: &Config) -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(config.workspace_dir.join("cron/schedule.db")).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// The next run of `job_id`, read through a listing (a point read is what the
/// fault injection refuses).
fn next_run_of(config: &Config, job_id: &str) -> chrono::DateTime<chrono::Utc> {
    cron::list_jobs(config)
        .unwrap()
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap()
        .next_run
}

/// Writes `policy` as the raw `policy` field of `job_id`'s document, bypassing
/// the store's own validation.
async fn put_raw_policy(docs: &ScopedStorage, job_id: &str, policy: serde_json::Value) {
    let mut stored = docs.documents().get(JOBS, job_id).await.unwrap().unwrap();
    stored.doc["policy"] = policy;
    docs.documents()
        .put(JOBS, job_id, stored.doc, Precondition::None)
        .await
        .unwrap();
}

pub(super) async fn policy_reads_refuse_execution_without_consuming_a_slot(case: Case) {
    let workspace = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: workspace.path().join("workspace"),
        config_path: workspace.path().join("config.toml"),
        ..Config::default()
    };
    tinyhumans_boot::boot();
    let _runtime = CoreBuilder::new(HostKind::Library)
        .config(config.clone())
        .services(ServiceSet::none())
        .domains(DomainSet::none())
        .build()
        .await
        .unwrap();
    let faults = Arc::new(Faults::default());
    storage::install(Arc::new(Backend {
        inner: case.backend(),
        faults: Arc::clone(&faults),
    }));
    let sqlite_before = sqlite_stamp(&config);
    let profile = |name: &str| {
        let ctx = CoreContext::current().unwrap().derive_with(
            ContextOverlay::new(config.clone(), DomainSet::none(), Default::default())
                .session_agent(name)
                .profile(name),
        );
        AgentContextRegistry::register(name, &ctx);
        ctx
    };
    let alpha = profile("policy-alpha");
    let beta = profile("policy-beta");
    let calls = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&calls);
    let _handler = cron::system_job_handlers::register(
        "policy-fixture",
        Arc::new(move |_| {
            recorded.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }),
    );
    let create = || async {
        cron::system_jobs::ensure_system_job(
            &config,
            "policy-fixture",
            Schedule::Every {
                every_ms: 3_600_000,
            },
        )
        .unwrap()
    };
    let job = CoreContext::scope(Arc::clone(&alpha), create()).await;
    let other = CoreContext::scope(Arc::clone(&beta), create()).await;
    assert_eq!(
        job.id, other.id,
        "same job id must remain isolated by profile"
    );
    let restricted = JobPolicy {
        retries: Some(0),
        single_flight: true,
    };
    CoreContext::scope(Arc::clone(&alpha), async {
        policy::set_policy(&config, &job.id, restricted).unwrap();
        assert_eq!(policy::get_policy(&config, &job.id).unwrap(), restricted);
        faults.write.store(true, Ordering::SeqCst);
        assert!(policy::set_policy(
            &config,
            &job.id,
            JobPolicy {
                retries: Some(4),
                single_flight: false
            }
        )
        .is_err());
        assert!(policy::clear_policy(&config, &job.id).is_err());
        faults.write.store(false, Ordering::SeqCst);
        assert_eq!(policy::get_policy(&config, &job.id).unwrap(), restricted);
    })
    .await;
    CoreContext::scope(Arc::clone(&beta), async {
        assert_eq!(
            policy::get_policy(&config, &job.id).unwrap(),
            JobPolicy::default()
        );
    })
    .await;
    // Force the persisted occurrence into the past through its actual document port.
    CoreContext::scope(Arc::clone(&alpha), async {
        let docs = storage::current_scoped().unwrap().unwrap();
        let mut stored = docs
            .documents()
            .get("cron_jobs", &job.id)
            .await
            .unwrap()
            .unwrap();
        let due = chrono::Utc::now() - chrono::Duration::seconds(1);
        stored.doc["next_run"] = json!(due.to_rfc3339());
        stored.doc["next_run_ms"] = json!(due.timestamp_millis());
        stored.doc["next_run_ns"] = json!(due.timestamp_nanos_opt().unwrap());
        docs.documents()
            .put("cron_jobs", &job.id, stored.doc, Precondition::None)
            .await
            .unwrap();
    })
    .await;
    let before =
        CoreContext::scope(Arc::clone(&alpha), async { next_run_of(&config, &job.id) }).await;
    faults.read.store(true, Ordering::SeqCst);
    cron::scheduler::run_live_agent_pass(1).await;
    let after =
        CoreContext::scope(Arc::clone(&alpha), async { next_run_of(&config, &job.id) }).await;
    let scheduled_calls = calls.load(Ordering::SeqCst);
    let (manual, queued, tool) = CoreContext::scope(Arc::clone(&alpha), async {
        assert!(policy::get_policy(&config, &job.id).is_err());
        let manual = cron::ops::run_job_now(
            &openhuman_core::core::runtime::DispatchCtx::ambient(),
            &config,
            &job.id,
        )
        .await;
        let queued = cron::ops::cron_run(
            &openhuman_core::core::runtime::DispatchCtx::ambient(),
            &config,
            &job.id,
        )
        .await;
        let tool = cron::tools::CronRunTool::new(Arc::new(config.clone()))
            .execute(json!({"job_id": job.id}))
            .await
            .unwrap();
        (manual, queued, tool)
    })
    .await;
    assert_eq!(
        faults.writes.load(Ordering::SeqCst),
        0,
        "policy refusal mutated persistent job/run state"
    );
    assert_eq!(
        scheduled_calls, 0,
        "policy read failure admitted a scheduled job"
    );
    assert_eq!(
        after, before,
        "policy read failure consumed the due occurrence"
    );
    assert!(
        manual.is_err(),
        "Run now must refuse before delivery/persistence"
    );
    assert!(
        queued.is_err(),
        "queued Run now must refuse before enqueueing"
    );
    assert!(tool.is_error, "run tool must refuse an unreadable policy");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "manual paths executed the job"
    );
    // Every manual path loads the job, and the policy is a field of that same
    // document, so the injected fault reaches them at the lookup; none may leak
    // backend detail.
    for (name, shown) in [
        ("manual", format!("{manual:?}")),
        ("queued", format!("{queued:?}")),
    ] {
        assert!(
            !shown.contains("secret"),
            "{name} leaked backend detail: {shown}"
        );
    }
    assert_eq!(
        sqlite_stamp(&config),
        sqlite_before,
        "configured storage fell back to SQLite"
    );
    {
        let reads = faults.calls.lock().unwrap();
        assert!(reads
            .iter()
            .any(|(scope, id)| scope == "profile:policy-alpha" && id == &job.id));
        assert!(reads
            .iter()
            .any(|(scope, id)| scope == "profile:policy-beta" && id == &job.id));
    }
    faults.read.store(false, Ordering::SeqCst);
    CoreContext::scope(Arc::clone(&alpha), async {
        policy::clear_policy(&config, &job.id).unwrap();
        assert_eq!(
            policy::get_policy(&config, &job.id).unwrap(),
            JobPolicy::default()
        );
        assert!(
            cron::ops::run_job_now(
                &openhuman_core::core::runtime::DispatchCtx::ambient(),
                &config,
                &job.id
            )
            .await
            .unwrap()
            .0
        );
    })
    .await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "missing policy row must still run with defaults"
    );
    assert!(AgentContextRegistry::deregister("policy-alpha", &alpha));
    assert!(AgentContextRegistry::deregister("policy-beta", &beta));
}

pub(super) async fn malformed_policy_rows_refuse_execution(case: Case) {
    let workspace = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: workspace.path().join("workspace"),
        config_path: workspace.path().join("config.toml"),
        ..Config::default()
    };
    tinyhumans_boot::boot();
    let _runtime = CoreBuilder::new(HostKind::Library)
        .config(config.clone())
        .services(ServiceSet::none())
        .domains(DomainSet::none())
        .build()
        .await
        .unwrap();
    case.install();
    let calls = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&calls);
    let _handler = cron::system_job_handlers::register(
        "malformed-policy-fixture",
        Arc::new(move |_| {
            recorded.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }),
    );
    let job = cron::system_jobs::ensure_system_job(
        &config,
        "malformed-policy-fixture",
        Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .unwrap();
    let docs = storage::current_scoped().unwrap().unwrap();
    for doc in [
        json!({"retries": -1, "single_flight": true}),
        json!({"retries": 4_294_967_296_u64, "single_flight": true}),
        json!({"retries": "private-payload", "single_flight": true}),
        json!({"retries": 0, "single_flight": "private-payload"}),
        json!({"retries": 0}),
    ] {
        put_raw_policy(&docs, &job.id, doc).await;
        assert!(
            policy::get_policy(&config, &job.id).is_err(),
            "a malformed existing row became a usable policy"
        );
        let refused = cron::ops::run_job_now(
            &openhuman_core::core::runtime::DispatchCtx::ambient(),
            &config,
            &job.id,
        )
        .await
        .expect_err("a malformed policy must refuse Run now");
        assert!(!refused.contains("private-payload"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    for (doc, wanted) in [
        (
            json!({"retries": null, "single_flight": false}),
            JobPolicy::default(),
        ),
        (
            json!({"single_flight": true}),
            JobPolicy {
                retries: None,
                single_flight: true,
            },
        ),
        (
            json!({"retries": u32::MAX, "single_flight": true}),
            JobPolicy {
                retries: Some(u32::MAX),
                single_flight: true,
            },
        ),
    ] {
        put_raw_policy(&docs, &job.id, doc).await;
        assert_eq!(policy::get_policy(&config, &job.id).unwrap(), wanted);
    }
    policy::clear_policy(&config, &job.id).unwrap();
    assert_eq!(
        policy::get_policy(&config, &job.id).unwrap(),
        JobPolicy::default()
    );
}
