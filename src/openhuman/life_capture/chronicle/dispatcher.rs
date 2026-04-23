//! Stage 0 — dedup + debounce dispatcher.
//!
//! Incoming raw focus events from the accessibility layer are noisy: the
//! same focused element can emit many redundant notifications while the
//! user dwells on it, and rapid re-focus events can arrive a few ms apart.
//! S0 filters apply **only when the immediately preceding stored event had
//! the same `(focused_app, focused_element)` key** — i.e. the user is still
//! "on" that element. A cross-key transition (focus A → focus B) is always
//! recorded so that returning to A after visiting B is never silently
//! dropped as a duplicate of the earlier A.
//!
//! Within the same key, S0 drops events that are either
//!
//! * **duplicates** — identical `(visible_text, url)` to the last stored
//!   event for that key, or
//! * **debounced** — less than 200ms after the last stored event for that
//!   key.
//!
//! What passes is handed to `parser::parse` (S1) and written to
//! `chronicle_events` via `tables::insert_event`.
//!
//! Concurrency: the internal mutex is held across the DB insert so two
//! tasks calling `on_focus_event` in parallel cannot both observe stale
//! state, pass the filters, and double-insert. At UI focus-event rates
//! this serialisation is negligible.
//!
//! State is per-`DispatchState` rather than global so the unit tests can
//! run independently and so multiple dispatchers (e.g. a test harness
//! alongside a live loop) don't cross-contaminate.

use tokio::sync::Mutex;

use crate::openhuman::life_capture::chronicle::parser::{self, RawFocusEvent};
use crate::openhuman::life_capture::chronicle::tables;
use crate::openhuman::life_capture::index::PersonalIndex;

/// Minimum gap between stored events for the same (app, element) pair.
/// Tuned for the accessibility focus-change rate — tighter than this
/// admits near-duplicates from UI transients; looser drops real
/// user-intent switches that happen on sub-second cadence.
pub const DEBOUNCE_MS: i64 = 200;

/// In-memory "last stored event" — only the single most recent one matters
/// because dedup/debounce only fire when the incoming event has the same
/// key as the immediately preceding stored event.
#[derive(Debug, Default)]
pub struct DispatchState {
    last: Mutex<Option<LastEvent>>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct DedupKey {
    focused_app: String,
    focused_element: Option<String>,
}

#[derive(Debug, Clone)]
struct LastEvent {
    key: DedupKey,
    ts_ms: i64,
    visible_text: Option<String>,
    url: Option<String>,
}

/// Outcome of a dispatch attempt. Surfaced for tests and diagnostic logs.
#[derive(Debug, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// Passed both filters, parsed, stored. Inner is the stored row id.
    Stored(i64),
    /// Dropped: immediately preceded by a field-identical event on the same
    /// (app, element) key.
    Dedup,
    /// Dropped: less than `DEBOUNCE_MS` after the previous event on the
    /// same (app, element) key.
    Debounced,
}

