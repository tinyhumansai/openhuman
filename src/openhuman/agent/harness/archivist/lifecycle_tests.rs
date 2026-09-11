//! Finalize-path tests for `on_segment_closed` (#6156).
//!
//! These drive the hook through the **contract**, not an engine: a
//! `RecordingProvider` serves every family and records what actually reached
//! it, so "the write did not happen" is asserted against the driver's call log
//! rather than inferred from an empty store.
//!
//! `ArchivistHook::new` leaves `summariser_available == false`, which is what
//! makes the heuristic arm of `summarize_entries` deterministic here — no chat
//! model is built, resolved or called anywhere in these tests.

use super::*;
use crate::openhuman::memory::api::provider::SegmentStatus;
use crate::openhuman::memory::guard::test_support::RecordingProvider;

const SESSION: &str = "lifecycle-6156";

/// One episodic turn. `id` is load-bearing: the episodic read path builds its
/// `SessionEntry`s with `sequence: None`, so segment membership is decided on
/// `turn.id` against the segment's episodic-id bounds.
fn turn(id: i64, role: &str, content: &str) -> EpisodicTurn {
    EpisodicTurn {
        id: Some(id),
        session_id: SESSION.into(),
        timestamp: 100.0 + id as f64,
        role: role.into(),
        content: content.into(),
        lesson: None,
        tool_calls_json: None,
        cost_microdollars: 0,
    }
}

/// A closed segment spanning both turns above.
///
/// `start_seq`/`end_seq` are deliberately `None`: the episodic read path builds
/// its `SessionEntry`s with `sequence: None`, so membership is decided on
/// `turn.id` against the episodic-id bounds.
fn segment() -> ConversationSegment {
    ConversationSegment {
        segment_id: "seg-6156".into(),
        session_id: SESSION.into(),
        namespace: "global".into(),
        start_episodic_id: 1,
        end_episodic_id: Some(2),
        start_timestamp: 100.0,
        end_timestamp: Some(103.0),
        turn_count: 2,
        summary: None,
        embedding: None,
        open: false,
        // #6186: the lifecycle marker the contract now carries. `Closed`
        // is what a segment whose recap failed is left as.
        status: Some(SegmentStatus::Closed),
        start_seq: None,
        end_seq: None,
    }
}

/// The driver's call log reduced to method names — what these tests assert on.
fn methods(recording: &RecordingProvider) -> Vec<String> {
    recording.calls().into_iter().map(|c| c.method).collect()
}

/// The bug in #6156: a heuristic bookend was persisted as the segment's durable
/// summary and embedded, both unconditionally.
///
/// Asserting `episodic.session_turns` is present matters as much as the
/// absences — without it the test would also pass if `on_segment_closed` had
/// bailed out before reaching the recap at all.
#[tokio::test]
async fn heuristic_recap_is_not_persisted_or_embedded() {
    let recording = Arc::new(RecordingProvider::new().with_session_turns(vec![
        turn(1, "user", "How do I pin a submodule?"),
        turn(2, "assistant", "Record the gitlink at the commit you want."),
    ]));
    let provider: Arc<dyn MemoryProvider> = recording.clone();
    let hook = ArchivistHook::new(provider, true);

    let recap_succeeded = hook.on_segment_closed(&segment(), SESSION, 200.0).await;

    // #6186: the caller gates the re-summarisation pass on this. Reporting
    // `true` here would make an outage spend every pending segment's retry
    // budget against the provider that is still down, and skip them for the
    // rest of the process — including after it recovers.
    assert!(
        !recap_succeeded,
        "a heuristic recap must not report the summariser as answering"
    );

    let methods = methods(&recording);
    assert!(
        methods.iter().any(|m| m == "episodic.session_turns"),
        "the finalize path must have read the segment's turns; got {methods:?}"
    );
    for forbidden in [
        "episodic.set_segment_summary",
        "scoring.embedder_slug",
        "scoring.embed_text",
        "episodic.upsert_segment_embedding",
    ] {
        assert!(
            !methods.iter().any(|m| m == forbidden),
            "{forbidden} must not run for a heuristic recap; got {methods:?}"
        );
    }
}

/// A segment whose turns are all outside its bounds short-circuits before the
/// recap — the pre-existing empty-entries exit, not the new heuristic arm.
///
/// Pinned so the new branch cannot quietly become the thing that handles an
/// empty segment.
#[tokio::test]
async fn empty_segment_still_short_circuits() {
    let recording = Arc::new(RecordingProvider::new());
    let provider: Arc<dyn MemoryProvider> = recording.clone();
    let hook = ArchivistHook::new(provider, true);

    hook.on_segment_closed(&segment(), SESSION, 200.0).await;

    let methods = methods(&recording);
    assert!(
        !methods.iter().any(|m| m == "episodic.set_segment_summary"),
        "an entryless segment must not be summarised; got {methods:?}"
    );
}

/// The predicate both consumers gate on. The emptiness cases cannot be produced
/// by `summarize_entries` today; they are pinned so a future arm that could
/// produce one still lands on "not usable".
#[test]
fn recap_is_usable_truth_table() {
    assert!(recap_is_usable(true, "a real recap"));
    assert!(!recap_is_usable(false, "a heuristic bookend"));
    assert!(!recap_is_usable(true, ""));
    assert!(!recap_is_usable(true, " \n\t"));
}
