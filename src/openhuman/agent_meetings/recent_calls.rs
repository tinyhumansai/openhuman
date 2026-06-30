//! Recent-calls recording for the **backend meet** flow.
//!
//! The "Send OpenHuman to a meeting" card drives the backend bot via
//! `agent_meetings_join` → `bot:join`, and the call ends with a
//! `BackendMeetTranscript` event carrying only `turns` / `duration_ms` /
//! `correlation_id`. None of the join context (who launched the bot, which
//! URL, the bot's display name) survives to call-end on its own.
//!
//! To give the Recent-calls panel real detail we:
//!   1. snapshot the join inputs in an in-memory registry keyed by
//!      `correlation_id` at [`remember_join`] time, and
//!   2. at transcript time [`record_backend_call`] looks that snapshot back
//!      up, mines the participant names out of the transcript, and appends a
//!      [`MeetCallRecord`] to the shared recent-calls store the UI reads.
//!
//! The registry is intentionally in-memory: a call lives for minutes and the
//! snapshot is only needed for that window. If the app restarts mid-call we
//! simply fall back to a leaner record (duration + participants, no owner/URL)
//! rather than failing — recording is best-effort and never blocks the call.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::core::event_bus::BackendMeetTurn;
use crate::openhuman::meet_agent::store::{
    self, MeetCallActionItem, MeetCallDetail, MeetCallRecord, MeetCallSummary,
    MeetCallTranscriptLine,
};

use super::summary::GeneratedSummary;
use super::types::ActionItemKind;

const LOG_PREFIX: &str = "[agent_meetings::recent_calls]";

/// Join-time context snapshotted so the call record can be enriched at
/// transcript time. Keyed by `correlation_id` in the pending registry.
#[derive(Debug, Clone)]
pub struct JoinMeta {
    pub meet_url: String,
    /// Display name the user gave themselves in the call ("who added the bot").
    pub owner_display_name: String,
    /// The bot's tile name in the meeting.
    pub bot_display_name: String,
    pub started_at_ms: u64,
}

/// Evict a pending join this long after its `started_at_ms` if no transcript
/// ever claimed it (failed join, crash, bot never admitted, …). Generous —
/// a real call's transcript lands within minutes — but bounded so abandoned
/// joins can't accumulate in a long-lived process.
const PENDING_JOIN_TTL_MS: u64 = 6 * 60 * 60 * 1000; // 6h

/// Backstop hard cap on retained pending joins, independent of the TTL, so
/// pathological churn within the window still can't grow without bound.
/// Oldest entries are evicted first.
const MAX_PENDING_JOINS: usize = 256;

fn registry() -> &'static Mutex<HashMap<String, JoinMeta>> {
    static REG: OnceLock<Mutex<HashMap<String, JoinMeta>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Snapshot join context for `correlation_id`. No-op when the id is absent
/// (we can't key the later lookup without it). Opportunistically prunes stale
/// entries on each insert so a transcript that never arrives can't leak.
pub fn remember_join(correlation_id: Option<&str>, meta: JoinMeta) {
    let Some(cid) = correlation_id.map(str::trim).filter(|c| !c.is_empty()) else {
        return;
    };
    let mut map = registry().lock().unwrap();
    map.insert(cid.to_string(), meta);
    prune_stale(&mut map, now_ms());
    log::debug!(
        "{LOG_PREFIX} remembered join correlation_id={cid} pending={}",
        map.len()
    );
}

/// Drop entries past their TTL, then enforce `MAX_PENDING_JOINS` by evicting
/// the oldest. Pure over its inputs (takes `now_ms`) so it is unit-testable.
fn prune_stale(map: &mut HashMap<String, JoinMeta>, now_ms: u64) {
    map.retain(|_, m| now_ms.saturating_sub(m.started_at_ms) <= PENDING_JOIN_TTL_MS);
    if map.len() > MAX_PENDING_JOINS {
        let mut by_age: Vec<(String, u64)> = map
            .iter()
            .map(|(k, m)| (k.clone(), m.started_at_ms))
            .collect();
        by_age.sort_by_key(|(_, ts)| *ts);
        for (k, _) in by_age.into_iter().take(map.len() - MAX_PENDING_JOINS) {
            map.remove(&k);
        }
    }
}

/// Take (and remove) the snapshot for `correlation_id`, if any.
fn take_join(correlation_id: Option<&str>) -> Option<JoinMeta> {
    let cid = correlation_id.map(str::trim).filter(|c| !c.is_empty())?;
    registry().lock().unwrap().remove(cid)
}

