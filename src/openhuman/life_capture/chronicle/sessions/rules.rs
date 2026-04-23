//! Pure boundary-detection rules for session bucketing (A4).
//!
//! A session ends when one of three conditions holds between the current
//! open session state and the newly-arrived event:
//!
//! * **idle_5m** — gap between the new event's `ts_ms` and the previous
//!   event's `ts_ms` is ≥ `IDLE_GAP_MS` (5 minutes). Short gaps under the
//!   threshold are treated as quiet-but-active and kept inside the session.
//! * **app_switch_3m** — the focused app has differed from the session's
//!   primary app for a sustained stretch of ≥ `APP_SWITCH_MS` (3 minutes).
//!   A brief focus flick (<3m) to a secondary app is treated as
//!   interruption noise — the manager keeps the session open and does NOT
//!   rewrite primary_app, so the next check compares the incoming app to
//!   the original primary, not the flick.
//! * **max_2h** — the session's total span (current event ts_ms minus
//!   start_ts_ms) would exceed `MAX_SESSION_MS` (2 hours). Forces a split
//!   even under continuous activity.
//!
//! All times are unix milliseconds. Rules are pure so the manager can be
//! tested with synthetic event streams without touching SQLite.

/// 5 minutes of silence before we declare the session idle-ended.
pub const IDLE_GAP_MS: i64 = 5 * 60 * 1_000;
/// Sustained time on a non-primary app before we declare an app-switch
/// boundary. Shorter flicks are absorbed as noise.
pub const APP_SWITCH_MS: i64 = 3 * 60 * 1_000;
/// Hard upper bound on a single session's duration, regardless of activity.
pub const MAX_SESSION_MS: i64 = 2 * 60 * 60 * 1_000;

/// Why a session was closed. Written verbatim into `chronicle_sessions.boundary_reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryReason {
    /// ≥ 5 minute gap since the last event.
    Idle,
    /// ≥ 3 minute sustained focus on a non-primary app.
    AppSwitch,
    /// Session has run for ≥ 2 hours; forced split.
    MaxDuration,
}

impl BoundaryReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            BoundaryReason::Idle => "idle_5m",
            BoundaryReason::AppSwitch => "app_switch_3m",
            BoundaryReason::MaxDuration => "max_2h",
        }
    }
}

/// Minimal fields the boundary check needs from an incoming event. Mirrors
/// `StoredEvent` but we don't want this module to depend on the SQLite row
/// struct — the manager builds this view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventView {
    pub ts_ms: i64,
    pub focused_app: String,
}

/// Open-session state tracked across events. The manager owns one of
/// these per process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSession {
    /// ts_ms of the session's first event.
    pub start_ts_ms: i64,
    /// ts_ms of the most recently observed event in this session.
    pub last_ts_ms: i64,
    /// Dominant app (first event's app, then recomputed only at close time).
    pub primary_app: String,
    /// ts_ms when a non-primary focus stretch started, or None if the
    /// current app is the primary. Reset to None whenever we re-focus
    /// the primary app.
    pub non_primary_since_ms: Option<i64>,
}

impl OpenSession {
    pub fn start(event: &EventView) -> Self {
        Self {
            start_ts_ms: event.ts_ms,
            last_ts_ms: event.ts_ms,
            primary_app: event.focused_app.clone(),
            non_primary_since_ms: None,
        }
    }
}

/// Decide whether `event` should close `session`. Call BEFORE folding the
/// event into the open session — the manager closes the existing session
/// first, then starts a new one from this event.
///
/// Returns `None` if the event belongs to the existing session. Returns
/// `Some(reason)` otherwise, where `reason` names the boundary that fired.
pub fn classify(session: &OpenSession, event: &EventView) -> Option<BoundaryReason> {
    // Hard cap first — a 2h session must split even if idle/app-switch
    // wouldn't have fired. Measured from start to this event.
    if event.ts_ms.saturating_sub(session.start_ts_ms) >= MAX_SESSION_MS {
        return Some(BoundaryReason::MaxDuration);
    }

    // Idle gap — measured from the last observed event to this one.
    if event.ts_ms.saturating_sub(session.last_ts_ms) >= IDLE_GAP_MS {
        return Some(BoundaryReason::Idle);
    }

    // Sustained app switch — only fires when the non-primary stretch has
    // been continuous for ≥ APP_SWITCH_MS. A flick back to primary resets
    // the stretch.
    if event.focused_app != session.primary_app {
        if let Some(since) = session.non_primary_since_ms {
            if event.ts_ms.saturating_sub(since) >= APP_SWITCH_MS {
                return Some(BoundaryReason::AppSwitch);
            }
        }
    }

    None
}

