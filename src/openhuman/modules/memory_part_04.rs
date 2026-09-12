#[async_trait]
impl MemoryProvider for ModuleMemoryProvider {
    fn driver_id(&self) -> &str {
        &self.driver_id
    }

    /// The families the **pinned artifact** serves — not the whole contract.
    ///
    /// # This couples to the registry pin, and the coupling IS enforced
    ///
    /// `Capabilities::all()` grows whenever a family is added to the contract,
    /// but the *artifact* only grows when a release is cut and
    /// [`registry`](super::registry) is re-pinned to it. Returning `all()`
    /// between those two moments over-claimed: the host said it could do
    /// something the loaded binary could not, and [`Self::verify`] noticed and
    /// logged it without narrowing the advertised set. The failure mode was a
    /// call that reached the module and came back `UnknownMethod` (#5598)
    /// rather than a family that cleanly reported itself absent.
    ///
    /// [`ARTIFACT_CAPABILITIES`] is now the source of truth, and
    /// `the_capability_list_matches_the_pinned_release` fails if it is widened
    /// without moving [`ARTIFACT_CAPABILITIES_PIN`] and the registry pin
    /// together.
    ///
    /// The kernel filters its RPC surface and agent-tool assembly from this set,
    /// and the guard builds one family decorator per `provides()`, so an
    /// over-claim here is precisely what turns an absent family into a live
    /// method that answers `UnknownMethod`.
    fn capabilities(&self) -> Capabilities {
        artifact_capabilities()
    }

    async fn health(&self) -> MemoryHealth {
        // An unreachable module is a *health* answer, not an error: that is the
        // question this method exists to answer, and returning `Down` is how
        // status output shows an unsupported platform or a refused artifact.
        //
        // A module that is still loading is *not* down. `Down` is the signal
        // that tells the kernel to give up on this driver and rebind the
        // fallback, and a cold launch on a slow link must not trip it; the
        // driver is serving, just not yet. Configuration stays authoritative:
        // a host whose modules are switched off is down whatever the
        // process-wide table says about a load someone else started.
        let modules_enabled = self
            .config
            .as_ref()
            .or_else(|| policy())
            .is_some_and(|config| config.modules.enabled);
        if modules_enabled && matches!(ops::state_of(MODULE_ID), super::types::ModuleState::Loading)
        {
            return MemoryHealth::degraded("the memory module is loading");
        }
        match self.proxy("health").await {
            Ok(proxy) => proxy
                .call::<MemoryHealth>("Health", ())
                .await
                .unwrap_or_else(|error| MemoryHealth::down(error.to_string())),
            Err(MemoryError::Unavailable(reason)) => MemoryHealth::degraded(reason),
            Err(error) => MemoryHealth::down(error.to_string()),
        }
    }

    async fn shutdown(&self) -> Result<(), MemoryError> {
        // Deliberately does not load the module in order to shut it down: a
        // shutdown on a driver that was never used should be a no-op, not a
        // download. tinybus never unloads a library anyway, so this releases
        // backend resources only.
        if self.verified.get().is_none() {
            return Ok(());
        }
        let proxy = self.proxy("shutdown").await?;
        proxy
            .call::<()>(methods::SHUTDOWN, ())
            .await
            .map_err(|error| from_bus(&error))
    }

    fn as_ingest(&self) -> Option<&dyn MemoryIngest> {
        Some(self)
    }
    fn as_documents(&self) -> Option<&dyn MemoryDocuments> {
        Some(self)
    }
    fn as_tree(&self) -> Option<&dyn MemoryTree> {
        Some(self)
    }
    fn as_entities(&self) -> Option<&dyn MemoryEntities> {
        Some(self)
    }
    fn as_graph(&self) -> Option<&dyn MemoryGraph> {
        Some(self)
    }
    fn as_diff(&self) -> Option<&dyn MemoryDiff> {
        Some(self)
    }
    fn as_goals(&self) -> Option<&dyn MemoryGoals> {
        Some(self)
    }
    fn as_tool_memory(&self) -> Option<&dyn MemoryToolMemory> {
        Some(self)
    }
    fn as_sources(&self) -> Option<&dyn MemorySourceSink> {
        Some(self)
    }
    fn as_maintenance(&self) -> Option<&dyn MemoryMaintenance> {
        Some(self)
    }
    // The four families below are gated on the pinned artifact rather than
    // returning `Some(self)` unconditionally. `provides()` derives from these
    // accessors, the guard builds its decorators from `provides()`, and every
    // caller already writes a clean "driver does not support the X family"
    // error on `None` — so gating here converts a deep `UnknownMethod` into an
    // early, accurate refusal at every call site at once (#5598).
    fn as_people(&self) -> Option<&dyn MemoryPeople> {
        artifact_serves(Capability::People).then_some(self as &dyn MemoryPeople)
    }
    fn as_chunks(&self) -> Option<&dyn MemoryChunks> {
        artifact_serves(Capability::Chunks).then_some(self as &dyn MemoryChunks)
    }
    fn as_retrieval(&self) -> Option<&dyn MemoryRetrieval> {
        artifact_serves(Capability::Retrieval).then_some(self as &dyn MemoryRetrieval)
    }
    fn as_profile(&self) -> Option<&dyn MemoryProfile> {
        artifact_serves(Capability::Profile).then_some(self as &dyn MemoryProfile)
    }