/// Build and persist a [`MeetCallRecord`] for a finished backend-meet call.
/// Returns the `request_id` the record was keyed by so the caller can persist
/// the matching call detail under the same id.
///
/// Best-effort: any failure is logged and swallowed — the call is already
/// over and the UI degrades to "no record" rather than erroring. The id is
/// returned even when the append fails, since it is deterministic and the
/// detail write should still use it.
pub async fn record_backend_call(
    turns: &[BackendMeetTurn],
    duration_ms: u64,
    correlation_id: Option<&str>,
) -> String {
    let meta = take_join(correlation_id);
    let record = build_record(meta.as_ref(), turns, duration_ms, correlation_id, now_ms());
    let request_id = record.request_id.clone();
    match store::append_record(&record).await {
        Ok(()) => log::info!(
            "{LOG_PREFIX} recorded call request_id={} participants={} duration_s={:.0}",
            record.request_id,
            record.participants.len(),
            record.listened_seconds
        ),
        Err(e) => log::warn!("{LOG_PREFIX} append_record failed: {e}"),
    }
    request_id
}

/// Persist the per-call detail (transcript + optional summary) under the same
/// `request_id` the lean record used, so the recent-calls panel can lazy-load
/// it when a row is expanded. Best-effort: logs and swallows failures.
pub async fn record_backend_call_detail(
    request_id: &str,
    turns: &[BackendMeetTurn],
    generated: Option<&GeneratedSummary>,
) {
    let detail = build_detail(request_id, turns, generated);
    match store::write_detail(&detail).await {
        Ok(()) => log::info!(
            "{LOG_PREFIX} recorded call detail request_id={} transcript_lines={} has_summary={}",
            request_id,
            detail.transcript.len(),
            detail.summary.is_some()
        ),
        Err(e) => log::warn!("{LOG_PREFIX} write_detail failed request_id={request_id}: {e}"),
    }
}

/// Map a finished call's turns + optional summary into a persisted
/// [`MeetCallDetail`]. Pure (no I/O) so the field-mapping is unit-testable.
/// Blank turns are dropped so the stored transcript matches what the summary
/// was generated from.
fn build_detail(
    request_id: &str,
    turns: &[BackendMeetTurn],
    generated: Option<&GeneratedSummary>,
) -> MeetCallDetail {
    let transcript = turns
        .iter()
        .filter(|t| !t.content.trim().is_empty())
        .map(|t| MeetCallTranscriptLine {
            role: if t.role.eq_ignore_ascii_case("assistant") {
                "assistant".to_string()
            } else {
                "participant".to_string()
            },
            content: t.content.trim().to_string(),
        })
        .collect();
    let summary = generated.map(|g| MeetCallSummary {
        headline: g.summary.headline.clone(),
        key_points: g.summary.key_points.clone(),
        action_items: g
            .summary
            .action_items
            .iter()
            .map(|a| MeetCallActionItem {
                description: a.description.clone(),
                kind: match a.kind {
                    ActionItemKind::Executable => "executable".to_string(),
                    ActionItemKind::Advisory => "advisory".to_string(),
                },
                tool_name: a.tool_name.clone(),
                assignee: a.assignee.clone(),
            })
            .collect(),
    });
    MeetCallDetail {
        request_id: request_id.to_string(),
        summary,
        transcript,
    }
}

/// Map a finished call's inputs to a [`MeetCallRecord`]. Pure (takes `now_ms`,
/// no I/O) so the field-mapping and fallback logic is unit-testable without a
/// store or a workspace.
fn build_record(
    meta: Option<&JoinMeta>,
    turns: &[BackendMeetTurn],
    duration_ms: u64,
    correlation_id: Option<&str>,
    now_ms: u64,
) -> MeetCallRecord {
    // Prefer the snapshotted start; otherwise derive it from the duration so
    // the row still sorts roughly correctly in the newest-first list.
    let started_at_ms = meta
        .map(|m| m.started_at_ms)
        .unwrap_or_else(|| now_ms.saturating_sub(duration_ms));
    MeetCallRecord {
        request_id: correlation_id
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("backend-{started_at_ms}")),
        meet_url: meta.map(|m| m.meet_url.clone()).unwrap_or_default(),
        bot_display_name: meta.map(|m| m.bot_display_name.clone()).unwrap_or_default(),
        owner_display_name: meta
            .map(|m| m.owner_display_name.clone())
            .unwrap_or_default(),
        started_at_ms,
        ended_at_ms: now_ms,
        // The backend flow reports a single wall-clock duration rather than
        // split listen/speak meters; surface it as "listened" so the existing
        // UI ("Ns on call" = listened + spoken) shows the real call length.
        listened_seconds: (duration_ms as f32) / 1000.0,
        spoken_seconds: 0.0,
        turn_count: turns.len() as u32,
        participants: extract_participants(turns),
    }
}

