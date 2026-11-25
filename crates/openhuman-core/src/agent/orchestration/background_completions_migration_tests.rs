use super::*;
use tinyagents_tasks::InMemoryCompletionStore;

/// A store whose writes always fail, like a full disk.
struct FullDisk(InMemoryCompletionStore);

impl CompletionStore for FullDisk {
    fn is_process_local(&self) -> bool {
        self.0.is_process_local()
    }
    fn coordination_identity(&self) -> Option<String> {
        self.0.coordination_identity()
    }
    fn parent_lifecycle(&self) -> Option<Arc<tinyagents_tasks::ParentLifecycle>> {
        self.0.parent_lifecycle()
    }
    fn bind_parent_lifecycle(
        &self,
        lifecycle: Arc<tinyagents_tasks::ParentLifecycle>,
    ) -> tinyagents_harness::error::Result<()> {
        self.0.bind_parent_lifecycle(lifecycle)
    }
    fn acquire_parent_gate(
        &self,
        parent_key: &str,
        holder: &str,
        ttl: std::time::Duration,
    ) -> tinyagents_harness::error::Result<Option<tinyagents_tasks::ParentGateClaim>> {
        self.0.acquire_parent_gate(parent_key, holder, ttl)
    }
    fn renew_parent_gate(
        &self,
        parent_key: &str,
        claim: &tinyagents_tasks::ParentGateClaim,
        ttl: std::time::Duration,
    ) -> tinyagents_harness::error::Result<bool> {
        self.0.renew_parent_gate(parent_key, claim, ttl)
    }
    fn release_parent_gate(
        &self,
        parent_key: &str,
        claim: &tinyagents_tasks::ParentGateClaim,
    ) -> tinyagents_harness::error::Result<bool> {
        self.0.release_parent_gate(parent_key, claim)
    }
    fn get(&self, task_id: &str) -> Option<CompletionRecord> {
        self.0.get(task_id)
    }
    fn try_get(
        &self,
        task_id: &str,
    ) -> tinyagents_harness::error::Result<Option<CompletionRecord>> {
        self.0.try_get(task_id)
    }
    fn put(&self, _record: &CompletionRecord) -> tinyagents_harness::error::Result<()> {
        Err(tinyagents_harness::error::TinyAgentsError::Graph(
            "disk full".into(),
        ))
    }
    fn list(&self, parent_key: Option<&str>) -> Vec<CompletionRecord> {
        self.0.list(parent_key)
    }
    fn put_fenced(
        &self,
        _record: &CompletionRecord,
        _parent_key: &str,
        _gate: &tinyagents_tasks::ParentGateClaim,
    ) -> tinyagents_harness::error::Result<()> {
        Err(tinyagents_harness::error::TinyAgentsError::Graph(
            "disk full".into(),
        ))
    }
    fn insert_if_absent(
        &self,
        _record: &CompletionRecord,
        _gate: &tinyagents_tasks::ParentGateClaim,
    ) -> tinyagents_harness::error::Result<bool> {
        Err(tinyagents_harness::error::TinyAgentsError::Graph(
            "disk full".into(),
        ))
    }
    fn claim_pending(
        &self,
        request: tinyagents_tasks::PendingClaimRequest,
    ) -> tinyagents_harness::error::Result<Vec<CompletionRecord>> {
        self.0.claim_pending(request)
    }
    fn claim_task(
        &self,
        request: tinyagents_tasks::TaskClaimRequest,
    ) -> tinyagents_harness::error::Result<Option<CompletionRecord>> {
        self.0.claim_task(request)
    }
    fn resolve_claim(
        &self,
        request: tinyagents_tasks::ResolveClaimRequest,
    ) -> tinyagents_harness::error::Result<Option<CompletionRecord>> {
        self.0.resolve_claim(request)
    }
    fn try_list(
        &self,
        parent_key: Option<&str>,
    ) -> tinyagents_harness::error::Result<Vec<CompletionRecord>> {
        self.0.try_list(parent_key)
    }
    fn compact(&self, retain: std::time::Duration) -> tinyagents_harness::error::Result<usize> {
        self.0.compact(retain)
    }
}

#[derive(Default)]
struct ToggleStore {
    inner: InMemoryCompletionStore,
    fail_writes: AtomicBool,
    fail_next_write: AtomicBool,
}

impl ToggleStore {
    fn should_fail_write(&self) -> bool {
        self.fail_writes.load(Ordering::SeqCst)
            || self.fail_next_write.swap(false, Ordering::SeqCst)
    }
}

