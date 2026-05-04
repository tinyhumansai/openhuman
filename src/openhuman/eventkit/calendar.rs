//! Calendar read-only access via EventKit (macOS only).
//!
//! Exposes `list_events` which:
//!   1. Requests calendar access (one permission prompt, cached by the OS).
//!   2. Queries EKEventStore for events in the requested window.
//!   3. Deduplicates on `(ical_uid, calendar_id)` using the local SQLite cache.
//!   4. Returns the full set of `CalendarEvent`s found in this fetch.
//!
//! On non-macOS targets this module compiles to a stub that returns a
//! `NotSupported` error so Linux/Windows CI builds succeed.
//!
//! PERMISSIONS: the Tauri host's Info.plist must include
//!   `NSCalendarsUsageDescription` — this module owns no plist concerns.

// ── macOS implementation ─────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::Arc;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2::AnyThread as _;
    use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
    use objc2_foundation::{NSDate, NSError};
    use tokio::sync::oneshot;

    use crate::openhuman::eventkit::runtime::EventStoreHandle;

    use crate::openhuman::eventkit::store;
    use crate::openhuman::eventkit::types::CalendarEvent;

    /// Request Calendar read access from EventKit.
    ///
    /// Must be called from a blocking thread (`spawn_blocking`).
    fn request_calendar_access(event_store: &EKEventStore) -> Result<(), String> {
        unsafe {
            let status = EKEventStore::authorizationStatusForEntityType(EKEntityType::Event);
            match status {
                EKAuthorizationStatus::FullAccess => {
                    log::debug!("[eventkit] calendar access already authorized (FullAccess)");
                    return Ok(());
                }
                EKAuthorizationStatus::Denied | EKAuthorizationStatus::Restricted => {
                    return Err(
                        "calendar access denied — grant access in System Settings > Privacy > Calendars"
                            .into(),
                    );
                }
                _ => {
                    log::debug!("[eventkit] requesting calendar access (status={status:?})");
                }
            }

            let (tx, rx) = oneshot::channel::<Result<(), String>>();
            let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
            let tx_clone = Arc::clone(&tx);

            // Build the completion block.
            //
            // SAFETY (block lifetime): `RcBlock::as_ptr` returns a `*mut Block<F>`
            // that is valid for as long as the `RcBlock` is alive.  The `RcBlock`
            // is kept alive on the stack below until `blocking_recv()` returns,
            // which guarantees the callback has already fired — so the block is
            // always live for EventKit's entire retention window.
            //
            // Using `RcBlock::as_ptr` (a proper `*mut Block<F>`) rather than the
            // previous `&*block as *const _ as *mut _` cast eliminates the
            // undefined-behaviour that arose from casting a shared reference to a
            // mutable raw pointer.
            let block = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
                let mut slot = tx_clone.lock().unwrap();
                if let Some(sender) = slot.take() {
                    let result = if granted.as_bool() {
                        Ok(())
                    } else {
                        Err("calendar access not granted by user".into())
                    };
                    let _ = sender.send(result);
                }
            });

            event_store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&block).cast());

            // `block` is alive here — EventKit has retained it internally.
            // `blocking_recv` waits until the callback fires and releases its
            // own reference, at which point EventKit drops its retain too.
            let result = rx
                .blocking_recv()
                .map_err(|_| "calendar permission callback never fired".to_string())?;

            // Explicit drop after recv so the compiler does not move it earlier.
            drop(block);
            result
        }
    }

    /// Fetch calendar events from EventKit for the given time window.
    ///
    /// `start_ts` / `end_ts` are Unix timestamps (seconds, UTC).
    /// `limit` caps the number of events returned.
    /// `ek_store` is the process-global `EKEventStore` (see `runtime::get_event_store`).
    pub fn fetch_events(
        conn: &rusqlite::Connection,
        ek_store: &EventStoreHandle,
        start_ts: i64,
        end_ts: i64,
        limit: usize,
    ) -> Result<Vec<CalendarEvent>, String> {
        log::debug!("[eventkit] fetch_events entry: start={start_ts} end={end_ts} limit={limit}");

        unsafe {
            request_calendar_access(&ek_store.0)?;

            let start = NSDate::initWithTimeIntervalSince1970(NSDate::alloc(), start_ts as f64);
            let end = NSDate::initWithTimeIntervalSince1970(NSDate::alloc(), end_ts as f64);

            // Build a predicate over all calendars for events.
            let calendars = ek_store.0.calendarsForEntityType(EKEntityType::Event);
            let predicate = ek_store
                .0
                .predicateForEventsWithStartDate_endDate_calendars(&start, &end, Some(&calendars));

            let raw_events = ek_store.0.eventsMatchingPredicate(&predicate);
            log::debug!(
                "[eventkit] eventsMatchingPredicate returned {} events",
                raw_events.len()
            );

            let now_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            let mut out: Vec<CalendarEvent> = Vec::new();
            for ek_event in raw_events.iter() {
                if out.len() >= limit {
                    break;
                }

                // Prefer `calendarItemExternalIdentifier` (cross-device stable iCal UID).
                //
                // DEDUP CORRECTNESS: we intentionally skip events that do not yet have
                // an external identifier.  Before iCloud hydrates an event the store
                // returns only the local `calendarItemIdentifier`.  Using the local id
                // as a fallback would assign hash A on the first sync, then a different
                // hash B (from the external id) on the next, emitting the same event
                // twice.  Skipping once-and-logging is safer than silently duping.
                let ical_uid = match ek_event.calendarItemExternalIdentifier() {
                    Some(s) => s.to_string(),
                    None => {
                        log::debug!(
                            "[eventkit] skipping event without external identifier \
                             — iCloud may not have synced yet"
                        );
                        continue;
                    }
                };

                let calendar = ek_event.calendar();
                let calendar_id = calendar
                    .as_deref()
                    .map(|c| c.calendarIdentifier().to_string())
                    .unwrap_or_default();

                // Dedup: skip events already in the local cache.
                match store::is_known(conn, &ical_uid, &calendar_id) {
                    Ok(true) => {
                        log::trace!(
                            "[eventkit] skipping known event uid={ical_uid} cal={calendar_id}"
                        );
                        continue;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        return Err(format!("cache lookup failed for calendar event: {e}"));
                    }
                }

                let title = ek_event.title().to_string();
                let notes = ek_event.notes().map(|s| s.to_string());
                let start_date = ns_date_to_rfc3339(ek_event.startDate());
                let end_date = ns_date_to_rfc3339(ek_event.endDate());
                let is_all_day = ek_event.isAllDay();

                let organizer = ek_event
                    .organizer()
                    .and_then(|p| p.name())
                    .map(|s| s.to_string());

                let location = ek_event.location().map(|s| s.to_string());

                let calendar_title = calendar
                    .as_deref()
                    .map(|c| c.title().to_string())
                    .unwrap_or_default();

                let ev = CalendarEvent {
                    ical_uid,
                    calendar_id,
                    calendar_title,
                    title,
                    notes,
                    start_date,
                    end_date,
                    is_all_day,
                    organizer,
                    location,
                    fetched_at: now_ts,
                };

                // Write to local cache before appending; fail closed on error
                // to prevent duplicate events on the next sync.
                store::upsert_event(conn, &ev)
                    .map_err(|e| format!("cache upsert failed for calendar event: {e}"))?;
                out.push(ev);
            }

            log::debug!(
                "[eventkit] fetch_events exit: returning {} new events",
                out.len()
            );
            Ok(out)
        }
    }

    /// Convert an `NSDate` to an RFC 3339 UTC string.
    fn ns_date_to_rfc3339(date: objc2::rc::Retained<NSDate>) -> String {
        let secs = date.timeIntervalSince1970() as i64;
        chrono::DateTime::from_timestamp(secs, 0)
            .unwrap_or_default()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    }
}

