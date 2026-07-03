//! Business logic for the `recall_calendar` domain.
//!
//! The core never talks to Recall.ai directly — every call proxies to the
//! openhuman backend's `/agent-integrations/recall-calendar/*` routes through
//! the shared [`IntegrationClient`], which attaches the app-session JWT and
//! unwraps the `{ success, data }` envelope (including 401 → session-expiry
//! handling). This mirrors the Composio domain's backend-proxied design.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::integrations::client::IntegrationClient;
use crate::rpc::RpcOutcome;

use super::types::{
    RecallCalendarConnect, RecallCalendarDisconnect, RecallCalendarStatus, RecallMeeting,
    RecallMeetingsResponse,
};

const CONNECT_PATH: &str = "/agent-integrations/recall-calendar/connect";
const STATUS_PATH: &str = "/agent-integrations/recall-calendar/status";
const DISCONNECT_PATH: &str = "/agent-integrations/recall-calendar/disconnect";
const MEETINGS_PATH: &str = "/agent-integrations/recall-calendar/meetings";

fn recall_client(config: &Config) -> Result<Arc<IntegrationClient>, String> {
    crate::openhuman::integrations::build_client(config)
        .ok_or_else(|| "[recall_calendar] backend client unavailable (no session)".to_string())
}

/// Start the Recall Calendar V1 OAuth flow; returns the Google consent URL.
pub async fn connect(config: &Config) -> Result<RpcOutcome<RecallCalendarConnect>, String> {
    tracing::debug!("[recall_calendar] rpc connect");
    let client = recall_client(config)?;
    let resp = client
        .post::<RecallCalendarConnect>(CONNECT_PATH, &json!({}))
        .await
        .map_err(|e| format!("[recall_calendar] connect failed: {e:#}"))?;
    // Connection state is about to change — drop the memoized probe so the
    // heartbeat/meetings fallback re-detects on its next tick instead of
    // serving a stale "not connected" for up to RECALL_DETECT_TTL.
    invalidate_detect_cache();
    Ok(RpcOutcome::new(
        resp,
        vec!["recall_calendar: connect flow started".to_string()],
    ))
}

/// Fetch the user's Recall calendar connection status.
pub async fn status(config: &Config) -> Result<RpcOutcome<RecallCalendarStatus>, String> {
    tracing::debug!("[recall_calendar] rpc status");
    let client = recall_client(config)?;
    let resp = client
        .get::<RecallCalendarStatus>(STATUS_PATH)
        .await
        .map_err(|e| format!("[recall_calendar] status failed: {e:#}"))?;
    Ok(RpcOutcome::new(resp, Vec::new()))
}

/// Return whether Recall Calendar is both enabled server-side and connected for
/// the current backend user. This is used by core meeting fetch paths that
/// should not depend on the settings UI being mounted long enough to sync the
/// local `meet.calendar_provider` flag.
pub async fn is_connected(config: &Config) -> Result<bool, String> {
    let outcome = status(config).await?;
    Ok(outcome.value.enabled && outcome.value.connected)
}

/// How long a Recall connectivity probe is trusted before it is re-checked.
/// Bounds the live `status` RPC on the hot path to at most once per window for
/// users who have not flipped `calendar_provider` to Recall — otherwise a
/// connected (or absent) Recall calendar is re-detected on *every* heartbeat
/// tick and *every* meetings-page fetch, a recurring external round-trip for
/// all logged-in non-Recall users (#4391 review). The settings UI still flips
/// the provider immediately on connect; this only rate-limits the
/// mount-independent fallback shared by both call sites.
const RECALL_DETECT_TTL: Duration = Duration::from_secs(30 * 60);

/// Process-wide memo of the last Recall probe: `(taken_at, connected)`.
static RECALL_DETECT_CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

/// True when a probe taken at `at` is still within `ttl` as of `now`. Pure so
/// the TTL boundary is unit-testable without controlling the wall clock.
fn recall_probe_fresh(at: Instant, now: Instant, ttl: Duration) -> bool {
    now.saturating_duration_since(at) < ttl
}

/// Detect whether a Recall calendar is connected, memoized for
/// [`RECALL_DETECT_TTL`] and shared by every mount-independent caller (heartbeat
/// planner + meetings-page fetch). A probe error (no backend session /
/// unavailable) is treated — and cached — as "not connected", so a Recall-less
/// user stops issuing a `status` RPC on every hot-path invocation.
pub async fn is_connected_cached(config: &Config) -> bool {
    let now = Instant::now();
    {
        let guard = RECALL_DETECT_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((at, connected)) = *guard {
            if recall_probe_fresh(at, now, RECALL_DETECT_TTL) {
                return connected;
            }
        }
    }
    let connected = match is_connected(config).await {
        Ok(connected) => connected,
        Err(error) => {
            tracing::debug!(
                error = %error,
                "[recall_calendar] status unavailable — treating as not connected"
            );
            false
        }
    };
    *RECALL_DETECT_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some((now, connected));
    connected
}

/// Drop the memoized connectivity probe. Called whenever the connection state
/// is (about to be) mutated — `connect` / `disconnect` — so the next
/// [`is_connected_cached`] call issues a fresh probe rather than serving a stale
/// value for the remainder of [`RECALL_DETECT_TTL`]. A transient error is still
/// cached as "not connected" (bounds probing for no-session users), but that
/// staleness is now cleared the moment the user actually connects/disconnects.
fn invalidate_detect_cache() {
    *RECALL_DETECT_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
}