impl CompletionStore for ToggleStore {
    fn is_process_local(&self) -> bool {
        self.inner.is_process_local()
    }
    fn coordination_identity(&self) -> Option<String> {
        self.inner.coordination_identity()
    }
    fn parent_lifecycle(&self) -> Option<Arc<tinyagents_tasks::ParentLifecycle>> {
        self.inner.parent_lifecycle()
    }
    fn bind_parent_lifecycle(
        &self,
        lifecycle: Arc<tinyagents_tasks::ParentLifecycle>,
    ) -> tinyagents_harness::error::Result<()> {
        self.inner.bind_parent_lifecycle(lifecycle)
    }
    fn acquire_parent_gate(
        &self,
        parent_key: &str,
        holder: &str,
        ttl: std::time::Duration,
    ) -> tinyagents_harness::error::Result<Option<tinyagents_tasks::ParentGateClaim>> {
        self.inner.acquire_parent_gate(parent_key, holder, ttl)
    }
    fn renew_parent_gate(
        &self,
        parent_key: &str,
        claim: &tinyagents_tasks::ParentGateClaim,
        ttl: std::time::Duration,
    ) -> tinyagents_harness::error::Result<bool> {
        self.inner.renew_parent_gate(parent_key, claim, ttl)
    }
    fn release_parent_gate(
        &self,
        parent_key: &str,
        claim: &tinyagents_tasks::ParentGateClaim,
    ) -> tinyagents_harness::error::Result<bool> {
        self.inner.release_parent_gate(parent_key, claim)
    }
    fn get(&self, task_id: &str) -> Option<CompletionRecord> {
        self.inner.get(task_id)
    }
    fn try_get(
        &self,
        task_id: &str,
    ) -> tinyagents_harness::error::Result<Option<CompletionRecord>> {
        self.inner.try_get(task_id)
    }

    fn put(&self, record: &CompletionRecord) -> tinyagents_harness::error::Result<()> {
        if self.should_fail_write() {
            return Err(tinyagents_harness::error::TinyAgentsError::Graph(
                "disk full".into(),
            ));
        }
        self.inner.put(record)
    }

    fn put_fenced(
        &self,
        record: &CompletionRecord,
        parent_key: &str,
        gate: &tinyagents_tasks::ParentGateClaim,
    ) -> tinyagents_harness::error::Result<()> {
        if self.should_fail_write() {
            return Err(tinyagents_harness::error::TinyAgentsError::Graph(
                "disk full".into(),
            ));
        }
        self.inner.put_fenced(record, parent_key, gate)
    }

    fn insert_if_absent(
        &self,
        record: &CompletionRecord,
        gate: &tinyagents_tasks::ParentGateClaim,
    ) -> tinyagents_harness::error::Result<bool> {
        if self.should_fail_write() {
            return Err(tinyagents_harness::error::TinyAgentsError::Graph(
                "disk full".into(),
            ));
        }
        self.inner.insert_if_absent(record, gate)
    }

    fn claim_pending(
        &self,
        request: tinyagents_tasks::PendingClaimRequest,
    ) -> tinyagents_harness::error::Result<Vec<CompletionRecord>> {
        self.inner.claim_pending(request)
    }

    fn claim_task(
        &self,
        request: tinyagents_tasks::TaskClaimRequest,
    ) -> tinyagents_harness::error::Result<Option<CompletionRecord>> {
        self.inner.claim_task(request)
    }

    fn resolve_claim(
        &self,
        request: tinyagents_tasks::ResolveClaimRequest,
    ) -> tinyagents_harness::error::Result<Option<CompletionRecord>> {
        self.inner.resolve_claim(request)
    }

    fn list(&self, parent_key: Option<&str>) -> Vec<CompletionRecord> {
        self.inner.list(parent_key)
    }

    fn try_list(
        &self,
        parent_key: Option<&str>,
    ) -> tinyagents_harness::error::Result<Vec<CompletionRecord>> {
        self.inner.try_list(parent_key)
    }

    fn compact(&self, retain: std::time::Duration) -> tinyagents_harness::error::Result<usize> {
        self.inner.compact(retain)
    }
}

