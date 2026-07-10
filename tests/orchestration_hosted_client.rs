//! Unit coverage for the device-side hosted-orchestration client: the world-diff
//! observation buffer, the world-observation note builder, and the evict-effect
//! parsing. These paths need no backend, so they run in the plain integration
//! crate (the root crate's `cfg(test)` build is gated elsewhere).

use openhuman_core::openhuman::orchestration::effect_executor::{
    effect_result_frame, is_duplicate_call, parse_evict, release_call,
};
use openhuman_core::openhuman::orchestration::store;
use openhuman_core::openhuman::orchestration::wire::OrchestrationEventEnvelopeWire;
use openhuman_core::openhuman::orchestration::world_model::observe_ingest_note;

// ── world_model: bounded, single-line, never leaks the body ───────────────────

#[test]
fn observe_ingest_note_summarises_without_the_body() {
    let note = observe_ingest_note("h-1", "@peer", "dm", "super secret plaintext body");
    // The body is summarised to a char count, never copied.
    assert!(!note.contains("super secret plaintext body"));
    assert!(note.contains("@peer"));
    assert!(note.contains("h-1"));
    assert!(note.contains("chars"));
    // Single line, bounded.
    assert!(!note.contains('\n'));
    assert!(note.chars().count() <= 240);
}

#[test]
fn observe_ingest_note_defaults_empty_session_and_kind() {
    let note = observe_ingest_note("", "@peer", "", "");
    assert!(note.contains("master")); // empty session → master
    assert!(note.contains("message")); // empty kind → message
    assert!(note.contains("empty")); // empty body → "empty"
}

#[test]
fn observe_ingest_note_collapses_newlines_and_clamps() {
    let long = "x".repeat(5000);
    let note = observe_ingest_note("h-1", "line1\nline2", "dm", &long);
    assert!(!note.contains('\n'));
    assert!(note.chars().count() <= 240);
}

// ── evict effect parsing + ack frame ──────────────────────────────────────────

#[test]
fn parse_evict_reads_the_backend_frame_shape() {
    let frame = serde_json::json!({
        "cycleId": "cyc:1",
        "callId": "cyc:1:evict:0",
        "sessionId": "h-1",
        "entries": [
            { "cycleId": "cyc:0", "summary": "user asked about billing" },
            { "cycleId": "cyc:1", "summary": "resolved via refund" },
        ],
    });
    let effect = parse_evict(&frame).expect("valid evict frame");
    assert_eq!(effect.call_id, "cyc:1:evict:0");
    assert_eq!(effect.session_id, "h-1");
    assert_eq!(effect.entries.len(), 2);
    assert_eq!(effect.entries[0].summary, "user asked about billing");
}

#[test]
fn parse_evict_rejects_a_frame_without_call_id() {
    let frame = serde_json::json!({ "sessionId": "h-1", "entries": [] });
    assert!(parse_evict(&frame).is_err());
}

#[test]
fn effect_result_frame_has_the_ack_shape() {
    let ok = effect_result_frame("c-1", true, None);
    assert_eq!(ok["callId"], "c-1");
    assert_eq!(ok["ok"], true);
    let err = effect_result_frame("c-2", false, Some("boom"));
    assert_eq!(err["ok"], false);
    assert_eq!(err["error"], "boom");
}

#[test]
fn is_duplicate_call_is_true_only_on_the_second_sight() {
    let id = "unique-call-id-orchestration-hosted-test";
    assert!(!is_duplicate_call(id), "first sight is not a duplicate");
    assert!(is_duplicate_call(id), "second sight is a duplicate");
}

#[test]
fn release_call_lets_a_failed_effect_be_retried() {
    // A claim whose effect FAILS is released so the hosted brain's redelivery
    // re-executes it, rather than the guard re-acking a stale success and dropping
    // the work. (Unique id so it can't collide with the other dedupe test's id in
    // the process-global set.)
    let id = "unique-call-id-orchestration-release-test";
    assert!(!is_duplicate_call(id), "first claim");
    assert!(
        is_duplicate_call(id),
        "still claimed while the effect is in flight"
    );
    release_call(id);
    assert!(
        !is_duplicate_call(id),
        "a released (failed) id is claimable again"
    );
}

