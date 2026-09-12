#[async_trait]
impl MemoryDocuments for ModuleMemoryProvider {
    async fn put_document(&self, input: NamespaceDocumentInput) -> Result<String, MemoryError> {
        module_call!(self, "put_document", methods::PUT_DOCUMENT, (input,))
    }
    async fn get_document(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<StoredMemoryDocument>, MemoryError> {
        module_call!(
            self,
            "get_document",
            methods::GET_DOCUMENT,
            (namespace, key)
        )
    }
    async fn list_documents(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, MemoryError> {
        module_call!(
            self,
            "list_documents",
            methods::LIST_DOCUMENTS,
            (namespace.map(str::to_string),)
        )
    }
    async fn list_namespaces(&self) -> Result<Vec<String>, MemoryError> {
        module_call!(self, "list_namespaces", methods::LIST_NAMESPACES, ())
    }
    async fn delete_document(
        &self,
        namespace: &str,
        document_id: &str,
    ) -> Result<serde_json::Value, MemoryError> {
        module_call!(
            self,
            "delete_document",
            methods::DELETE_DOCUMENT,
            (namespace, document_id)
        )
    }
    async fn clear_namespace(&self, namespace: &str) -> Result<(), MemoryError> {
        module_call!(
            self,
            "clear_namespace",
            methods::CLEAR_NAMESPACE,
            (namespace,)
        )
    }
    async fn query_documents(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
    ) -> Result<NamespaceRetrievalContext, MemoryError> {
        module_call!(
            self,
            "query_documents",
            methods::QUERY_DOCUMENTS,
            (namespace, query, limit)
        )
    }
    async fn recall_documents(
        &self,
        namespace: &str,
        limit: usize,
    ) -> Result<NamespaceRetrievalContext, MemoryError> {
        module_call!(
            self,
            "recall_documents",
            methods::RECALL_DOCUMENTS,
            (namespace, limit)
        )
    }
}

#[async_trait]
impl MemoryTree for ModuleMemoryProvider {
    async fn append(&self, request: IngestRequest) -> Result<(), MemoryError> {
        module_call!(self, "append", methods::APPEND, (request,))
    }
    async fn query_source(
        &self,
        namespace: &str,
        source_id: &str,
        limit: usize,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<Chunk>, MemoryError> {
        module_call!(
            self,
            "query_source",
            methods::QUERY_SOURCE,
            (namespace, source_id, limit, scope.cloned())
        )
    }
    async fn drill_down(&self, namespace: &str, node_id: &str) -> Result<QueryResult, MemoryError> {
        module_call!(
            self,
            "drill_down",
            methods::DRILL_DOWN,
            (namespace, node_id)
        )
    }
    async fn seal(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        module_call!(self, "seal", methods::SEAL, (namespace,))
    }
    async fn cascade(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        module_call!(self, "cascade", methods::CASCADE, (namespace,))
    }
    async fn summary_forest(
        &self,
        limit: usize,
        scope: Option<&SourceScope>,
    ) -> Result<SummaryForest, MemoryError> {
        module_call!(
            self,
            "summary_forest",
            methods::SUMMARY_FOREST,
            (limit, scope)
        )
    }

    async fn flush_source_tree(&self, source_scope: &str) -> Result<u64, MemoryError> {
        module_call!(
            self,
            "flush_source_tree",
            methods::FLUSH_SOURCE_TREE,
            (source_scope,)
        )
    }
    async fn recent_leaves(
        &self,
        limit: usize,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<TreeLeaf>, MemoryError> {
        module_call!(
            self,
            "recent_leaves",
            methods::RECENT_LEAVES,
            (limit, scope)
        )
    }
    /// The one member here that costs a provider call rather than a store read,
    /// so it is also the one whose bus deadline could bind. It rides the
    /// default: the module clamps the fold to the `token_budget` this caller
    /// supplied, and a summariser that outruns the deadline is the same failure
    /// a caller must already handle — the contract puts the deterministic
    /// fallback on the *caller* and states that the driver never substitutes
    /// one, precisely so a fallback cannot be mistaken for a model's own work
    /// once it is in the tree. An `Ok` here is therefore always the model's
    /// text, or empty when there was nothing to fold; a model that errors,
    /// times out or refuses arrives as `Err`, never as a filled-in summary.
    async fn summarise(
        &self,
        inputs: &[SummaryInput],
        context: &SummaryContext,
    ) -> Result<SummaryOutput, MemoryError> {
        module_call!(self, "summarise", methods::SUMMARISE, (inputs, context))
    }
    /// The wire member is `RootSummaries`; the caps are in the signature on
    /// both sides, so the name carries only what distinguishes the call.
    async fn root_summaries_with_caps(
        &self,
        per_namespace_cap: usize,
        total_cap: usize,
    ) -> Result<Vec<RootSummary>, MemoryError> {
        module_call!(
            self,
            "root_summaries_with_caps",
            methods::ROOT_SUMMARIES,
            (per_namespace_cap, total_cap)
        )
    }

    // ── The runtime-tree and flavour doors ──────────────────────────────────
    //
    // The seven below are named through `tinymemory_bus::names::methods`
    // rather than as string literals, unlike their neighbours above. The
    // failure a literal invites is precisely the one this family is prone to:
    // a member the pinned artifact does not serve answers `Unsupported` at run
    // time, so a mistyped wire name is indistinguishable from a stale pin, and
    // both look like "the module is old". The constants make the typo a
    // compile error and leave `Unsupported` meaning only what it should.
    //
    // Every one of them is **defaulted** on the trait, which is what makes
    // forwarding them mandatory rather than optional: an override that is
    // missing here does not fail to compile, it silently inherits
    // `Err(Unsupported)` and the driver underneath is never asked.

    async fn runtime_buffer_write(
        &self,
        namespace: &str,
        content: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
        metadata: Option<serde_json::Value>,
    ) -> Result<String, MemoryError> {
        module_call!(
            self,
            "runtime_buffer_write",
            methods::RUNTIME_BUFFER_WRITE,
            (namespace, content, timestamp, metadata)
        )
    }

    async fn runtime_read_node(
        &self,
        namespace: &str,
        node_id: &str,
    ) -> Result<Option<TreeNode>, MemoryError> {
        module_call!(
            self,
            "runtime_read_node",
            methods::RUNTIME_READ_NODE,
            (namespace, node_id)
        )
    }

    async fn runtime_read_children(
        &self,
        namespace: &str,
        parent_id: &str,
    ) -> Result<Vec<TreeNode>, MemoryError> {
        module_call!(
            self,
            "runtime_read_children",
            methods::RUNTIME_READ_CHILDREN,
            (namespace, parent_id)
        )
    }

    async fn runtime_tree_status(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        module_call!(
            self,
            "runtime_tree_status",
            methods::RUNTIME_TREE_STATUS,
            (namespace,)
        )
    }

    /// Long-running on [`Self::summarise`]'s terms — the fold is one provider
    /// call per hour group drained, plus the propagation above them — and it
    /// rides the default deadline for the same reason: the module clamps each
    /// fold to the level's own token budget, and a summariser that outruns the
    /// deadline is the failure every caller of this surface already handles.
    async fn runtime_summarize(
        &self,
        namespace: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<TreeNode>, MemoryError> {
        module_call!(
            self,
            "runtime_summarize",
            methods::RUNTIME_SUMMARIZE,
            (namespace, timestamp)
        )
    }

    /// As [`Self::runtime_summarize`], over every level of the tree at once.
    async fn runtime_rebuild(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        module_call!(
            self,
            "runtime_rebuild",
            methods::RUNTIME_REBUILD,
            (namespace,)
        )
    }

    async fn flavour_profile(&self, scope: &str) -> Result<Option<String>, MemoryError> {
        module_call!(self, "flavour_profile", methods::FLAVOUR_PROFILE, (scope,))
    }
}

#[async_trait]
impl MemoryEntities for ModuleMemoryProvider {
    async fn entities(
        &self,
        namespace: &str,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EntityHit>, MemoryError> {
        module_call!(
            self,
            "entities",
            methods::ENTITIES,
            (namespace, query.map(str::to_string), limit)
        )
    }
    async fn entity_edges(
        &self,
        namespace: &str,
        entity_id: &str,
        limit: usize,
    ) -> Result<Vec<GraphRelationRecord>, MemoryError> {
        module_call!(
            self,
            "entity_edges",
            methods::ENTITY_EDGES,
            (namespace, entity_id, limit)
        )
    }
    async fn touch_entities(
        &self,
        namespace: &str,
        entity_ids: &[String],
    ) -> Result<(), MemoryError> {
        module_call!(
            self,
            "touch_entities",
            methods::TOUCH_ENTITIES,
            (namespace, entity_ids.to_vec())
        )
    }
    async fn top_entities(
        &self,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EntityOccurrence>, MemoryError> {
        module_call!(self, "top_entities", methods::TOP_ENTITIES, (kind, limit))
    }
    async fn chunk_entities(
        &self,
        chunk_ids: &[String],
        kinds: Option<&[String]>,
    ) -> Result<Vec<ChunkEntityOccurrence>, MemoryError> {
        module_call!(
            self,
            "chunk_entities",
            methods::CHUNK_ENTITIES,
            (chunk_ids, kinds)
        )
    }
    async fn entity_chunk_ids(
        &self,
        entity_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, MemoryError> {
        module_call!(
            self,
            "entity_chunk_ids",
            methods::ENTITY_CHUNK_IDS,
            (entity_id, limit)
        )
    }
}

#[async_trait]
impl MemoryGraph for ModuleMemoryProvider {
    async fn kv_get(
        &self,
        namespace: Option<&str>,
        key: &str,
    ) -> Result<Option<MemoryKvRecord>, MemoryError> {
        module_call!(
            self,
            "kv_get",
            methods::KV_GET,
            (namespace.map(str::to_string), key)
        )
    }
    async fn kv_put(
        &self,
        namespace: Option<&str>,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), MemoryError> {
        module_call!(
            self,
            "kv_put",
            methods::KV_PUT,
            (namespace.map(str::to_string), key, value)
        )
    }
    async fn kv_delete(&self, namespace: Option<&str>, key: &str) -> Result<bool, MemoryError> {
        module_call!(
            self,
            "kv_delete",
            methods::KV_DELETE,
            (namespace.map(str::to_string), key)
        )
    }
    async fn kv_list(
        &self,
        namespace: Option<&str>,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryKvRecord>, MemoryError> {
        module_call!(
            self,
            "kv_list",
            methods::KV_LIST,
            (
                namespace.map(str::to_string),
                prefix.map(str::to_string),
                limit
            )
        )
    }
    async fn relations(
        &self,
        namespace: Option<&str>,
        subject: Option<&str>,
        predicate: Option<&str>,
        limit: usize,
    ) -> Result<Vec<GraphRelationRecord>, MemoryError> {
        module_call!(
            self,
            "relations",
            methods::RELATIONS,
            (
                namespace.map(str::to_string),
                subject.map(str::to_string),
                predicate.map(str::to_string),
                limit
            )
        )
    }
    async fn put_relation(&self, relation: GraphRelationRecord) -> Result<(), MemoryError> {
        module_call!(self, "put_relation", methods::PUT_RELATION, (relation,))
    }
}

#[async_trait]
impl MemoryDiff for ModuleMemoryProvider {
    async fn capture_snapshot(&self, source_id: &str) -> Result<SnapshotRef, MemoryError> {
        module_call!(
            self,
            "capture_snapshot",
            methods::CAPTURE_SNAPSHOT,
            (source_id,)
        )
    }
    async fn snapshots(
        &self,
        source_id: &str,
        limit: usize,
    ) -> Result<Vec<SnapshotRef>, MemoryError> {
        module_call!(self, "snapshots", methods::SNAPSHOTS, (source_id, limit))
    }
    async fn diff(
        &self,
        source_id: &str,
        from: Option<&str>,
        to: &str,
    ) -> Result<DiffReport, MemoryError> {
        module_call!(
            self,
            "diff",
            methods::DIFF,
            (source_id, from.map(str::to_string), to)
        )
    }
}

#[async_trait]
impl MemoryGoals for ModuleMemoryProvider {
    async fn goals(&self) -> Result<GoalsDoc, MemoryError> {
        module_call!(self, "goals", methods::GOALS, ())
    }
    async fn set_goals(&self, goals: GoalsDoc) -> Result<(), MemoryError> {
        module_call!(self, "set_goals", methods::SET_GOALS, (goals,))
    }
}

#[async_trait]
impl MemoryToolMemory for ModuleMemoryProvider {
    async fn tool_rules(&self, tool_name: &str) -> Result<Vec<ToolMemoryRule>, MemoryError> {
        module_call!(self, "tool_rules", methods::TOOL_RULES, (tool_name,))
    }
    async fn put_tool_rule(&self, rule: ToolMemoryRule) -> Result<(), MemoryError> {
        module_call!(self, "put_tool_rule", methods::PUT_TOOL_RULE, (rule,))
    }
    async fn delete_tool_rule(&self, tool_name: &str, rule_id: &str) -> Result<bool, MemoryError> {
        module_call!(
            self,
            "delete_tool_rule",
            methods::DELETE_TOOL_RULE,
            (tool_name, rule_id)
        )
    }
}

#[async_trait]
impl MemorySourceSink for ModuleMemoryProvider {
    async fn accept_source_items(
        &self,
        source_id: &str,
        source_kind: &str,
        items: Vec<SourceItem>,
        taint: MemoryTaint,
    ) -> Result<IngestOutcome, MemoryError> {
        module_call_slow!(
            self,
            "accept_source_items",
            methods::ACCEPT_SOURCE_ITEMS,
            (source_id, source_kind, items, taint)
        )
    }
    async fn forget_source(&self, source_id: &str) -> Result<u64, MemoryError> {
        module_call!(self, "forget_source", methods::FORGET_SOURCE, (source_id,))
    }
    async fn forget_matching(
        &self,
        selector: &ForgetSelector,
    ) -> Result<ForgetOutcome, MemoryError> {
        module_call!(
            self,
            "forget_matching",
            methods::FORGET_MATCHING,
            (selector,)
        )
    }
}

#[async_trait]
impl MemoryMaintenance for ModuleMemoryProvider {
    async fn reembed(&self) -> Result<MaintenanceReport, MemoryError> {
        module_call!(self, "reembed", methods::REEMBED, ())
    }
    async fn compact(&self) -> Result<MaintenanceReport, MemoryError> {
        module_call!(self, "compact", methods::COMPACT, ())
    }
    async fn consolidate(&self) -> Result<MaintenanceReport, MemoryError> {
        module_call!(self, "consolidate", methods::CONSOLIDATE, ())
    }
    async fn doctor(&self) -> Result<MaintenanceReport, MemoryError> {
        module_call!(self, "doctor", methods::DOCTOR, ())
    }
    async fn retry_failed(&self) -> Result<MaintenanceReport, MemoryError> {
        module_call!(self, "retry_failed", methods::RETRY_FAILED, ())
    }
    async fn store_stats(&self) -> Result<StoreStats, MemoryError> {
        module_call!(self, "store_stats", methods::STORE_STATS, ())
    }
    async fn queue_stats(&self, kind: Option<&str>) -> Result<QueueStats, MemoryError> {
        module_call!(self, "queue_stats", methods::QUEUE_STATS, (kind,))
    }
    async fn latest_queue_failure(&self) -> Result<Option<QueueFailure>, MemoryError> {
        module_call!(
            self,
            "latest_queue_failure",
            methods::LATEST_QUEUE_FAILURE,
            ()
        )
    }
    async fn backfill_in_progress(&self) -> Result<bool, MemoryError> {
        module_call!(
            self,
            "backfill_in_progress",
            methods::BACKFILL_IN_PROGRESS,
            ()
        )
    }
    async fn flush_pending(&self) -> Result<FlushOutcome, MemoryError> {
        module_call!(self, "flush_pending", methods::FLUSH_PENDING, ())
    }
    /// Long-running by nature — a pass reads and re-embeds up to its whole
    /// limit of documents — so this takes the bulk deadline rather than the
    /// default 30s one. `AcceptSourceItems` is here for the same reason: a call
    /// that outruns the deadline while the module goes on working is the
    /// pathology that made the connector sync retry a finished handoff forever.
    async fn backfill_connector_trees(
        &self,
        request: BackfillTreesRequest,
    ) -> Result<BackfillTreesOutcome, MemoryError> {
        module_call_slow!(
            self,
            "backfill_connector_trees",
            methods::BACKFILL_CONNECTOR_TREES,
            (request,)
        )
    }
    async fn reset_derived_index(&self) -> Result<ResetOutcome, MemoryError> {
        module_call!(
            self,
            "reset_derived_index",
            methods::RESET_DERIVED_INDEX,
            ()
        )
    }
    async fn purge_all(&self) -> Result<PurgeOutcome, MemoryError> {
        module_call!(self, "purge_all", methods::PURGE_ALL, ())
    }
    async fn diagnose(&self) -> Result<Diagnosis, MemoryError> {
        module_call!(self, "diagnose", methods::DIAGNOSE, ())
    }
    /// The degradation flags alone — three booleans and at most one cause,
    /// read from the atomics the module's own embed/extract/storage stages set.
    /// Deliberately not answered from [`Self::diagnose`]'s payload: a status
    /// light polls this, and `Diagnose` runs an aggregate scan of the chunk
    /// table.
    async fn degraded_state(&self) -> Result<DegradedCapabilities, MemoryError> {
        module_call!(self, "degraded_state", methods::DEGRADED_STATE, ())
    }
}

impl ModuleMemoryProvider {
    /// Open a bounded manual-override window on the module's scheduler gate.
    ///
    /// Not part of the `MemoryProvider` contract traits: the override is a
    /// host-initiated maintenance action, not a memory capability, and adding
    /// it to `tinymemory-api` would put a consent-bypass lever into every
    /// embedding's vocabulary. The module serves it as a plain member
    /// (`OverrideSchedulerGate`), and only this host client names it.
    pub(crate) async fn override_scheduler_gate(&self, seconds: u64) -> Result<(), MemoryError> {
        self.proxy("override_scheduler_gate")
            .await?
            .call(methods::OVERRIDE_SCHEDULER_GATE, (seconds,))
            .await
            .map_err(|error| {
                // Typed, not text-matched: a module predating the member
                // answers `tinybus::Error::UnknownMethod`, and mapping that
                // variant to `Unsupported` here lets the RPC report a version
                // gap on the type rather than by grepping arbitrary error
                // strings (review finding on #5932).
                if matches!(error, tinybus::Error::UnknownMethod { .. }) {
                    MemoryError::unsupported_raw("scheduler_override")
                } else {
                    from_bus(&error)
                }
            })
    }
}

/// Load config and publish the module host policy, for a CLI process.
///
/// The server does this during boot; every CLI family that crosses the memory
/// module binding (`memory`, `tree-summarizer`, the raw `call` dispatcher) is
/// its own process and must do the same before its first driver call, or that
/// call fails with "the module host policy was never published". One helper so
/// the sequence lives in the openhuman layer — `src/core/` is transport and
/// carries a one-line call, not the business of loading config and installing
/// sinks (review finding on #5932).
pub async fn publish_cli_boot_policy() -> Result<Config, String> {
    let mut config = Config::load_or_init()
        .await
        .map_err(|error| format!("load config for module policy: {error}"))?;
    config.apply_env_overrides();
    crate::openhuman::memory::host::install_memory_event_sink();
    set_modules_policy(Arc::new(config.clone()));
    Ok(config)
}