#[tokio::test]
async fn a_store_that_keeps_failing_degrades_to_memory_instead_of_losing_the_result() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    install_store_for_test(w, Arc::new(FullDisk(InMemoryCompletionStore::new())));

    record_completion(
        &cx(),
        w,
        "sess-fd",
        "sub-1",
        "researcher",
        "must not be lost",
        Some("thread-fd".into()),
    )
    .await;

    let pending = pending_for(w, "thread-fd");
    assert_eq!(pending.len(), 1, "the result is held in memory");
    assert_eq!(pending[0].result.text, "must not be lost");
}

#[tokio::test]
async fn reopening_a_durable_store_preserves_pending_memory_fallback_records() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    install_store_for_test(w, Arc::new(FullDisk(InMemoryCompletionStore::new())));

    record_completion(
        &cx(),
        w,
        "sess-reopen",
        "memory-task",
        "researcher",
        "held in memory",
        Some("thread-reopen".into()),
    )
    .await;

    let before = pending_for(w, "thread-reopen");
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].result.text, "held in memory");
    drop(before);

    let ctx = crate::core::runtime::DispatchCtx::ambient();
    forget_recovery_ctx(&ctx, w);

    let after = pending_for(w, "thread-reopen");
    assert_eq!(
        after.len(),
        1,
        "re-opening must migrate memory-held records"
    );
    assert_eq!(after[0].result.text, "held in memory");
}

#[tokio::test]
async fn concurrent_degradation_installs_one_fallback() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    let failing = router_for_workspace(w);

    // Two records saw the same failing router; both ask to degrade.
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let first = degrade_to_memory(&ctx, w, &failing);
    let second = degrade_to_memory(&ctx, w, &failing);

    assert!(
        Arc::ptr_eq(&first.router, &second.router),
        "the second caller reuses the fallback the first installed"
    );
    assert!(Arc::ptr_eq(&entry_for(w).router, &first.router));
}

#[tokio::test]
async fn degrading_a_leased_completion_keeps_its_lease() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let store = Arc::new(ToggleStore::default());
    install_store_for_test(w, store.clone());
    let router = router_for_workspace(w);
    router
        .record(CompletionRecord::new(
            "leased-task",
            "leased-thread",
            "researcher",
            CompletionStatus::Success,
            CompletionResult::text("done"),
        ))
        .await
        .expect("record completion");
    assert_eq!(router.claim_pending("leased-thread", 1).unwrap().len(), 1);
    store.fail_writes.store(true, Ordering::SeqCst);

    record_completion(
        &ctx,
        w,
        "leased-session",
        "trigger-task",
        "researcher",
        "force fallback",
        Some("leased-thread".into()),
    )
    .await;

    let claims = router_for_workspace(w)
        .claim_pending("leased-thread", 10)
        .unwrap();
    assert!(
        claims.iter().all(|record| record.task_id != "leased-task"),
        "a fallback router must not make another delivery claim while the first lease is active"
    );
}

#[tokio::test]
async fn transient_parent_admission_failure_does_not_permanently_delete_thread() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let store = Arc::new(ToggleStore::default());
    install_store_for_test(w, store.clone());
    store.fail_writes.store(true, Ordering::SeqCst);

    assert!(mark_stopped_task_if_thread_stopped(
        &ctx,
        w,
        "retryable-parent",
        "rejected-child",
    ));

    store.fail_writes.store(false, Ordering::SeqCst);
    assert!(!mark_stopped_task_if_thread_stopped(
        &ctx,
        w,
        "retryable-parent",
        "accepted-child",
    ));
    record_admitted(
        w,
        "retryable-session",
        "accepted-child",
        "recovered",
        "retryable-parent",
    );
    assert_eq!(pending_ids(w, "retryable-parent"), ["accepted-child"]);
}

#[tokio::test]
async fn failed_stop_resume_does_not_cache_permanent_deletion() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let store = Arc::new(ToggleStore::default());
    install_store_for_test(w, store.clone());
    router_for_workspace(w)
        .cancel_parent("retry-after-resume-error")
        .expect("the stop marker is durable before simulating the resume error");
    store.fail_next_write.store(true, Ordering::SeqCst);

    assert!(mark_stopped_task_if_thread_stopped(
        &ctx,
        w,
        "retry-after-resume-error",
        "rejected-child",
    ));
    assert!(!mark_stopped_task_if_thread_stopped(
        &ctx,
        w,
        "retry-after-resume-error",
        "accepted-child",
    ));
    record_admitted(
        w,
        "retry-after-resume-session",
        "accepted-child",
        "recovered after resume retry",
        "retry-after-resume-error",
    );
    assert_eq!(
        pending_ids(w, "retry-after-resume-error"),
        ["accepted-child"]
    );
}
