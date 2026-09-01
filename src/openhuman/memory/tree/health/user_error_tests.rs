use super::*;

/// Pins the wire shape the frontend `socketService` handler reads, plus the
/// metadata-only no-leak contract.
#[test]
fn payload_is_metadata_only() {
    let event = local_model_unavailable_user_error();

    assert_eq!(event.event, "user_error");
    // The "system" room is the one every socket auto-joins.
    assert_eq!(event.client_id, "system");
    assert_eq!(
        event.error_type.as_deref(),
        Some(LOCAL_MODEL_UNAVAILABLE_KIND)
    );
    assert_eq!(
        event.error_source.as_deref(),
        Some(MEMORY_USER_ERROR_SOURCE)
    );

    // Nothing that could carry the base URL, a model id, or raw provider
    // prose may ride along.
    assert!(event.message.is_none(), "must not carry raw error prose");
    assert!(event.full_response.is_none());
    assert!(event.thread_id.is_empty());
}

/// The kind token is a cross-language contract: `app/src/types/userError.ts`
/// declares this exact `UserErrorKind` discriminator and `classify.ts` keys
/// on it. A rename on either side drops the signal with no compile error on
/// either side, so pin the wire string.
#[test]
fn kind_matches_frontend_discriminator() {
    assert_eq!(LOCAL_MODEL_UNAVAILABLE_KIND, "local_model_unavailable");
}

/// `socketService` only maps `error_source == "memory"` onto the `memory`
/// scope; anything else falls back to the historical `cron` default, which
/// would file this entry under the wrong heading.
#[test]
fn source_matches_frontend_scope_mapping() {
    assert_eq!(MEMORY_USER_ERROR_SOURCE, "memory");
}

/// Same no-leak contract for the corrupt-store payload (openhuman#5820):
/// the stable kind token and the memory source, never the quarantined
/// path or SQLite prose.
#[test]
fn corrupt_store_payload_is_metadata_only() {
    let event = store_corrupt_quarantined_user_error();

    assert_eq!(event.event, "user_error");
    assert_eq!(event.client_id, "system");
    assert_eq!(event.error_type.as_deref(), Some(STORE_CORRUPT_KIND));
    assert_eq!(
        event.error_source.as_deref(),
        Some(MEMORY_USER_ERROR_SOURCE)
    );
    assert!(event.message.is_none(), "must not carry raw error prose");
    assert!(event.full_response.is_none());
    assert!(event.thread_id.is_empty());
}

/// The corrupt kind token is the same cross-language contract as the
/// local-model one: `classify.ts` keys on exactly this string.
#[test]
fn corrupt_kind_matches_frontend_discriminator() {
    assert_eq!(STORE_CORRUPT_KIND, "memory_store_corrupt");
}

/// The wire-text classifier matches SQLite's two corruption renderings —
/// the shapes a `MemoryError` string carries after crossing the bus — and
/// nothing else. Quarantine-adjacent decisions key on this, so a false
/// positive would raise a "memory quarantined" notice for a healthy store.
#[test]
fn corrupt_text_classifier_matches_sqlite_renderings_only() {
    assert!(is_corrupt_store_error(
        "memory-tree ingest failed for source `conversations:agent`: \
         database disk image is malformed"
    ));
    assert!(is_corrupt_store_error(
        "open failed: File is NOT a Database"
    ));
    assert!(!is_corrupt_store_error("database or disk is full"));
    assert!(!is_corrupt_store_error("rate limited (429)"));
    assert!(!is_corrupt_store_error(""));
}

/// The once-latch bounds the archivist's per-segment detection to one
/// notice per process — 747 failing segments in the incident must not
/// become 747 notices. (The engine's own quarantine event is un-latched
/// and stays the authoritative per-quarantine notice.)
#[test]
fn wire_notice_is_latched_once_per_process() {
    // Publishing twice must be safe and quiet; the second call returns on
    // the latch. There is no socket in unit tests, so the observable
    // contract is "no panic, no double side effects on the latch path".
    notice_corrupt_store_once("test detector");
    notice_corrupt_store_once("test detector");
}

// ── memory module unavailable ────────────────────────────────────────────────

/// Same no-leak contract as the corrupt-store payload: stable kind + source,
/// and none of the loader's raw text (which carries release URLs and paths).
#[test]
fn module_unavailable_payload_is_metadata_only() {
    let event = super::memory_module_unavailable_user_error();
    assert_eq!(event.event, "user_error");
    assert_eq!(event.client_id, "system");
    assert_eq!(
        event.error_type.as_deref(),
        Some(super::MEMORY_MODULE_UNAVAILABLE_KIND)
    );
    assert_eq!(
        event.error_source.as_deref(),
        Some(tinymemory_api::host::MEMORY_USER_ERROR_SOURCE)
    );
    let wire = serde_json::to_string(&event).expect("payload serializes");
    assert!(
        !wire.contains("github") && !wire.contains("http") && !wire.contains('/'),
        "no loader detail may reach the wire: {wire}"
    );
}
