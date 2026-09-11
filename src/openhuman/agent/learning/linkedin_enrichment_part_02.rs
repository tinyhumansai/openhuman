
/// The guarded memory driver's documents family, for profile persistence.
///
/// Deliberately **not** an engine constructor. `MemoryClient::new_local()`
/// resolved `~/.openhuman/workspace` from the home directory and never
/// consulted `OPENHUMAN_WORKSPACE`, so on any host that scopes its workspace —
/// every hosted tenant, and any local run with the variable set — the scraped
/// profile was written to a store nothing else reads. It failed by
/// *succeeding*, which is why nothing surfaced it. It was also a second engine
/// construction against the same data directory, the hazard
/// `memory::bypass_allowlist_tests` refuses `MemoryClient::from_workspace_dir`
/// for ("risks a second ingestion worker on one store").
///
/// `active_memory_guard` cannot reintroduce either: it resolves the ambient
/// `CoreContext`'s workspace, falling back to `Config::load_or_init` — the
/// *same* workspace `write_profile_md` writes `PROFILE.md` into, so the two
/// halves of `run_linkedin_enrichment` cannot target different stores — and it
/// hands back the one cached driver for that workspace rather than building
/// anything.
///
/// The caller still treats an error as "skip persistence and warn", for the
/// genuine failures that remain.
async fn profile_memory_writer(
) -> anyhow::Result<std::sync::Arc<crate::openhuman::memory::guard::MemoryGuard>> {
    let guard = crate::openhuman::memory::ops::guard::active_memory_guard()
        .await
        .map_err(|e| anyhow::anyhow!("memory driver unavailable: {e}"))?;
    if guard.as_documents().is_none() {
        anyhow::bail!(
            "memory driver `{}` does not support the documents family",
            guard.driver_id()
        );
    }
    Ok(guard)
}

/// Upsert one LinkedIn-derived document through the guarded documents family.
///
/// This is `MemoryClient::store_skill_sync`'s body, minus the engine: that
/// method built exactly this `NamespaceDocumentInput` and handed it to
/// `put_doc`, which is what `MemoryDocuments::put_document` resolves to on the
/// embedded driver — same upsert, same background graph-extraction enqueue.
/// `ExternalSync` is carried explicitly for the same reason `store_skill_sync`
/// hard-coded it: a scraped third-party profile is not user-authored, and the
/// subconscious gate reads that provenance off the persisted chunk.
///
/// The dedup key is the title, matching `store_skill_sync`'s `document_id: None`
/// branch — LinkedIn enrichment has no stable upstream id to key on.
async fn put_profile_document(
    memory: &crate::openhuman::memory::guard::MemoryGuard,
    title: String,
    content: String,
    source_type: &str,
    priority: &str,
    metadata: serde_json::Value,
) -> anyhow::Result<()> {
    let documents = memory.as_documents().ok_or_else(|| {
        anyhow::anyhow!(
            "memory driver `{}` does not support the documents family",
            memory.driver_id()
        )
    })?;
    tracing::debug!(
        namespace = PROFILE_MEMORY_NAMESPACE,
        source_type = source_type,
        "[linkedin_enrichment] upserting profile document through the memory driver"
    );
    documents
        .put_document(NamespaceDocumentInput {
            namespace: PROFILE_MEMORY_NAMESPACE.to_string(),
            key: title.clone(),
            title,
            content,
            source_type: source_type.to_string(),
            priority: priority.to_string(),
            tags: Vec::new(),
            metadata,
            category: "core".to_string(),
            session_id: None,
            document_id: None,
            taint: MemoryTaint::ExternalSync,
        })
        .await
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("memory store failed: {e}"))
}

/// Persist the full scraped LinkedIn profile to the user-profile memory
/// namespace so the agent has rich context about the user.
async fn persist_linkedin_profile(
    memory: &crate::openhuman::memory::guard::MemoryGuard,
    url: &str,
    data: &serde_json::Value,
) -> anyhow::Result<()> {
    let content = format!(
        "LinkedIn profile for {url}:\n\n{}",
        serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
    );

    put_profile_document(
        memory,
        format!("LinkedIn profile: {url}"),
        content,
        "onboarding-linkedin-enrichment",
        "high",
        json!({
            "source": "apify-linkedin-scraper",
            "url": url,
            "actor": LINKEDIN_SCRAPER_ACTOR,
        }),
    )
    .await
}

/// Fallback: persist just the LinkedIn URL when the full scrape fails.
async fn persist_linkedin_url_only(
    memory: &crate::openhuman::memory::guard::MemoryGuard,
    url: &str,
) -> anyhow::Result<()> {
    put_profile_document(
        memory,
        format!("LinkedIn profile URL: {url}"),
        format!("User LinkedIn profile: {url}"),
        "onboarding-linkedin-url",
        "medium",
        json!({ "source": "gmail-linkedin-extraction", "url": url }),
    )
    .await
}
