//! No-op stand-ins for the `agent_meetings` symbols that always-compiled code
//! reaches for, used when the `meet` feature is off (#4800).
//!
//! Only the three symbols with non-registration call sites outside the Meet
//! domain live here. Everything else in `agent_meetings` is reachable only via
//! the controller registry, which is itself `#[cfg(feature = "meet")]` at
//! `core::all` — so it needs no stub.
//!
//! These are genuine no-ops, not "register a controller that then errors": a
//! `register_*` that registers nothing is exactly the intended disabled-build
//! behaviour — the subscriber simply never exists, so the events never route.

/// Stub of [`super::calendar`](../calendar/index.html) for `--no-default-features` builds.
pub mod calendar {
    /// No-op stand-in for `calendar::register_meet_calendar_subscriber`.
    ///
    /// With `meet` compiled out there is no calendar-triggered meeting flow to
    /// drive, so registering nothing is the correct behaviour: the event bus
    /// keeps routing, it just has no Meet subscriber attached.
    pub fn register_meet_calendar_subscriber() {}

    /// No-op stand-in for `calendar::handle_calendar_meeting_candidate`.
    ///
    /// The `bool` means "Meet published its own actionable card, so the caller
    /// should skip its generic one". With `meet` compiled out no card was ever
    /// published, so `false` is semantically exact — the heartbeat planner
    /// correctly falls through to its plain "meeting starting" reminder.
    pub async fn handle_calendar_meeting_candidate(
        _meet_url: String,
        _event_title: String,
        _owner_display_name: Option<String>,
        _calendar_event_id: Option<String>,
    ) -> bool {
        false
    }
}

/// Stub of [`super::bus`](../bus/index.html) for `--no-default-features` builds.
pub mod bus {
    /// No-op stand-in for `bus::register_meeting_event_subscriber`.
    ///
    /// Same reasoning as `calendar::register_meet_calendar_subscriber`: the
    /// `Meeting*` / `BackendMeet*` domain events still exist on the bus (they
    /// are inert plain data), they simply have no subscriber and never fire.
    pub fn register_meeting_event_subscriber() {}
}