/// Extract distinct human participant names from a transcript.
///
/// Backend transcript lines look like `[00:51] [Shanu Goyanka] your time` or
/// `[00:00] [System] Tiny joined the meeting`. The speaker is the first
/// bracketed token that is **not** a `MM:SS` timestamp. We skip:
///   - assistant turns (that's the bot, not a participant),
///   - the synthetic `System` speaker (join/leave/presence noise),
///   - blank/duplicate names (first-seen order preserved).
fn extract_participants(turns: &[BackendMeetTurn]) -> Vec<String> {
    let mut seen = Vec::new();
    for turn in turns {
        if turn.role.eq_ignore_ascii_case("assistant") {
            continue;
        }
        let Some(name) = speaker_name(&turn.content) else {
            continue;
        };
        if name.eq_ignore_ascii_case("system") {
            continue;
        }
        if !seen.iter().any(|n: &String| n == &name) {
            seen.push(name);
        }
    }
    seen
}

/// Pull the speaker name from a single transcript line: the first
/// `[...]` group whose contents are not a clock timestamp.
fn speaker_name(content: &str) -> Option<String> {
    let mut rest = content.trim_start();
    while let Some(stripped) = rest.strip_prefix('[') {
        let close = stripped.find(']')?;
        let inner = stripped[..close].trim();
        rest = stripped[close + 1..].trim_start();
        if inner.is_empty() || is_timestamp(inner) {
            continue;
        }
        return Some(inner.to_string());
    }
    None
}

