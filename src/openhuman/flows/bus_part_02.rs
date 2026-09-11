
impl DedupCommitSubscriber {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            #[cfg(test)]
            test_hooks: None,
        }
    }

    /// Test constructor: attaches [`CommitTestHooks`] so a test can arm a
    /// delay inside the commit critical section and observe how many
    /// `handle_finished` calls were concurrently inside it.
    #[cfg(test)]
    fn with_test_hooks(config: Arc<Config>, hooks: Arc<CommitTestHooks>) -> Self {
        Self {
            config,
            test_hooks: Some(hooks),
        }
    }

    /// No-op unless [`Self::with_test_hooks`] attached hooks — awaited right
    /// after `handle_finished` acquires the per-flow commit lock, while
    /// still holding it. This is what makes it possible to force two
    /// spawned tasks to genuinely interleave on a single-threaded test
    /// executor (there are no other `.await` points inside the
    /// commit/release critical section to give the executor a chance to
    /// poll a contending task) — a test can then prove the lock, not
    /// accidental scheduling luck, is what serializes two overlapping
    /// `FlowRunFinished` events for the same flow. Compiles to an empty
    /// async fn body (zero-cost) in non-test builds.
    async fn maybe_test_delay(&self) {
        #[cfg(test)]
        if let Some(hooks) = &self.test_hooks {
            use std::sync::atomic::Ordering;
            let now = hooks.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            hooks.max_concurrent.fetch_max(now, Ordering::SeqCst);

            let ms = hooks.delay_ms.load(Ordering::SeqCst);
            if ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }

            hooks.concurrent.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// The node ids of every `dedup` node in `flow_id`'s saved graph, or an
    /// empty vec (logged, not propagated) if the flow can't be loaded — a
    /// flow deleted between run-finish and this handler firing, or a
    /// transient store error, both degrade to "nothing to settle" rather than
    /// panicking the event bus.
    ///
    /// **Known limitation (issue #5265, Codex "P2" on the dedup engine PR):**
    /// this reads the flow's CURRENT saved definition at settlement time, not
    /// a snapshot of the graph the finishing run actually executed. Nothing
    /// today persists a per-run graph/node-id snapshot — `prepare_flow_run`
    /// loads `Flow` fresh into the spawned run's own task, and that copy is
    /// discarded once the run starts; the `FlowRun` row has no `graph` field.
    /// If a long-running flow is edited (or deleted) while a run is still in
    /// flight:
    /// - a `dedup` node the run wrote `tentative` keys under, then deleted or
    ///   renamed before `FlowRunFinished` fires, is no longer found here — its
    ///   tentative keys are neither committed nor released, so those items
    ///   silently retry on the flow's next run (safe-direction: at worst a
    ///   duplicate, never a lost item, matching this subsystem's existing
    ///   safe-failure posture — see the module doc's "Best-effort throughout"
    ///   paragraph);
    /// - conversely a `dedup` node id newly added to the saved graph after the
    ///   run started is settled here even though the run never executed it
    ///   (a harmless no-op: it has no `tentative` keys to commit/release, see
    ///   `commit`/`release`'s early returns).
    ///
    /// Closing this properly means persisting a per-run graph/dedup-node-id
    /// snapshot at run-start (`start_flow_run_row` or a sibling write) and
    /// having this method read that snapshot instead of `store::get_flow` —
    /// a schema + call-site change bigger than this PR's scope; reported as a
    /// follow-up rather than attempted here.
    fn dedup_node_ids(&self, flow_id: &str) -> Vec<String> {
        match store::get_flow(&self.config, flow_id) {
            Ok(Some(flow)) => flow
                .graph
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Dedup)
                .map(|n| n.id.clone())
                .collect(),
            Ok(None) => {
                tracing::debug!(target: "flows", %flow_id, "[dedup-commit] flow no longer exists — skipping");
                Vec::new()
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, error = %e, "[dedup-commit] failed to load flow graph — skipping");
                Vec::new()
            }
        }
    }

    async fn handle_finished(&self, flow_id: &str, run_id: &str, status: &str) {
        let node_ids = self.dedup_node_ids(flow_id);
        if node_ids.is_empty() {
            tracing::trace!(target: "flows", %flow_id, %run_id, %status, "[dedup-commit] no dedup nodes in this flow — nothing to settle");
            return;
        }

        let success = matches!(status, "completed" | "completed_with_warnings");
        tracing::debug!(
            target: "flows", %flow_id, %run_id, %status, success,
            dedup_node_count = node_ids.len(),
            "[dedup-commit] settling dedup nodes for finished run"
        );

        // Serialize this flow's settlement against any other overlapping
        // `FlowRunFinished` handling for the SAME flow_id — held across the
        // whole read-modify-write loop below so two overlapping runs can
        // never interleave their load(committed)+union(tentative)+
        // store(committed) and lose one run's keys. See `FLOW_COMMIT_LOCKS`
        // docs for the full race this closes.
        let lock = flow_commit_lock(flow_id);
        let lock_guard = lock.lock().await;
        tracing::trace!(target: "flows", %flow_id, %run_id, "[dedup-commit] acquired per-flow commit lock");
        self.maybe_test_delay().await;

        let namespace = format!("flow:{flow_id}");
        for node_id in node_ids {
            if success {
                self.commit(&namespace, &node_id, flow_id, run_id);
            } else {
                self.release(&namespace, &node_id, flow_id, run_id);
            }
        }

        drop(lock_guard);
        tracing::trace!(target: "flows", %flow_id, %run_id, "[dedup-commit] released per-flow commit lock");
    }

    /// Success path: union this node's `tentative` set into `committed`, then
    /// clear `tentative`.
    fn commit(&self, namespace: &str, node_id: &str, flow_id: &str, run_id: &str) {
        let tentative_key = dedup_node::tentative_key(node_id);
        let committed_key = dedup_node::committed_key(node_id);

        let tentative = load_key_set(&self.config, namespace, &tentative_key);
        if tentative.is_empty() {
            tracing::trace!(target: "flows", %flow_id, %run_id, node_id, "[dedup-commit] no tentative keys — nothing to commit");
            return;
        }

        let mut committed = load_key_set(&self.config, namespace, &committed_key);
        let added = tentative
            .iter()
            .filter(|k| committed.insert((*k).clone()))
            .count();

        if let Err(e) = store_key_set(&self.config, namespace, &committed_key, &committed) {
            tracing::warn!(
                target: "flows", %flow_id, %run_id, node_id, error = %e,
                "[dedup-commit] failed to write committed set — tentative left in place, will \
                 retry the commit on this node's next successful run"
            );
            return;
        }
        tracing::debug!(
            target: "flows", %flow_id, %run_id, node_id, added, committed_len = committed.len(),
            "[dedup-commit] committed tentative keys"
        );

        if let Err(e) = store::kv_delete(&self.config, namespace, &tentative_key) {
            tracing::warn!(
                target: "flows", %flow_id, %run_id, node_id, error = %e,
                "[dedup-commit] committed but failed to clear tentative — harmless: the next \
                 run's dedup load will re-union the same, now-already-committed keys (committed \
                 is a set, so re-adding them is a no-op)"
            );
        }
    }

    /// Failure path: clear `tentative` only, leaving `committed` untouched so
    /// the released keys retry on the flow's next run.
    ///
    /// Deliberately does NOT `load_key_set` first to report a count: that
    /// would be a full `kv_get` + JSON deserialize + `HashSet` build purely
    /// for a log line, and `kv_delete` already silently no-ops on a missing
    /// key, so there is no early-return to save either (Greptile, issue
    /// #5265).
    fn release(&self, namespace: &str, node_id: &str, flow_id: &str, run_id: &str) {
        match store::kv_delete(&self.config, namespace, &dedup_node::tentative_key(node_id)) {
            Ok(()) => tracing::debug!(
                target: "flows", %flow_id, %run_id, node_id,
                "[dedup-commit] released tentative keys (if any) — will retry next run"
            ),
            Err(e) => tracing::warn!(
                target: "flows", %flow_id, %run_id, node_id, error = %e,
                "[dedup-commit] failed to release tentative — those keys remain tentative until \
                 a future successful commit reconciles them (harmless: committed stays untouched \
                 either way, so no item is ever wrongly marked done)"
            ),
        }
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for DedupCommitSubscriber {
    fn name(&self) -> &str {
        "flows::dedup_commit"
    }

    fn domains(&self) -> Option<&[&str]> {
        // Same reasoning as `FlowRunDigestSubscriber::domains` just above:
        // `FlowRunFinished` is tagged `"cron"` by `DomainEvent::domain()`.
        Some(&["cron"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::FlowRunFinished {
            flow_id,
            run_id,
            status,
        } = event
        {
            self.handle_finished(flow_id, run_id, status).await;
        }
    }
}

/// Loads a `dedup` node's key set (stored as a JSON array of strings) from
/// the flow-state KV table. Mirrors
/// `tinyflows::nodes::control_flow::dedup`'s own key-set loader: a missing
/// key, a non-array value, or an array with non-string elements all degrade
/// to an empty set rather than an error — a first run against a fresh store
/// has nothing recorded yet, which is not a fault.
fn load_key_set(config: &Config, namespace: &str, key: &str) -> HashSet<String> {
    match store::kv_get(config, namespace, key) {
        Ok(Some(value)) => value
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        Ok(None) => HashSet::new(),
        Err(e) => {
            tracing::warn!(target: "flows", %namespace, key, error = %e, "[dedup-commit] failed to load key set — treating as empty");
            HashSet::new()
        }
    }
}

/// Persists `set` under `key` as a JSON array of strings, sorted for a
/// stable, diffable on-disk representation (membership is exact-match either
/// way, so sort order carries no semantic meaning).
fn store_key_set(
    config: &Config,
    namespace: &str,
    key: &str,
    set: &HashSet<String>,
) -> anyhow::Result<()> {
    let mut keys: Vec<String> = set.iter().cloned().collect();
    keys.sort_unstable();
    let value = Value::Array(keys.into_iter().map(Value::String).collect());
    store::kv_set(config, namespace, key, &value)
}