/// Fold `event` into `session`, updating `last_ts_ms` and tracking the
/// non-primary focus stretch. Caller must have already confirmed the event
/// does NOT break the session (i.e. `classify` returned `None`).
pub fn extend(session: &mut OpenSession, event: &EventView) {
    session.last_ts_ms = event.ts_ms;
    if event.focused_app == session.primary_app {
        // Re-focused the primary app — any pending non-primary stretch is
        // now definitively a flick, not a switch. Reset the timer.
        session.non_primary_since_ms = None;
    } else if session.non_primary_since_ms.is_none() {
        // First event on a non-primary app since last reset — start timing
        // the stretch.
        session.non_primary_since_ms = Some(event.ts_ms);
    }
    // If already mid-stretch on a non-primary app, leave
    // non_primary_since_ms untouched so duration keeps accumulating.
}

/// Minute bucket key: truncate ts_ms down to its minute boundary.
pub fn minute_bucket(ts_ms: i64) -> i64 {
    (ts_ms / 60_000) * 60_000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(app: &str, ts: i64) -> EventView {
        EventView {
            ts_ms: ts,
            focused_app: app.into(),
        }
    }

    #[test]
    fn no_boundary_when_events_are_tight_same_app() {
        let s = OpenSession::start(&ev("code", 0));
        assert_eq!(classify(&s, &ev("code", 60_000)), None);
    }

    #[test]
    fn idle_boundary_fires_at_exactly_five_minutes() {
        let s = OpenSession::start(&ev("code", 0));
        // Under threshold → still same session.
        assert_eq!(classify(&s, &ev("code", IDLE_GAP_MS - 1)), None);
        // At threshold → breaks.
        assert_eq!(
            classify(&s, &ev("code", IDLE_GAP_MS)),
            Some(BoundaryReason::Idle)
        );
    }

    #[test]
    fn app_flick_under_three_minutes_is_noise() {
        // Start on code, flick to slack for 2min, back to code.
        let mut s = OpenSession::start(&ev("code", 0));
        // 30s later: still code, no stretch.
        extend(&mut s, &ev("code", 30_000));
        // 1min later: flick to slack. Below threshold at this point.
        let slack_event = ev("slack", 60_000);
        assert_eq!(classify(&s, &slack_event), None);
        extend(&mut s, &slack_event);
        assert_eq!(s.non_primary_since_ms, Some(60_000));
        // 2m30s later on slack (90s stretch, still under 3m) — no break.
        let slack_event2 = ev("slack", 150_000);
        assert_eq!(classify(&s, &slack_event2), None);
        extend(&mut s, &slack_event2);
        // Back to code — stretch resets.
        let code_event = ev("code", 180_000);
        assert_eq!(classify(&s, &code_event), None);
        extend(&mut s, &code_event);
        assert_eq!(s.non_primary_since_ms, None);
    }

    #[test]
    fn app_switch_fires_when_stretch_hits_three_minutes() {
        let mut s = OpenSession::start(&ev("code", 0));
        // Move to slack at 60s.
        let slack_start = ev("slack", 60_000);
        assert_eq!(classify(&s, &slack_start), None);
        extend(&mut s, &slack_start);
        // Keep on slack for another 3 minutes — stretch = 3m at this point.
        let slack_cont = ev("slack", 60_000 + APP_SWITCH_MS);
        assert_eq!(
            classify(&s, &slack_cont),
            Some(BoundaryReason::AppSwitch),
            "3m continuous non-primary focus must fire AppSwitch"
        );
    }

    #[test]
    fn max_duration_splits_even_under_constant_activity() {
        let mut s = OpenSession::start(&ev("code", 0));
        // Feed a dense stream of code events up to just under 2h.
        let mut t = 60_000;
        while t < MAX_SESSION_MS {
            let e = ev("code", t);
            assert_eq!(classify(&s, &e), None);
            extend(&mut s, &e);
            t += 60_000;
        }
        // Exactly at 2h — forced split.
        assert_eq!(
            classify(&s, &ev("code", MAX_SESSION_MS)),
            Some(BoundaryReason::MaxDuration),
        );
    }

    #[test]
    fn max_duration_takes_priority_over_idle() {
        // If a single event arrives after 3h of silence, both idle and
        // max_duration trigger. MaxDuration wins (checked first) because a
        // forced-split session semantically predates the idle gap that
        // followed it.
        let s = OpenSession::start(&ev("code", 0));
        let reason = classify(&s, &ev("code", 3 * 60 * 60 * 1_000));
        assert_eq!(reason, Some(BoundaryReason::MaxDuration));
    }

    #[test]
    fn minute_bucket_floors_correctly() {
        assert_eq!(minute_bucket(0), 0);
        assert_eq!(minute_bucket(59_999), 0);
        assert_eq!(minute_bucket(60_000), 60_000);
        assert_eq!(minute_bucket(61_234), 60_000);
        assert_eq!(minute_bucket(119_999), 60_000);
        assert_eq!(minute_bucket(120_000), 120_000);
    }

    #[test]
    fn boundary_reason_strings_are_stable() {
        // Stored verbatim in SQL — must not change silently.
        assert_eq!(BoundaryReason::Idle.as_str(), "idle_5m");
        assert_eq!(BoundaryReason::AppSwitch.as_str(), "app_switch_3m");
        assert_eq!(BoundaryReason::MaxDuration.as_str(), "max_2h");
    }
}