/// True for `M:SS` / `MM:SS` / `H:MM:SS` clock stamps.
fn is_timestamp(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return false;
    }
    parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str, content: &str) -> BackendMeetTurn {
        BackendMeetTurn {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn speaker_name_skips_timestamp_and_takes_name() {
        assert_eq!(
            speaker_name("[00:51] [Shanu Goyanka] your time").as_deref(),
            Some("Shanu Goyanka")
        );
        assert_eq!(
            speaker_name("[System] Tiny joined").as_deref(),
            Some("System")
        );
        // No bracketed speaker → none.
        assert_eq!(speaker_name("just text"), None);
        assert_eq!(speaker_name("[12:00]"), None);
    }

    #[test]
    fn is_timestamp_matches_clock_only() {
        assert!(is_timestamp("0:00"));
        assert!(is_timestamp("00:51"));
        assert!(is_timestamp("1:02:03"));
        assert!(!is_timestamp("Shanu"));
        assert!(!is_timestamp("12"));
        assert!(!is_timestamp("a:bb"));
    }

    #[test]
    fn extract_participants_dedups_and_excludes_system_and_bot() {
        let turns = vec![
            turn("user", "[00:00] [System] Tiny joined the meeting"),
            turn("user", "[00:51] [Shanu Goyanka] your time"),
            turn("assistant", "[00:55] [Tiny] On it."),
            turn("user", "[02:09] [Shanu Goyanka] hey hello"),
            turn("user", "[02:20] [Alex Rivera] sounds good"),
        ];
        let names = extract_participants(&turns);
        assert_eq!(names, vec!["Shanu Goyanka", "Alex Rivera"]);
    }

    #[test]
    fn extract_participants_empty_when_no_speakers() {
        let turns = vec![turn("user", "no brackets here"), turn("assistant", "hi")];
        assert!(extract_participants(&turns).is_empty());
    }

    fn meta(started_at_ms: u64) -> JoinMeta {
        JoinMeta {
            meet_url: "https://meet.google.com/abc-defg-hij".into(),
            owner_display_name: "Shanu".into(),
            bot_display_name: "Tiny".into(),
            started_at_ms,
        }
    }

    #[test]
    fn remember_then_take_round_trips_and_consumes() {
        let cid = "corr-rc-test-1";
        // Fresh timestamp so the opportunistic prune in remember_join keeps it.
        remember_join(Some(cid), meta(now_ms()));
        let got = take_join(Some(cid)).expect("present");
        assert_eq!(got.owner_display_name, "Shanu");
        // Consumed — a second take is empty.
        assert!(take_join(Some(cid)).is_none());
    }

    #[test]
    fn prune_stale_evicts_expired_keeps_fresh() {
        let now = 10 * PENDING_JOIN_TTL_MS;
        let mut map = HashMap::new();
        map.insert("fresh".to_string(), meta(now)); // age 0
        map.insert("old".to_string(), meta(now - PENDING_JOIN_TTL_MS - 1)); // just past TTL
        prune_stale(&mut map, now);
        assert!(map.contains_key("fresh"));
        assert!(!map.contains_key("old"));
    }

    #[test]
    fn prune_stale_enforces_size_cap_evicting_oldest() {
        let now = 10 * PENDING_JOIN_TTL_MS;
        let mut map = HashMap::new();
        // All within TTL, but more than the cap — oldest must be evicted.
        for i in 0..(MAX_PENDING_JOINS + 5) as u64 {
            map.insert(format!("c{i}"), meta(now - i)); // c0 newest … higher i older
        }
        prune_stale(&mut map, now);
        assert_eq!(map.len(), MAX_PENDING_JOINS);
        assert!(map.contains_key("c0")); // newest kept
        assert!(!map.contains_key(&format!("c{}", MAX_PENDING_JOINS + 4))); // oldest evicted
    }

    #[test]
    fn build_record_uses_join_meta_when_present() {
        let turns = vec![turn("user", "[00:10] [Shanu Goyanka] hi")];
        let rec = build_record(Some(&meta(1000)), &turns, 30_000, Some("corr-42"), 50_000);
        assert_eq!(rec.request_id, "corr-42");
        assert_eq!(rec.meet_url, "https://meet.google.com/abc-defg-hij");
        assert_eq!(rec.owner_display_name, "Shanu");
        assert_eq!(rec.bot_display_name, "Tiny");
        assert_eq!(rec.started_at_ms, 1000); // from meta, not derived
        assert_eq!(rec.ended_at_ms, 50_000);
        assert_eq!(rec.listened_seconds, 30.0);
        assert_eq!(rec.turn_count, 1);
        assert_eq!(rec.participants, vec!["Shanu Goyanka"]);
    }

    #[test]
    fn build_record_falls_back_without_meta() {
        let turns = vec![turn("user", "[00:00] [System] joined")];
        let rec = build_record(None, &turns, 8_000, None, 100_000);
        // No correlation id → synthesised request_id from the derived start.
        assert_eq!(rec.started_at_ms, 92_000); // now - duration
        assert_eq!(rec.request_id, "backend-92000");
        assert!(rec.meet_url.is_empty());
        assert!(rec.owner_display_name.is_empty());
        // Only a System turn → no human participants.
        assert!(rec.participants.is_empty());
    }

    #[test]
    fn remember_join_noop_without_correlation_id() {
        remember_join(
            None,
            JoinMeta {
                meet_url: "x".into(),
                owner_display_name: "y".into(),
                bot_display_name: "z".into(),
                started_at_ms: 0,
            },
        );
        assert!(take_join(None).is_none());
        assert!(take_join(Some("   ")).is_none());
    }

    #[test]
    fn build_detail_maps_transcript_and_summary() {
        use super::super::types::{ActionItem, MeetingSummary};

        let turns = vec![
            turn("user", "[00:51] [Shanu] your time"),
            turn("user", "   "), // blank → dropped
            turn("assistant", "[00:55] [Tiny] On it."),
        ];
        let generated = GeneratedSummary {
            label: "Q3 Roadmap".into(),
            summary: MeetingSummary {
                meeting_id: "corr-9".into(),
                headline: "Agreed to ship Friday.".into(),
                key_points: vec!["Ship Friday".into()],
                action_items: vec![ActionItem {
                    description: "Send release notes".into(),
                    kind: ActionItemKind::Executable,
                    tool_name: Some("gmail".into()),
                    assignee: Some("Sam".into()),
                }],
                generated_at_ms: 0,
            },
        };

        let detail = build_detail("corr-9", &turns, Some(&generated));
        assert_eq!(detail.request_id, "corr-9");
        // Blank turn dropped; roles normalised to participant/assistant.
        assert_eq!(detail.transcript.len(), 2);
        assert_eq!(detail.transcript[0].role, "participant");
        assert_eq!(detail.transcript[0].content, "[00:51] [Shanu] your time");
        assert_eq!(detail.transcript[1].role, "assistant");
        let summary = detail.summary.expect("summary present");
        assert_eq!(summary.headline, "Agreed to ship Friday.");
        assert_eq!(summary.key_points, vec!["Ship Friday".to_string()]);
        assert_eq!(summary.action_items.len(), 1);
        assert_eq!(summary.action_items[0].kind, "executable");
        assert_eq!(summary.action_items[0].tool_name.as_deref(), Some("gmail"));
    }

    #[test]
    fn build_detail_without_summary_keeps_transcript() {
        let turns = vec![turn("user", "[00:10] [Shanu] hi")];
        let detail = build_detail("corr-x", &turns, None);
        assert!(detail.summary.is_none());
        assert_eq!(detail.transcript.len(), 1);
    }
}