/// Disconnect the user's Google calendar from Recall.
pub async fn disconnect(config: &Config) -> Result<RpcOutcome<RecallCalendarDisconnect>, String> {
    tracing::debug!("[recall_calendar] rpc disconnect");
    let client = recall_client(config)?;
    let resp = client
        .post::<RecallCalendarDisconnect>(DISCONNECT_PATH, &json!({}))
        .await
        .map_err(|e| format!("[recall_calendar] disconnect failed: {e:#}"))?;
    // Just disconnected — drop the memoized probe so the fallback stops routing
    // to Recall within one tick instead of after up to RECALL_DETECT_TTL.
    invalidate_detect_cache();
    Ok(RpcOutcome::new(
        resp,
        vec!["recall_calendar: calendar disconnected".to_string()],
    ))
}

/// Fetch upcoming meetings from the connected calendar (raw list).
pub async fn fetch_recall_meetings(config: &Config) -> Result<Vec<RecallMeeting>, String> {
    let client = recall_client(config)?;
    let resp = client
        .get::<RecallMeetingsResponse>(MEETINGS_PATH)
        .await
        .map_err(|e| format!("[recall_calendar] list meetings failed: {e:#}"))?;
    Ok(resp.meetings)
}

/// RPC wrapper around [`fetch_recall_meetings`].
pub async fn list_meetings(config: &Config) -> Result<RpcOutcome<RecallMeetingsResponse>, String> {
    tracing::debug!("[recall_calendar] rpc list_meetings");
    let meetings = fetch_recall_meetings(config).await?;
    let count = meetings.len();
    Ok(RpcOutcome::new(
        RecallMeetingsResponse { meetings },
        vec![format!("recall_calendar: {count} upcoming meeting(s)")],
    ))
}

/// Reshape backend-normalized Recall meetings into a Google-Calendar
/// `events.list`-style payload so the existing calendar extractors
/// (`agent_meetings::upcoming::extract_upcoming_meetings` and
/// `heartbeat::planner::collectors::extract_calendar_events`) parse them
/// unchanged. Only meetings with both a join URL and a start time survive.
pub fn meetings_to_gcal_json(meetings: &[RecallMeeting]) -> Value {
    let items: Vec<Value> = meetings
        .iter()
        .filter_map(|m| {
            let url = m
                .meeting_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            let start = m
                .start_time
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            let mut item = json!({
                "id": m.id,
                "summary": m.title.clone().unwrap_or_else(|| "Meeting".to_string()),
                "start": { "dateTime": start },
                "hangoutLink": url,
                "htmlLink": url,
            });
            if let Some(end) = m
                .end_time
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                item["end"] = json!({ "dateTime": end });
            }
            Some(item)
        })
        .collect();
    json!({ "items": items })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(id: &str, url: Option<&str>, start: Option<&str>) -> RecallMeeting {
        RecallMeeting {
            id: id.to_string(),
            title: Some("Standup".to_string()),
            meeting_url: url.map(ToString::to_string),
            start_time: start.map(ToString::to_string),
            end_time: Some("2026-07-01T10:30:00Z".to_string()),
            platform: Some("google_meet".to_string()),
            bot_id: None,
        }
    }

    #[test]
    fn recall_probe_freshness_respects_ttl() {
        let ttl = Duration::from_secs(600);
        let now = Instant::now();
        // A probe 60s old is inside a 600s TTL → reuse the cached result.
        let fresh_at = now.checked_sub(Duration::from_secs(60)).unwrap();
        assert!(recall_probe_fresh(fresh_at, now, ttl));
        // A probe 601s old is past the TTL → re-probe.
        let stale_at = now.checked_sub(Duration::from_secs(601)).unwrap();
        assert!(!recall_probe_fresh(stale_at, now, ttl));
    }

    #[test]
    fn invalidate_detect_cache_clears_memo() {
        // Prime the memo with a fresh "connected" result, then invalidate.
        *RECALL_DETECT_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), true));
        invalidate_detect_cache();
        assert!(
            RECALL_DETECT_CACHE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_none(),
            "connect/disconnect must drop the memoized probe so the next \
             is_connected_cached re-detects instead of serving a stale value"
        );
    }

    #[test]
    fn maps_fields_into_gcal_shape() {
        let m = meeting(
            "evt-1",
            Some("https://meet.google.com/abc"),
            Some("2026-07-01T10:00:00Z"),
        );
        let json = meetings_to_gcal_json(&[m]);
        let items = json["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it["id"], "evt-1");
        assert_eq!(it["summary"], "Standup");
        assert_eq!(it["start"]["dateTime"], "2026-07-01T10:00:00Z");
        assert_eq!(it["end"]["dateTime"], "2026-07-01T10:30:00Z");
        assert_eq!(it["hangoutLink"], "https://meet.google.com/abc");
    }

    #[test]
    fn drops_meetings_without_url_or_start() {
        let items = meetings_to_gcal_json(&[
            meeting("no-url", None, Some("2026-07-01T10:00:00Z")),
            meeting("no-start", Some("https://meet.google.com/x"), None),
            meeting("blank-url", Some("   "), Some("2026-07-01T10:00:00Z")),
        ]);
        assert_eq!(items["items"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn defaults_missing_title() {
        let mut m = meeting(
            "evt-2",
            Some("https://meet.google.com/y"),
            Some("2026-07-01T10:00:00Z"),
        );
        m.title = None;
        let json = meetings_to_gcal_json(&[m]);
        assert_eq!(json["items"][0]["summary"], "Meeting");
    }
}