    fn as_source_sync(&self) -> Option<&dyn MemorySourceSync> {
        artifact_serves(Capability::SourceSync).then_some(self as &dyn MemorySourceSync)
    }

    fn as_coding_sessions(&self) -> Option<&dyn MemoryCodingSessions> {
        artifact_serves(Capability::CodingSessions).then_some(self as &dyn MemoryCodingSessions)
    }
    fn as_episodic(&self) -> Option<&dyn MemoryEpisodic> {
        artifact_serves(Capability::Episodic).then_some(self as &dyn MemoryEpisodic)
    }
    fn as_scoring(&self) -> Option<&dyn MemoryScoring> {
        artifact_serves(Capability::Scoring).then_some(self as &dyn MemoryScoring)
    }
    fn as_document_ingest(&self) -> Option<&dyn MemoryDocumentIngest> {
        artifact_serves(Capability::DocumentIngest).then_some(self as &dyn MemoryDocumentIngest)
    }
    fn as_conversation_ingest(&self) -> Option<&dyn MemoryConversationIngest> {
        artifact_serves(Capability::ConversationIngest)
            .then_some(self as &dyn MemoryConversationIngest)
    }
    fn as_learning_ingest(&self) -> Option<&dyn MemoryLearningIngest> {
        artifact_serves(Capability::LearningIngest).then_some(self as &dyn MemoryLearningIngest)
    }
    fn as_event_ingest(&self) -> Option<&dyn MemoryEventIngest> {
        artifact_serves(Capability::EventIngest).then_some(self as &dyn MemoryEventIngest)
    }
    fn as_answer(&self) -> Option<&dyn MemoryAnswer> {
        artifact_serves(Capability::Answer).then_some(self as &dyn MemoryAnswer)
    }
}

#[async_trait]
impl MemoryCore for ModuleMemoryProvider {
    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> Result<(), MemoryError> {
        // No log line carries `namespace`, `key` or `content`: all three are user
        // memory content.
        self.proxy("store")
            .await?
            .call::<()>(
                methods::STORE,
                (
                    namespace,
                    key,
                    content,
                    category,
                    session_id.map(str::to_string),
                    taint,
                ),
            )
            .await
            .map_err(|error| from_bus(&error))
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        self.proxy("get")
            .await?
            .call(methods::GET, (namespace, key))
            .await
            .map_err(|error| from_bus(&error))
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool, MemoryError> {
        self.proxy("forget")
            .await?
            .call(methods::FORGET, (namespace, key))
            .await
            .map_err(|error| from_bus(&error))
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.proxy("list")
            .await?
            .call(
                methods::LIST,
                (
                    namespace.map(str::to_string),
                    category.cloned(),
                    session_id.map(str::to_string),
                ),
            )
            .await
            .map_err(|error| from_bus(&error))
    }

    async fn namespaces(&self) -> Result<Vec<NamespaceSummary>, MemoryError> {
        self.proxy("namespaces")
            .await?
            .call(methods::NAMESPACES, ())
            .await
            .map_err(|error| from_bus(&error))
    }
}

#[async_trait]
impl MemoryRecall for ModuleMemoryProvider {
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: &OwnedRecallOpts,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        // `scope` crosses as a value because the driver must apply it as a query
        // predicate internally; narrowing the result here instead would let the
        // module spend its `limit` on entries the caller may not see.
        self.proxy("recall")
            .await?
            .call(methods::RECALL, (query, limit, opts, scope.cloned()))
            .await
            .map_err(|error| from_bus(&error))
    }
}

#[async_trait]
impl MemoryPortability for ModuleMemoryProvider {
    async fn export_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ExportPage, MemoryError> {
        self.proxy("export_page")
            .await?
            .call(methods::EXPORT_PAGE, (cursor.map(str::to_string), limit))
            .await
            .map_err(|error| from_bus(&error))
    }

    async fn import_records(
        &self,
        records: Vec<ExportRecord>,
    ) -> Result<ImportOutcome, MemoryError> {
        self.proxy("import_records")
            .await?
            .call(methods::IMPORT_RECORDS, (records,))
            .await
            .map_err(|error| from_bus(&error))
    }
}
#[async_trait]
impl MemoryIngest for ModuleMemoryProvider {
    async fn ingest_document(&self, item: IngestItem) -> Result<IngestOutcome, MemoryError> {
        module_call!(self, "ingest_document", methods::INGEST_DOCUMENT, (item,))
    }
    async fn ingest_chat(&self, messages: Vec<IngestItem>) -> Result<IngestOutcome, MemoryError> {
        module_call_slow!(self, "ingest_chat", methods::INGEST_CHAT, (messages,))
    }
    async fn ingest_email(&self, messages: Vec<IngestItem>) -> Result<IngestOutcome, MemoryError> {
        module_call_slow!(self, "ingest_email", methods::INGEST_EMAIL, (messages,))
    }
}
