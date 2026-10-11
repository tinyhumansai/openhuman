//! Policy-backend failures at canonical scheduler and manual-run boundaries.

use super::*;
use async_trait::async_trait;
use openhuman_core::cron::policy::{self, JobPolicy};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use tinystoragedrivers_core::{
    Capabilities, CollectionSpec, DocumentStore, Filter, Page, Precondition, Query, ScopedStorage,
    Sort, StorageBackend, StorageError, Version, Versioned,
};
use tinytools::Tool;

type StoreResult<T> = Result<T, StorageError>;
const POLICIES: &str = "cron_job_policies";
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
        if collection == POLICIES {
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
        if collection == POLICIES && self.faults.write.load(Ordering::SeqCst) {
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
        if collection == POLICIES && self.faults.delete.load(Ordering::SeqCst) {
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
}

/// Size and mtime of the SQLite job store, if it exists: unchanged means
/// nothing fell back to it.
fn sqlite_stamp(config: &Config) -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(config.workspace_dir.join("cron/jobs.db")).ok()?;
    Some((meta.len(), meta.modified().ok()?))
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
        faults.write.store(false, Ordering::SeqCst);
        faults.delete.store(true, Ordering::SeqCst);
        assert!(policy::clear_policy(&config, &job.id).is_err());
        faults.delete.store(false, Ordering::SeqCst);
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
    let before = CoreContext::scope(Arc::clone(&alpha), async {
        cron::get_job(&config, &job.id).unwrap().next_run
    })
    .await;
    faults.read.store(true, Ordering::SeqCst);
    cron::scheduler::run_live_agent_pass(1).await;
    let after = CoreContext::scope(Arc::clone(&alpha), async {
        cron::get_job(&config, &job.id).unwrap().next_run
    })
    .await;
    let scheduled_calls = calls.load(Ordering::SeqCst);
    let mut agent_job = job.clone();
    agent_job.job_type = cron::JobType::Agent;
    agent_job.prompt = Some("complete the fixture".into());
    let (direct, agent, manual, queued, tool) = CoreContext::scope(Arc::clone(&alpha), async {
        assert!(policy::get_policy(&config, &job.id).is_err());
        let direct = cron::scheduler::execute_job_now(&config, &job, "policy-direct").await;
        let agent = cron::scheduler::execute_job_now(&config, &agent_job, "policy-agent").await;
        let manual = cron::ops::run_job_now(&config, &job.id).await;
        let queued = cron::ops::cron_run(&config, &job.id).await;
        let tool = cron::tools::CronRunTool::new(Arc::new(config.clone()))
            .execute(json!({"job_id": job.id}))
            .await
            .unwrap();
        (direct, agent, manual, queued, tool)
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
        !direct.0,
        "direct execution must refuse an unreadable policy"
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
    assert!(!format!("{direct:?}{agent:?}{manual:?}{queued:?}{tool:?}").contains("secret"));
    assert_eq!(
        sqlite_stamp(&config),
        sqlite_before,
        "configured storage fell back to SQLite"
    );
    let reads = faults.calls.lock().unwrap();
    assert!(reads
        .iter()
        .any(|(scope, id)| scope == "profile:policy-alpha" && id == &job.id));
    assert!(reads
        .iter()
        .any(|(scope, id)| scope == "profile:policy-beta" && id == &job.id));
    drop(reads);
    faults.read.store(false, Ordering::SeqCst);
    CoreContext::scope(Arc::clone(&alpha), async {
        policy::clear_policy(&config, &job.id).unwrap();
        assert_eq!(
            policy::get_policy(&config, &job.id).unwrap(),
            JobPolicy::default()
        );
        assert!(cron::ops::run_job_now(&config, &job.id).await.unwrap().0);
    })
    .await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "missing policy row must still run with defaults"
    );
    // Losing the backend after a failed attempt must stop further attempts.
    let retry_calls = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&retry_calls);
    let failed_backend = Arc::clone(&faults);
    let _retry_handler = cron::system_job_handlers::register(
        "policy-retry-fixture",
        Arc::new(move |_| {
            recorded.fetch_add(1, Ordering::SeqCst);
            failed_backend.read.store(true, Ordering::SeqCst);
            Box::pin(async { Err("retryable fixture failure".to_string()) })
        }),
    );
    CoreContext::scope(Arc::clone(&alpha), async {
        let job = cron::system_jobs::ensure_system_job(
            &config,
            "policy-retry-fixture",
            Schedule::Every {
                every_ms: 3_600_000,
            },
        )
        .unwrap();
        policy::set_policy(
            &config,
            &job.id,
            JobPolicy {
                retries: Some(2),
                single_flight: false,
            },
        )
        .unwrap();
        let (success, output) =
            cron::scheduler::execute_job_now(&config, &job, "retry-read-failure").await;
        assert!(!success);
        assert_eq!(
            retry_calls.load(Ordering::SeqCst),
            1,
            "unreadable policy admitted a retry"
        );
        assert!(
            output.contains("policy"),
            "the refusal must identify policy availability"
        );
        assert!(!output.contains("secret"));
    })
    .await;
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
        docs.documents()
            .put(POLICIES, &job.id, doc, Precondition::None)
            .await
            .unwrap();
        assert!(
            policy::get_policy(&config, &job.id).is_err(),
            "a malformed existing row became a usable policy"
        );
        let (success, output) =
            cron::scheduler::execute_job_now(&config, &job, "malformed-policy-run").await;
        assert!(!success);
        assert!(!output.contains("private-payload"));
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
        docs.documents()
            .put(POLICIES, &job.id, doc, Precondition::None)
            .await
            .unwrap();
        assert_eq!(policy::get_policy(&config, &job.id).unwrap(), wanted);
    }
    policy::clear_policy(&config, &job.id).unwrap();
    assert_eq!(
        policy::get_policy(&config, &job.id).unwrap(),
        JobPolicy::default()
    );
}
