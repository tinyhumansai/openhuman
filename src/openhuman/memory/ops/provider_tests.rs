use super::*;

#[tokio::test]
async fn status_without_a_context_reports_an_unresolved_slot() {
    let status = unresolved_status("no workspace".to_string());
    assert_eq!(status.slot, "memory");
    assert_eq!(status.class, "null");
    assert_eq!(status.health, "down");
    assert!(status.capabilities.is_empty());
    assert_eq!(status.last_error.as_deref(), Some("no workspace"));
    // Still reports the contract version this build speaks — that is a
    // build fact, independent of whether anything bound.
    assert_eq!(
        status.contract_version,
        crate::core::subsystem::format_contract_version(
            crate::openhuman::memory::api::CONTRACT_VERSION
        )
    );
}

#[tokio::test]
async fn bound_driver_status_reports_id_class_contract_and_capabilities() {
    // `health` below is `degraded` for as long as the module is loading, and
    // the load is process-wide: without this the assertion races whichever
    // sibling test asked for the module first (openhuman#6172).
    crate::openhuman::memory::test_support::settle_memory_module().await;
    let workspace = tempfile::tempdir().expect("tempdir");
    let cfg = crate::openhuman::config::schema::MemorySubsystemConfig::default();
    let binding = crate::openhuman::memory::binding::for_workspace(workspace.path(), &cfg)
        .expect("binding resolves");

    let status = status_from_binding(&binding).await;
    assert_eq!(status.slot, "memory");
    // The legacy default id is normalized to the compiled TinyMemory
    // module at the binding boundary.
    assert_eq!(status.driver, crate::openhuman::memory::binding::MODULE_ID);
    assert_eq!(status.class, "module");
    assert_eq!(status.health, "ready");
    assert_eq!(
        status.contract_version,
        crate::core::subsystem::format_contract_version(
            crate::openhuman::memory::api::CONTRACT_VERSION
        )
    );
    // The full eighteen families the pinned tinymemory artifact serves.
    // Previously this comment documented a narrower set (seventeen, then
    // thirteen) while the host under-claimed or over-claimed; the gap is
    // now closed: v1.4.0 serves all eighteen, `ModuleMemoryProvider`
    // implements all twenty accessors including `as_episodic`, so the
    // wire surface and the advertised set agree.
    // `modules::memory::ARTIFACT_CAPABILITIES` is the machine-checked
    // source of truth for what the pinned release serves (see its module
    // docs); this pins the same corrected boundary. Spelled out rather
    // than derived from `Capabilities::all()` on purpose: this is the
    // wire surface the frontend reads, so the strings themselves are the
    // assertion. A NEW contract family must still widen this deliberately,
    // together with the registry version bump — not silently.
    assert_eq!(
        status.capabilities,
        vec![
            // Widened to eighteen with the Episodic accessor, then twenty when
            // tinymemory v1.7.0 added SourceSync and CodingSessions —
            // landing — the archivist writes its turns and segments
            // through that family, so hiding it here would be the
            // under-claim. The list stays spelled out: a NEW contract
            // family must still widen this deliberately, with its
            // accessor and its release.
            // v1.7.0's two families. Deliberately widened here rather than
            // derived: this list is the wire surface the frontend reads, so
            // a new family has to be a decision someone made, not something
            // that appeared.
            // Added with tinymemory v1.13.0: the MemoryScoring bus family
            // (#5560). Widened deliberately — the wire surface the frontend
            // reads must be an explicit decision, not a silent addition.
            // Widened to twenty-six with v1.13.7: the typed ingestion round
            // (document/conversation/learning/event) and the answer surface.
            "answer",
            "chunks",
            "coding_sessions",
            "conversation_ingest",
            "core",
            "diff",
            "document_ingest",
            "documents",
            "entities",
            "episodic",
            "event_ingest",
            "goals",
            "graph",
            "ingest",
            "learning_ingest",
            "maintenance",
            "people",
            "portability",
            "profile",
            "recall",
            "retrieval",
            "scoring",
            "source_sync",
            "sources",
            "tool_memory",
            "tree",
        ],
    );
    assert_eq!(status.fell_back_from, None);
    assert_eq!(status.last_error, None);
}

#[tokio::test]
async fn a_refused_driver_reports_the_fallback_and_its_reason() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let mut cfg = crate::openhuman::config::schema::MemorySubsystemConfig {
        driver: "supermemory".into(),
        ..Default::default()
    };
    cfg.drivers.insert(
        "supermemory".into(),
        crate::openhuman::config::schema::MemoryDriverConfig {
            class: Some("external".into()),
            transport: Some("http".into()),
            endpoint: Some("https://api.supermemory.ai".into()),
            credential_ref: Some("keychain:supermemory".into()),
            trust_state: "untrusted".into(),
        },
    );
    let binding = crate::openhuman::memory::binding::for_workspace(workspace.path(), &cfg)
        .expect("binding falls back rather than failing");

    let status = status_from_binding(&binding).await;
    assert_eq!(status.driver, "null");
    assert_eq!(status.class, "null");
    assert_eq!(status.fell_back_from.as_deref(), Some("supermemory"));
    let last_error = status.last_error.expect("a refused bind records why");
    assert!(last_error.contains("supermemory"), "{last_error}");
    assert!(last_error.contains("untrusted"), "{last_error}");
    // The refusal reason must never leak the credential reference or the
    // endpoint — same rule the binding's own tests pin.
    assert!(!last_error.contains("keychain:supermemory"), "{last_error}");
    assert!(!last_error.contains("api.supermemory.ai"), "{last_error}");
}

#[tokio::test]
async fn provider_status_wraps_the_snapshot_with_no_logs() {
    let outcome = memory_provider_status().await;
    assert!(outcome.logs.is_empty());
    assert_eq!(outcome.value.slot, "memory");
}