// ── Public surface (macOS) ───────────────────────────────────────────────────

/// List calendar events for the given UTC Unix timestamp window.
///
/// Spawns onto a `tokio::task::spawn_blocking` thread because EventKit
/// must be called from a non-async context on macOS.
///
/// New events are dedup'd via `(ical_uid, calendar_id)` and cached locally.
/// Returns only the events that were *new* in this fetch (not already cached).
///
/// Events without a `calendarItemExternalIdentifier` (not yet iCloud-hydrated)
/// are skipped to avoid emitting duplicates on subsequent syncs.
#[cfg(target_os = "macos")]
pub async fn list_events(
    conn: std::sync::Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    ek_store: std::sync::Arc<
        tokio::sync::Mutex<crate::openhuman::eventkit::runtime::EventStoreHandle>,
    >,
    start_ts: i64,
    end_ts: i64,
    limit: usize,
) -> Result<Vec<crate::openhuman::eventkit::types::CalendarEvent>, String> {
    log::debug!("[eventkit] list_events async entry: start={start_ts} end={end_ts} limit={limit}");
    tokio::task::spawn_blocking(move || {
        let conn_guard = conn.blocking_lock();
        let store_guard = ek_store.blocking_lock();
        imp::fetch_events(&conn_guard, &store_guard, start_ts, end_ts, limit)
    })
    .await
    .map_err(|e| format!("[eventkit] spawn_blocking panicked: {e}"))?
}

