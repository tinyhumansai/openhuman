// ── Entities ─────────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryEntities for GuardedEntities {
    async fn entities(
        &self,
        namespace: &str,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EntityHit>, MemoryError> {
        self.policy.admit_read(
            Capability::Entities,
            "entities.entities",
            namespace,
            query.is_some(),
        )?;
        let redacted = query.map(|q| self.policy.redact_outbound(q).into_owned());
        self.family()?
            .entities(namespace, redacted.as_deref(), limit)
            .await
    }

    async fn entity_edges(
        &self,
        namespace: &str,
        entity_id: &str,
        limit: usize,
    ) -> Result<Vec<GraphRelationRecord>, MemoryError> {
        self.policy.admit_read(
            Capability::Entities,
            "entities.entity_edges",
            namespace,
            false,
        )?;
        self.family()?
            .entity_edges(namespace, entity_id, limit)
            .await
    }

    async fn touch_entities(
        &self,
        namespace: &str,
        entity_ids: &[String],
    ) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::Entities,
            "entities.touch_entities",
            namespace,
            false,
        )?;
        self.family()?.touch_entities(namespace, entity_ids).await
    }

    /// The occurrence index has no namespace and the contract gives this member
    /// no scope argument, so there is nothing to intersect — the tier check is
    /// the whole gate. Worth stating rather than leaving as an apparent
    /// omission beside the scoped members above.
    async fn top_entities(
        &self,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EntityOccurrence>, MemoryError> {
        self.policy.admit_read(
            Capability::Entities,
            "entities.top_entities",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.top_entities(kind, limit).await
    }

    /// Scoped by the chunk ids the caller already holds: it can only name
    /// chunks a previous, scoped read handed it, so this adds no reach beyond
    /// the read that produced them.
    async fn chunk_entities(
        &self,
        chunk_ids: &[String],
        kinds: Option<&[String]>,
    ) -> Result<Vec<ChunkEntityOccurrence>, MemoryError> {
        self.policy.admit_read(
            Capability::Entities,
            "entities.chunk_entities",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.chunk_entities(chunk_ids, kinds).await
    }

    /// Returns ids only, never content. A caller still has to read those chunks
    /// through [`MemoryChunks`] to see anything, and that path applies the
    /// scope intersection.
    async fn entity_chunk_ids(
        &self,
        entity_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, MemoryError> {
        self.policy.admit_read(
            Capability::Entities,
            "entities.entity_chunk_ids",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.entity_chunk_ids(entity_id, limit).await
    }
}

// ── Graph ────────────────────────────────────────────────────────────────────

/// Namespace label for the graph family's `Option<&str>` namespace — `None`
/// addresses the global, namespace-less slice.
fn graph_ns(namespace: Option<&str>) -> &str {
    namespace.unwrap_or(NO_NAMESPACE)
}

#[async_trait]
impl MemoryGraph for GuardedGraph {
    async fn kv_get(
        &self,
        namespace: Option<&str>,
        key: &str,
    ) -> Result<Option<MemoryKvRecord>, MemoryError> {
        self.policy.admit_read(
            Capability::Graph,
            "graph.kv_get",
            graph_ns(namespace),
            false,
        )?;
        self.family()?.kv_get(namespace, key).await
    }

    async fn kv_put(
        &self,
        namespace: Option<&str>,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), MemoryError> {
        self.policy
            .admit_write(Capability::Graph, "graph.kv_put", graph_ns(namespace), true)?;
        let value = self.policy.redact_outbound_json(value);
        self.family()?.kv_put(namespace, key, value).await
    }

    async fn kv_delete(&self, namespace: Option<&str>, key: &str) -> Result<bool, MemoryError> {
        self.policy.admit_write(
            Capability::Graph,
            "graph.kv_delete",
            graph_ns(namespace),
            false,
        )?;
        self.family()?.kv_delete(namespace, key).await
    }

    async fn kv_list(
        &self,
        namespace: Option<&str>,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryKvRecord>, MemoryError> {
        self.policy.admit_read(
            Capability::Graph,
            "graph.kv_list",
            graph_ns(namespace),
            false,
        )?;
        self.family()?.kv_list(namespace, prefix, limit).await
    }

    async fn relations(
        &self,
        namespace: Option<&str>,
        subject: Option<&str>,
        predicate: Option<&str>,
        limit: usize,
    ) -> Result<Vec<GraphRelationRecord>, MemoryError> {
        self.policy.admit_read(
            Capability::Graph,
            "graph.relations",
            graph_ns(namespace),
            false,
        )?;
        self.family()?
            .relations(namespace, subject, predicate, limit)
            .await
    }

    async fn put_relation(&self, relation: GraphRelationRecord) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::Graph,
            "graph.put_relation",
            graph_ns(relation.namespace.as_deref()),
            true,
        )?;
        self.family()?.put_relation(relation).await
    }
}

// ── Diff ─────────────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryDiff for GuardedDiff {
    async fn capture_snapshot(&self, source_id: &str) -> Result<SnapshotRef, MemoryError> {
        self.policy.admit_write(
            Capability::Diff,
            "diff.capture_snapshot",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.capture_snapshot(source_id).await
    }

    async fn snapshots(
        &self,
        source_id: &str,
        limit: usize,
    ) -> Result<Vec<SnapshotRef>, MemoryError> {
        self.policy
            .admit_read(Capability::Diff, "diff.snapshots", NO_NAMESPACE, false)?;
        self.family()?.snapshots(source_id, limit).await
    }

    async fn diff(
        &self,
        source_id: &str,
        from: Option<&str>,
        to: &str,
    ) -> Result<DiffReport, MemoryError> {
        self.policy
            .admit_read(Capability::Diff, "diff.diff", NO_NAMESPACE, false)?;
        self.family()?.diff(source_id, from, to).await
    }
}

// ── Goals ────────────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryGoals for GuardedGoals {
    async fn goals(&self) -> Result<GoalsDoc, MemoryError> {
        self.policy
            .admit_read(Capability::Goals, "goals.goals", NO_NAMESPACE, false)?;
        self.family()?.goals().await
    }

    async fn set_goals(&self, goals: GoalsDoc) -> Result<(), MemoryError> {
        self.policy
            .admit_write(Capability::Goals, "goals.set_goals", NO_NAMESPACE, true)?;
        // The goals document's own validating mutation surface (the PII and
        // secret predicates) is host policy that already runs in
        // `memory::goals` before a document reaches the contract, so the guard
        // does not re-scrub item text here. If an external driver ever binds,
        // M6 must decide whether that upstream scrub is sufficient for egress
        // or whether item bodies need the same `redact_outbound` treatment the
        // document and ingest paths get.
        self.family()?.set_goals(goals).await
    }
}

// ── Tool memory ──────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryToolMemory for GuardedToolMemory {
    async fn tool_rules(&self, tool_name: &str) -> Result<Vec<ToolMemoryRule>, MemoryError> {
        self.policy.admit_read(
            Capability::ToolMemory,
            "tool_memory.tool_rules",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.tool_rules(tool_name).await
    }

    async fn put_tool_rule(&self, rule: ToolMemoryRule) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::ToolMemory,
            "tool_memory.put_tool_rule",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.put_tool_rule(rule).await
    }

    async fn delete_tool_rule(&self, tool_name: &str, rule_id: &str) -> Result<bool, MemoryError> {
        self.policy.admit_write(
            Capability::ToolMemory,
            "tool_memory.delete_tool_rule",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.delete_tool_rule(tool_name, rule_id).await
    }
}

// ── Sources ──────────────────────────────────────────────────────────────────

#[async_trait]
impl MemorySourceSink for GuardedSources {
    async fn accept_source_items(
        &self,
        source_id: &str,
        source_kind: &str,
        items: Vec<SourceItem>,
        taint: MemoryTaint,
    ) -> Result<IngestOutcome, MemoryError> {
        self.policy.admit_write(
            Capability::Sources,
            "sources.accept_source_items",
            NO_NAMESPACE,
            true,
        )?;
        // Step 3: the batch taint is the guard's to decide. `stamp_taint` never
        // downgrades, so a sync path that already asked for `ExternalSync` keeps
        // it whether or not a source scope is active.
        let taint = self.policy.stamp_taint(taint);
        let items: Vec<SourceItem> = items
            .into_iter()
            .map(|mut item| {
                item.title = self.policy.redact_outbound(&item.title).into_owned();
                item.content = self.policy.redact_outbound(&item.content).into_owned();
                item
            })
            .collect();
        trace_allowed(
            &self.policy,
            "sources.accept_source_items",
            NO_NAMESPACE,
            items.iter().map(|i| i.content.chars().count()).sum(),
        );
        self.family()?
            .accept_source_items(source_id, source_kind, items, taint)
            .await
    }

    async fn forget_source(&self, source_id: &str) -> Result<u64, MemoryError> {
        self.policy.admit_write(
            Capability::Sources,
            "sources.forget_source",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.forget_source(source_id).await
    }

    /// The one door for every scoped forget, so it takes the same write tier as
    /// [`Self::forget_source`]. The selector names what to remove rather than
    /// carrying content, which is why the egress flag is `false` — the same
    /// reading its single-source sibling makes.
    async fn forget_matching(
        &self,
        selector: &ForgetSelector,
    ) -> Result<ForgetOutcome, MemoryError> {
        self.policy.admit_write(
            Capability::Sources,
            "sources.forget_matching",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.forget_matching(selector).await
    }
}

// ── Maintenance ──────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryMaintenance for GuardedMaintenance {
    async fn reembed(&self) -> Result<MaintenanceReport, MemoryError> {
        self.policy.admit_write(
            Capability::Maintenance,
            "maintenance.reembed",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.reembed().await
    }

    async fn compact(&self) -> Result<MaintenanceReport, MemoryError> {
        self.policy.admit_write(
            Capability::Maintenance,
            "maintenance.compact",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.compact().await
    }

    async fn consolidate(&self) -> Result<MaintenanceReport, MemoryError> {
        self.policy.admit_write(
            Capability::Maintenance,
            "maintenance.consolidate",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.consolidate().await
    }

    /// Read-only by contract, so this takes the **read** tier check: a
    /// `readonly` operator must still be able to run `doctor`, which is exactly
    /// the tier where diagnosing without mutating matters most.
    async fn doctor(&self) -> Result<MaintenanceReport, MemoryError> {
        self.policy.admit_read(
            Capability::Maintenance,
            "maintenance.doctor",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.doctor().await
    }

    /// Empties the whole store, so it takes the write tier rather than
    /// `doctor`'s read one — and deliberately carries no scope, because there
    /// is no scoped reading of "purge everything". A source-restricted caller
    /// that reached this would be destroying rows it is not even allowed to
    /// read; the write tier is what stops it.
    async fn purge_all(&self) -> Result<PurgeOutcome, MemoryError> {
        self.policy.admit_write(
            Capability::Maintenance,
            "maintenance.purge_all",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.purge_all().await
    }

    /// [`Self::doctor`]'s findings in full, and read-only on the same terms —
    /// it inspects configuration, persisted state and counters and mutates
    /// nothing, so it takes the read tier for the reason `doctor` gives.
    ///
    /// Forwarded here rather than left to the trait's default: a defaulted
    /// method on a decorator answers `Unsupported` even when the driver below
    /// serves it, so the default would refuse every diagnosis reached through
    /// the guard.
    async fn diagnose(&self) -> Result<Diagnosis, MemoryError> {
        self.policy.admit_read(
            Capability::Maintenance,
            "maintenance.diagnose",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.diagnose().await
    }

    /// The degradation flags without the diagnosis around them — three
    /// booleans and at most one cause. Read tier for the same reason
    /// [`Self::doctor`] takes it, and for one more: this is what a status
    /// light polls, so refusing it under `readonly` would leave the surface
    /// that reports a reduced pipeline unable to say so.
    async fn degraded_state(&self) -> Result<DegradedCapabilities, MemoryError> {
        self.policy.admit_read(
            Capability::Maintenance,
            "maintenance.degraded_state",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.degraded_state().await
    }
}

// ── People ───────────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryPeople for GuardedPeople {
    async fn list_people(&self, limit: Option<usize>) -> Result<Vec<RankedPerson>, MemoryError> {
        self.policy.admit_read(
            Capability::People,
            "people.list_people",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.list_people(limit).await
    }

    async fn get_person(&self, person_id: &str) -> Result<Option<PersonRecord>, MemoryError> {
        self.policy
            .admit_read(Capability::People, "people.get_person", NO_NAMESPACE, false)?;
        self.family()?.get_person(person_id).await
    }

    /// A read *unless* it may mint a person, which is a write.
    ///
    /// The tier check follows what the call can actually do rather than what it
    /// is named: with `create_if_missing` set this inserts a row, so a
    /// `readonly` operator must be refused. Classifying the whole method as a
    /// read would have handed `readonly` a working insert through the back
    /// door.
    async fn resolve_handle(
        &self,
        handle: &PersonHandle,
        create_if_missing: bool,
    ) -> Result<Option<ResolvedPerson>, MemoryError> {
        if create_if_missing {
            self.policy.admit_write(
                Capability::People,
                "people.resolve_handle",
                NO_NAMESPACE,
                true,
            )?;
        } else {
            self.policy.admit_read(
                Capability::People,
                "people.resolve_handle",
                NO_NAMESPACE,
                false,
            )?;
        }
        self.family()?
            .resolve_handle(handle, create_if_missing)
            .await
    }

    async fn add_handle_alias(
        &self,
        person_id: &str,
        handle: &PersonHandle,
    ) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::People,
            "people.add_handle_alias",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.add_handle_alias(person_id, handle).await
    }

    async fn score_person(&self, person_id: &str) -> Result<Option<PersonScore>, MemoryError> {
        self.policy.admit_read(
            Capability::People,
            "people.score_person",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.score_person(person_id).await
    }

    async fn record_interaction(&self, interaction: &PersonInteraction) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::People,
            "people.record_interaction",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.record_interaction(interaction).await
    }

    /// A write: it reads the platform address book and inserts what it finds.
    async fn seed_from_address_book(&self) -> Result<AddressBookSeedOutcome, MemoryError> {
        self.policy.admit_write(
            Capability::People,
            "people.seed_from_address_book",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.seed_from_address_book().await
    }
}

// ── Chunks ───────────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryChunks for GuardedChunks {
    async fn list_chunks(
        &self,
        query: &ChunkQuery,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<Chunk>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.list_chunks",
            NO_NAMESPACE,
            false,
        )?;
        // Intersected with the ambient allowlist, never passed through. The
        // ambient scope is an upper bound: forwarding the caller's scope
        // unchanged would let a source-restricted turn widen itself back out by
        // naming a collection the restriction excluded. See
        // `GuardPolicy::narrow_scope`.
        let effective = self.policy.narrow_scope(scope);
        self.family()?.list_chunks(query, effective.as_ref()).await
    }

    /// The count that labels a [`Self::list_chunks`] page, and it must be
    /// narrowed by exactly the same rule. A total computed against a wider
    /// scope than the page it labels leaks the existence of rows the caller may
    /// not read — "showing 20 of 4000" tells a source-restricted turn how much
    /// it is not being shown.
    async fn count_chunks(
        &self,
        query: &ChunkQuery,
        scope: Option<&SourceScope>,
    ) -> Result<u64, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.count_chunks",
            NO_NAMESPACE,
            false,
        )?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?.count_chunks(query, effective.as_ref()).await
    }

    /// Same rows as [`Self::list_chunks`] with the stored facts beside them, so
    /// the same intersection applies for the same reason.
    async fn list_chunk_details(
        &self,
        query: &ChunkQuery,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<ChunkListRow>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.list_chunk_details",
            NO_NAMESPACE,
            false,
        )?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .list_chunk_details(query, effective.as_ref())
            .await
    }

    /// Per-source totals are computed from the chunks the scope admits, not
    /// filtered afterwards — so a restricted caller must not learn that a
    /// forbidden source exists by seeing its row, nor see a permitted source
    /// carrying a count that includes rows it cannot read.
    async fn source_totals(
        &self,
        limit: usize,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<SourceTotal>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.source_totals",
            NO_NAMESPACE,
            false,
        )?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .source_totals(limit, effective.as_ref())
            .await
    }

    async fn get_chunk(&self, chunk_id: &str) -> Result<Option<Chunk>, MemoryError> {
        self.policy
            .admit_read(Capability::Chunks, "chunks.get_chunk", NO_NAMESPACE, false)?;
        self.family()?.get_chunk(chunk_id).await
    }

    async fn chunk_detail(&self, chunk_id: &str) -> Result<Option<ChunkDetail>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.chunk_detail",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.chunk_detail(chunk_id).await
    }

    /// The catalog is not user content, so it takes no namespace and the
    /// lightest read check — refusing it under `readonly` would stop an
    /// operator finding out what the store can even hold.
    async fn storage_kinds(&self) -> Result<Vec<String>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.storage_kinds",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.storage_kinds().await
    }

    /// Vectors, not content — but still a read of stored material, so it takes
    /// the same tier check rather than being waved through as metadata.
    async fn chunk_embeddings(
        &self,
        chunk_ids: &[String],
        model_signature: &str,
    ) -> Result<Vec<ChunkEmbedding>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.chunk_embeddings",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .chunk_embeddings(chunk_ids, model_signature)
            .await
    }

    /// One chunk's admission verdict, read by chunk id exactly as
    /// [`Self::chunk_detail`] is — so it takes that member's check, not
    /// [`Self::list_chunks`]'s scope intersection. There is no scope to narrow:
    /// the caller already holds the id.
    async fn chunk_score(&self, chunk_id: &str) -> Result<Option<ChunkScore>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.chunk_score",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.chunk_score(chunk_id).await
    }

    /// Ingest progress for the sources the caller names, one row per query.
    ///
    /// Unlike [`Self::source_totals`], which enumerates the groups that exist
    /// and therefore has to be narrowed to the ambient allowlist, this answers
    /// only about prefixes the caller supplied — it discloses no source the
    /// caller did not already name — and the contract member carries no
    /// [`SourceScope`] for the guard to intersect anything into.
    async fn source_ingest_status(
        &self,
        source_prefixes: &[SourceIngestQuery],
    ) -> Result<Vec<SourceIngestStatus>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.source_ingest_status",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.source_ingest_status(source_prefixes).await
    }
}