impl DispatchState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run an event through S0 and (on pass) S1 + storage.
    pub async fn on_focus_event(
        &self,
        idx: &PersonalIndex,
        raw: RawFocusEvent,
    ) -> anyhow::Result<DispatchOutcome> {
        let key = DedupKey {
            focused_app: raw.focused_app.clone(),
            focused_element: raw.focused_element.clone(),
        };

        // Held across the insert to serialise concurrent dispatches. This
        // prevents two tasks from both observing stale state, both passing
        // the filters, and both inserting the "same" event.
        let mut guard = self.last.lock().await;
        if let Some(prev) = guard.as_ref() {
            if prev.key == key {
                if raw.ts_ms - prev.ts_ms < DEBOUNCE_MS && raw.ts_ms >= prev.ts_ms {
                    return Ok(DispatchOutcome::Debounced);
                }
                if prev.visible_text == raw.visible_text && prev.url == raw.url {
                    return Ok(DispatchOutcome::Dedup);
                }
            }
            // prev.key != key → focus actually moved; always record.
        }

        let snapshot = LastEvent {
            key,
            ts_ms: raw.ts_ms,
            visible_text: raw.visible_text.clone(),
            url: raw.url.clone(),
        };
        let event = parser::parse(raw);
        let row_id = tables::insert_event(idx, event).await?;
        *guard = Some(snapshot);
        Ok(DispatchOutcome::Stored(row_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::life_capture::chronicle::tables::list_recent;

    fn raw(app: &str, element: Option<&str>, text: Option<&str>, ts_ms: i64) -> RawFocusEvent {
        RawFocusEvent {
            focused_app: app.into(),
            focused_element: element.map(str::to_string),
            visible_text: text.map(str::to_string),
            url: None,
            ts_ms,
        }
    }

    #[tokio::test]
    async fn identical_consecutive_events_collapse() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let state = DispatchState::new();

        let a = raw("app", Some("AXTextField"), Some("hello"), 1_000);
        let b = raw("app", Some("AXTextField"), Some("hello"), 2_000);

        let o1 = state.on_focus_event(&idx, a).await.unwrap();
        let o2 = state.on_focus_event(&idx, b).await.unwrap();

        assert!(matches!(o1, DispatchOutcome::Stored(_)));
        assert_eq!(o2, DispatchOutcome::Dedup);

        let rows = list_recent(&idx, 10).await.unwrap();
        assert_eq!(rows.len(), 1, "only the first event should be stored");
    }

    #[tokio::test]
    async fn sub_debounce_events_collapse_even_if_content_differs() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let state = DispatchState::new();

        // Same (app, element); gap < 200ms; different visible_text.
        let a = raw("app", Some("AXTextField"), Some("hello"), 1_000);
        let b = raw("app", Some("AXTextField"), Some("world"), 1_050);

        let o1 = state.on_focus_event(&idx, a).await.unwrap();
        let o2 = state.on_focus_event(&idx, b).await.unwrap();

        assert!(matches!(o1, DispatchOutcome::Stored(_)));
        assert_eq!(o2, DispatchOutcome::Debounced);
    }

    #[tokio::test]
    async fn debounce_applies_per_app_element_pair() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let state = DispatchState::new();

        // Interleaved same-timestamp events on two different pairs both
        // get stored — debounce is per-key.
        let a1 = raw("appA", Some("e"), Some("x"), 1_000);
        let b1 = raw("appB", Some("e"), Some("y"), 1_050);

        let o1 = state.on_focus_event(&idx, a1).await.unwrap();
        let o2 = state.on_focus_event(&idx, b1).await.unwrap();

        assert!(matches!(o1, DispatchOutcome::Stored(_)));
        assert!(matches!(o2, DispatchOutcome::Stored(_)));
    }

    #[tokio::test]
    async fn after_debounce_window_content_change_is_stored() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let state = DispatchState::new();

        let a = raw("app", Some("e"), Some("hello"), 1_000);
        let b = raw("app", Some("e"), Some("hello world"), 1_000 + DEBOUNCE_MS);

        let o1 = state.on_focus_event(&idx, a).await.unwrap();
        let o2 = state.on_focus_event(&idx, b).await.unwrap();

        assert!(matches!(o1, DispatchOutcome::Stored(_)));
        assert!(
            matches!(o2, DispatchOutcome::Stored(_)),
            "post-debounce content change should be stored, got {o2:?}"
        );
    }

    #[tokio::test]
    async fn focus_return_after_detour_is_always_stored() {
        // A → B → A with identical content on each A. The second A must be
        // stored (a real chronological transition) even though its content
        // matches the earlier A. Otherwise the timeline shows the user
        // "stuck" on B and the return to A is silently lost.
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let state = DispatchState::new();

        let a1 = raw("appA", Some("e"), Some("hello"), 1_000);
        let b1 = raw("appB", Some("e"), Some("hello"), 1_500);
        let a2 = raw("appA", Some("e"), Some("hello"), 2_000);

        let o1 = state.on_focus_event(&idx, a1).await.unwrap();
        let o2 = state.on_focus_event(&idx, b1).await.unwrap();
        let o3 = state.on_focus_event(&idx, a2).await.unwrap();

        assert!(matches!(o1, DispatchOutcome::Stored(_)));
        assert!(matches!(o2, DispatchOutcome::Stored(_)));
        assert!(
            matches!(o3, DispatchOutcome::Stored(_)),
            "return to appA must not be dedup'd against the earlier appA, got {o3:?}"
        );

        let rows = list_recent(&idx, 10).await.unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn concurrent_same_key_events_cannot_double_insert() {
        // Two tasks fire the same (app, element, content) event in parallel.
        // The mutex must serialise them: one stores, the other sees the
        // just-stored snapshot and dedup's. Without the lock held across
        // the DB insert, both could pass the read-phase filters.
        use std::sync::Arc;

        let idx = Arc::new(PersonalIndex::open_in_memory().await.unwrap());
        let state = Arc::new(DispatchState::new());

        let r1 = raw("app", Some("e"), Some("x"), 1_000);
        let r2 = raw("app", Some("e"), Some("x"), 1_000);

        let (idx1, idx2) = (idx.clone(), idx.clone());
        let (s1, s2) = (state.clone(), state.clone());

        let h1 = tokio::spawn(async move { s1.on_focus_event(&idx1, r1).await.unwrap() });
        let h2 = tokio::spawn(async move { s2.on_focus_event(&idx2, r2).await.unwrap() });
        let (o1, o2) = tokio::join!(h1, h2);
        let outcomes = [o1.unwrap(), o2.unwrap()];

        let stored = outcomes
            .iter()
            .filter(|o| matches!(o, DispatchOutcome::Stored(_)))
            .count();
        let dropped = outcomes
            .iter()
            .filter(|o| !matches!(o, DispatchOutcome::Stored(_)))
            .count();
        assert_eq!(stored, 1, "exactly one of the concurrent calls must store");
        assert_eq!(dropped, 1, "the other must be filtered (dedup or debounce)");

        let rows = list_recent(&idx, 10).await.unwrap();
        assert_eq!(rows.len(), 1, "no double-insert under concurrent dispatch");
    }

    #[tokio::test]
    async fn pipeline_n_raw_events_yields_expected_stored_count() {
        // Integration test: mix of duplicates, debounces, and real
        // transitions. 8 raw → 4 stored (first event of each distinct
        // app/element/content triple, separated by >= 200ms).
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let state = DispatchState::new();

        let events = vec![
            raw("app", Some("e1"), Some("a"), 0),     // store
            raw("app", Some("e1"), Some("a"), 100),   // debounce (<200ms)
            raw("app", Some("e1"), Some("a"), 300),   // dedup (same content)
            raw("app", Some("e1"), Some("b"), 600),   // store (new content, >200ms)
            raw("app", Some("e2"), Some("x"), 600),   // store (different element)
            raw("app", Some("e2"), Some("x"), 700),   // debounce (<200ms)
            raw("other", Some("e1"), Some("a"), 650), // store (different app)
            raw("other", Some("e1"), Some("a"), 900), // dedup
        ];

        let mut stored = 0usize;
        for ev in events {
            if let DispatchOutcome::Stored(_) = state.on_focus_event(&idx, ev).await.unwrap() {
                stored += 1;
            }
        }
        assert_eq!(stored, 4);
        let rows = list_recent(&idx, 100).await.unwrap();
        assert_eq!(rows.len(), 4);
    }
}