// ── world_obs device buffer round-trip ────────────────────────────────────────

#[test]
fn world_obs_buffer_appends_monotonic_drains_fifo_and_deletes() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();

    // Append three observations; seq is globally monotonic.
    let (s1, s2, s3) = store::with_connection(&ws, |conn| {
        let s1 = store::append_world_obs(conn, "h-1", "note-1", 100)?;
        let s2 = store::append_world_obs(conn, "h-1", "note-2", 200)?;
        let s3 = store::append_world_obs(conn, "h-2", "note-3", 300)?;
        Ok((s1, s2, s3))
    })
    .unwrap();
    assert_eq!((s1, s2, s3), (1, 2, 3));

    // Drain FIFO by insert order.
    let rows = store::with_connection(&ws, |conn| store::drain_world_obs(conn, 10)).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].note, "note-1");
    assert_eq!(rows[0].session_id, "h-1");
    assert_eq!(rows[2].session_id, "h-2");

    // Delete the first two; the third remains buffered (retry semantics).
    let keep = rows[2].id;
    store::with_connection(&ws, |conn| {
        store::delete_world_obs(conn, &[rows[0].id, rows[1].id])
    })
    .unwrap();
    let remaining = store::with_connection(&ws, |conn| store::drain_world_obs(conn, 10)).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, keep);
    assert_eq!(remaining[0].note, "note-3");
}

#[test]
fn forwarded_counterpart_id_is_not_re_encoded() {
    // Load-bearing invariant for sync's re-paging guard: the device forwards
    // `counterpartAgentId` as the exact base64 `envelope.from` it also stores as the
    // local `agent_id`, and sync keys on the backend's verbatim echo of that string.
    // Pin that the client never transforms the encoding here — if it did (or if the
    // backend re-encoded), `next_session_seq` would miss the device's own rows and
    // re-page every hosted turn under a second encoding, duplicating the session.
    let agent = "ZCAAuA+2GVoRrT08Gt8JUVnxnISTelSxnDuyScze334=";
    let env =
        OrchestrationEventEnvelopeWire::build(agent, "sess-1", 3, "user", agent, "hi", 100, "dm");
    assert_eq!(env.counterpart_agent_id, agent);
    assert_eq!(env.event.sender, agent);
}

#[test]
fn world_obs_seq_never_resets_after_a_full_drain() {
    // Regression: `seq` must be a persistent monotonic counter, not `MAX(seq)+1`
    // over the table. The uploader deletes every row after a successful push, so a
    // table-scoped max would restart at 1 once the buffer empties and reuse seqs the
    // backend already saw — its `(userId, sessionId, seq)` dedupe would then silently
    // drop the fresh observation.
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();

    let s1 =
        store::with_connection(&ws, |conn| store::append_world_obs(conn, "h-1", "n1", 1)).unwrap();
    // Drain and delete everything, emptying the buffer.
    let rows = store::with_connection(&ws, |conn| store::drain_world_obs(conn, 10)).unwrap();
    let ids: Vec<i64> = rows.iter().map(|o| o.id).collect();
    store::with_connection(&ws, |conn| store::delete_world_obs(conn, &ids)).unwrap();

    // The next ordinal must climb past s1, not reset to 1.
    let s2 =
        store::with_connection(&ws, |conn| store::append_world_obs(conn, "h-1", "n2", 2)).unwrap();
    assert!(s2 > s1, "seq reset after a full drain: s1={s1} s2={s2}");
}

#[test]
fn world_obs_drain_respects_the_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();
    store::with_connection(&ws, |conn| {
        for i in 0..5 {
            store::append_world_obs(conn, "h-1", &format!("n{i}"), i)?;
        }
        Ok(())
    })
    .unwrap();
    let rows = store::with_connection(&ws, |conn| store::drain_world_obs(conn, 2)).unwrap();
    assert_eq!(rows.len(), 2);
}