// ── Stub (non-macOS) ─────────────────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
pub async fn list_events(
    _conn: std::sync::Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    _start_ts: i64,
    _end_ts: i64,
    _limit: usize,
) -> Result<Vec<crate::openhuman::eventkit::types::CalendarEvent>, String> {
    Err("eventkit::calendar is only supported on macOS".into())
}

// ── Non-macOS stub tests ─────────────────────────────────────────────────────

#[cfg(all(test, not(target_os = "macos")))]
mod stub_tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn list_events_returns_not_supported_on_non_macos() {
        let conn = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
        let err = list_events(conn, 0, 1, 10).await.unwrap_err();
        assert!(
            err.contains("only supported on macOS"),
            "unexpected error: {err}"
        );
    }
}

// ── Dedupe correctness test (platform-independent, uses store module) ────────

#[cfg(test)]
mod dedup_tests {
    use crate::openhuman::eventkit::store;
    use crate::openhuman::eventkit::types::CalendarEvent;

    fn fresh_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::run_migrations(&conn).unwrap();
        conn
    }

    fn make_event(uid: &str, cal: &str) -> CalendarEvent {
        CalendarEvent {
            ical_uid: uid.into(),
            calendar_id: cal.into(),
            calendar_title: "Work".into(),
            title: "Standup".into(),
            notes: None,
            start_date: "2026-04-22T09:00:00Z".into(),
            end_date: "2026-04-22T09:30:00Z".into(),
            is_all_day: false,
            organizer: None,
            location: None,
            fetched_at: 1_745_000_000,
        }
    }

    /// Simulates two back-to-back syncs that return the same `external_id`.
    /// The second sync must produce zero new events (no duplicate emission).
    #[test]
    fn no_dupe_on_resync_with_same_external_id() {
        let conn = fresh_conn();
        let external_id = "EXT-UID-001@icloud.com";
        let cal_id = "cal-primary";

        let ev = make_event(external_id, cal_id);

        // First sync: event is new, should be inserted.
        assert!(!store::is_known(&conn, external_id, cal_id).unwrap());
        store::upsert_event(&conn, &ev).unwrap();
        assert!(store::is_known(&conn, external_id, cal_id).unwrap());

        // Second sync: same external_id returned again — is_known must be true.
        assert!(
            store::is_known(&conn, external_id, cal_id).unwrap(),
            "re-sync must detect the already-cached event"
        );

        // Total cached events should still be 1.
        let cached = store::list_cached(&conn, 10).unwrap();
        assert_eq!(cached.len(), 1, "no duplicate should be stored");
    }

    /// Confirms that two events with different external ids in the same calendar
    /// are stored as separate entries (not confused by the dedup key).
    #[test]
    fn different_external_ids_stored_separately() {
        let conn = fresh_conn();
        let ev1 = make_event("EXT-A@icloud.com", "cal-1");
        let ev2 = make_event("EXT-B@icloud.com", "cal-1");
        store::upsert_event(&conn, &ev1).unwrap();
        store::upsert_event(&conn, &ev2).unwrap();
        let cached = store::list_cached(&conn, 10).unwrap();
        assert_eq!(cached.len(), 2);
    }
}
