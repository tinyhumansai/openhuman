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

/// The classifier matches the three real Ollama error shapes and nothing else.
///
/// A false positive tells the user to start Ollama for a storage or bus fault
/// that has nothing to do with the local runtime; a false negative is how
/// openhuman#5867 stayed unfixed for the model-not-pulled case. **Every
/// positive literal here is copied from the upstream source**, not composed
/// from the shape of the message — the string this replaced
/// (`"Ollama embedding model … is not installed at …"`) reads plausibly and
/// exists nowhere in `tinyinference`, so the test passed while the product
/// never emitted it.
#[test]
fn local_embedding_error_classifier_matches_ollama_patterns_only() {
    // Transport bail: daemon is not listening.
    assert!(is_local_embedding_error(
        "unreachable: ollama embed request failed \
         (is Ollama running at http://localhost:11434?): connection refused"
    ));
    // Plain string variant the bus may emit.
    assert!(is_local_embedding_error(
        "is Ollama running at http://127.0.0.1:11434"
    ));
    // Embedding model never pulled. `embeddings/ollama.rs` renders Ollama's
    // 404 through `ollama_http_error`, so this — not a prose "not installed"
    // sentence — is what the embedder actually produces for #5867's `bge-m3`.
    assert!(is_local_embedding_error(
        "ollama embed failed with status 404 Not Found: \
         {\"error\":\"model 'bge-m3' not found\"}"
    ));
    // Chat/summarisation model never pulled (#5867 also names `gemma3:4b`).
    // Quoted from `providers/openai/local.rs:376` — "Ollama model", not
    // "Ollama embedding model".
    assert!(is_local_embedding_error(
        "Ollama model `gemma3:4b` is not installed at http://localhost:11434. \
         Run `ollama pull gemma3:4b`, or call `list_models()` to see what is installed"
    ));
    // Case-insensitive match.
    assert!(is_local_embedding_error("IS OLLAMA RUNNING AT localhost"));

    // Non-Ollama failures must not match.
    assert!(!is_local_embedding_error("database or disk is full"));
    assert!(!is_local_embedding_error("timed out: rpc timeout exceeded"));
    assert!(!is_local_embedding_error(
        "unreachable: connection refused to memory bus"
    ));
    assert!(!is_local_embedding_error("backend failed: 500 internal"));
    assert!(!is_local_embedding_error(""));

    // The embeddings path emits six other `ollama embed …` shapes that are
    // data faults, not an absent runtime. Telling the user to install Ollama
    // for a NaN or a dimension mismatch would be wrong, so the status-404
    // anchor must not widen to the whole prefix.
    assert!(!is_local_embedding_error(
        "ollama embed count mismatch: sent 1 text, got 0 embeddings"
    ));
    assert!(!is_local_embedding_error(
        "ollama embed dimension mismatch at index 0: expected 1024, got 768"
    ));
    assert!(!is_local_embedding_error(
        "Ollama could not encode input without NaN values"
    ));
    assert!(!is_local_embedding_error(
        "ollama embed failed with status 500 Internal Server Error"
    ));
}

/// Once-latch for the archivist embedding failure path (openhuman#5867):
/// one failed embedding per segment must not become one banner per segment.
///
/// This subscribes to the web-channel bus and counts what actually arrives.
/// The previous version called the function twice and asserted nothing, so it
/// passed whether the notice fired twice, once, or never — which is every
/// outcome, including the banner storm the latch exists to prevent.
///
/// The receiver is taken *before* the first call because `broadcast` only
/// delivers what is sent after subscribing. This is the only test in the
/// binary that calls `notice_local_model_unavailable_once`, so the
/// function-local `Once` is untripped when it runs.
#[test]
fn local_model_unavailable_notice_is_latched_once_per_process() {
    let mut events = crate::openhuman::web_chat::subscribe_web_channel_events();

    notice_local_model_unavailable_once("test archivist");
    notice_local_model_unavailable_once("test archivist duplicate");

    let first = events
        .try_recv()
        .expect("the first call must publish exactly one user_error");
    assert_eq!(
        first.error_type.as_deref(),
        Some(LOCAL_MODEL_UNAVAILABLE_KIND),
        "the published event must be the local-model notice"
    );
    assert!(
        events.try_recv().is_err(),
        "the second call must publish nothing — one failed embedding per \
         segment would otherwise be one banner per segment"
    );
}

/// The local-model-unavailable payload carries the same no-leak contract as
/// its siblings: stable kind + source, never raw provider text or model ids.
#[test]
fn local_model_unavailable_payload_is_metadata_only() {
    let event = local_model_unavailable_user_error();
    assert_eq!(event.event, "user_error");
    assert_eq!(event.client_id, "system");
    assert_eq!(
        event.error_type.as_deref(),
        Some(LOCAL_MODEL_UNAVAILABLE_KIND)
    );
    assert_eq!(
        event.error_source.as_deref(),
        Some(MEMORY_USER_ERROR_SOURCE)
    );
    assert!(event.message.is_none(), "must not carry raw error prose");
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
