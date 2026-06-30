//! Post-call meeting summarisation.
//!
//! After a Google Meet call ends, the raw transcript is turned into a
//! structured [`MeetingSummary`] (headline + key points + action items) **and**
//! a short context label (e.g. "Q3 Planning") via a single LLM call. The
//! summary is appended to the meeting thread; the label becomes the thread
//! *title* so each call is distinguishable from the others in the shared
//! "Meetings" group (it is not added as a second label, which would pollute the
//! shared label taxonomy with a unique, never-reused entry per call).
//!
//! Generation is best-effort: callers treat a failure as "no summary" and fall
//! back to the plain transcript thread rather than aborting thread creation.

use serde::Deserialize;

use crate::core::event_bus::BackendMeetTurn;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::create_chat_provider;

use super::types::{ActionItem, ActionItemKind, MeetingSummary};

const LOG_PREFIX: &str = "[agent_meetings::summary]";

/// Cap on the transcript size (in **bytes**, matching `str::len`) fed to the
/// model so a marathon call doesn't blow the context window. The tail is kept
/// (most recent) — summaries care most about conclusions and action items,
/// which land at the end. For multibyte (e.g. CJK) transcripts the effective
/// character budget is correspondingly smaller.
const MAX_TRANSCRIPT_BYTES: usize = 24_000;

/// Workload role used to resolve the summarisation provider/model from config.
const SUMMARIZATION_ROLE: &str = "summarization";

const SYSTEM_PROMPT: &str = "You summarise meeting transcripts. The transcript lines are \
prefixed with the speaker role (\"Participant\" or \"Assistant\"). Reply with ONLY a single \
JSON object — no prose, no markdown fences — with exactly these keys:\n\
- \"label\": a short topic label for the meeting, 2-4 words, Title Case (e.g. \"Q3 Roadmap\", \
\"Hiring Sync\"). This is used as the thread title, so make it specific to THIS meeting.\n\
- \"headline\": one sentence (<= 160 chars) capturing the meeting's outcome or purpose.\n\
- \"key_points\": an array of 2-6 short strings, the most important discussion points or decisions.\n\
- \"action_items\": an array of objects, each {\"description\": string, \"kind\": \"executable\" \
or \"advisory\", \"tool_name\": string-or-null, \"assignee\": string-or-null}. Use \"executable\" \
only when a connected tool could carry it out; otherwise \"advisory\". Use [] when there are none.\n\
Base every field strictly on the transcript. Do not invent attendees, decisions, or actions.";

/// LLM-shaped JSON the summariser is asked to return.
#[derive(Debug, Deserialize)]
struct RawSummary {
    #[serde(default)]
    label: String,
    #[serde(default)]
    headline: String,
    #[serde(default)]
    key_points: Vec<String>,
    #[serde(default)]
    action_items: Vec<RawActionItem>,
}

#[derive(Debug, Deserialize)]
struct RawActionItem {
    #[serde(default)]
    description: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    assignee: Option<String>,
}

impl From<RawActionItem> for ActionItem {
    fn from(raw: RawActionItem) -> Self {
        let kind = match raw
            .kind
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("executable") => ActionItemKind::Executable,
            _ => ActionItemKind::Advisory,
        };
        ActionItem {
            description: raw.description.trim().to_string(),
            kind,
            tool_name: raw.tool_name.and_then(non_empty),
            assignee: raw.assignee.and_then(non_empty),
        }
    }
}

/// A generated summary plus the short context label for the meeting thread.
#[derive(Debug)]
pub struct GeneratedSummary {
    /// Short topic label, e.g. "Q3 Roadmap". May be empty if the model
    /// returned nothing usable — callers should skip the label in that case.
    pub label: String,
    pub summary: MeetingSummary,
}

/// Generate a structured summary + context label from a finished call's turns.
///
/// `correlation_id` (when present) becomes the summary's `meeting_id`.
pub async fn generate_meeting_summary(
    turns: &[BackendMeetTurn],
    correlation_id: Option<&str>,
) -> Result<GeneratedSummary, String> {
    let transcript = render_transcript(turns);
    if transcript.trim().is_empty() {
        return Err("transcript had no usable turns".to_string());
    }

    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("config load failed: {e}"))?;
    let (provider, model) = create_chat_provider(SUMMARIZATION_ROLE, &config)
        .map_err(|e| format!("summarisation provider init failed: {e}"))?;

    let reply = provider
        .chat_with_system(Some(SYSTEM_PROMPT), &transcript, &model, 0.3)
        .await
        .map_err(|e| format!("summarisation LLM call failed: {e}"))?;

    let raw = parse_summary_json(&reply)
        .ok_or_else(|| "model reply did not contain parseable summary JSON".to_string())?;

    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let summary = MeetingSummary {
        meeting_id: correlation_id.unwrap_or("unknown").to_string(),
        headline: raw.headline.trim().to_string(),
        key_points: raw
            .key_points
            .into_iter()
            .filter_map(|p| non_empty(p.trim().to_string()))
            .collect(),
        action_items: raw
            .action_items
            .into_iter()
            .map(ActionItem::from)
            .filter(|a| !a.description.is_empty())
            .collect(),
        generated_at_ms: now_ms,
    };

    Ok(GeneratedSummary {
        label: sanitize_label(&raw.label),
        summary,
    })
}

/// Upper bound on the best-effort post-call summarisation call. The provider
/// has a 120s per-request timeout and the reliable wrapper retries transient
/// failures with backoff, so without a bound a slow/flaky `summarization`
/// provider could stall the call-end pipeline for minutes. 30s sits well above
/// a healthy summarisation latency while capping the worst case.
pub const SUMMARY_GENERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Generate a summary bounded by [`SUMMARY_GENERATION_TIMEOUT`], returning
/// `None` on failure or timeout. Centralises the best-effort policy so the
/// call-end pipeline can generate a single summary and share it across the
/// recent-call detail store and the meeting thread instead of paying for the
/// LLM call twice.
pub async fn generate_meeting_summary_bounded(
    turns: &[BackendMeetTurn],
    correlation_id: Option<&str>,
) -> Option<GeneratedSummary> {
    match tokio::time::timeout(
        SUMMARY_GENERATION_TIMEOUT,
        generate_meeting_summary(turns, correlation_id),
    )
    .await
    {
        Ok(Ok(g)) => Some(g),
        Ok(Err(e)) => {
            tracing::warn!("{LOG_PREFIX} summary generation failed: {e}");
            None
        }
        Err(_) => {
            tracing::warn!(
                timeout_secs = SUMMARY_GENERATION_TIMEOUT.as_secs(),
                "{LOG_PREFIX} summary generation timed out"
            );
            None
        }
    }
}

/// Render a [`MeetingSummary`] as a markdown body for the thread message.
pub fn format_summary_markdown(summary: &MeetingSummary, label: &str) -> String {
    let mut md = String::from("## Meeting summary\n\n");
    if !label.is_empty() {
        md.push_str(&format!("**Topic:** {label}\n\n"));
    }
    if !summary.headline.is_empty() {
        md.push_str(&format!("{}\n\n", summary.headline));
    }
    if !summary.key_points.is_empty() {
        md.push_str("### Key points\n\n");
        for point in &summary.key_points {
            md.push_str(&format!("- {point}\n"));
        }
        md.push('\n');
    }
    if !summary.action_items.is_empty() {
        md.push_str("### Action items\n\n");
        for item in &summary.action_items {
            let mut line = format!("- {}", item.description);
            if let Some(assignee) = &item.assignee {
                line.push_str(&format!(" _(owner: {assignee})_"));
            }
            if matches!(item.kind, ActionItemKind::Executable) {
                match &item.tool_name {
                    Some(tool) => line.push_str(&format!(" — actionable via `{tool}`")),
                    None => line.push_str(" — actionable"),
                }
            }
            md.push_str(&line);
            md.push('\n');
        }
        md.push('\n');
    }
    md
}

/// Flatten turns into `Role: text` lines, capped to the tail when very long.
fn render_transcript(turns: &[BackendMeetTurn]) -> String {
    let mut out = String::new();
    for turn in turns {
        let text = turn.content.trim();
        if text.is_empty() {
            continue;
        }
        let role = if turn.role.eq_ignore_ascii_case("assistant") {
            "Assistant"
        } else {
            "Participant"
        };
        out.push_str(role);
        out.push_str(": ");
        out.push_str(text);
        out.push('\n');
    }
    cap_tail(&out, MAX_TRANSCRIPT_BYTES)
}

/// Keep the last `max` bytes of `s`, trimmed forward to a char + line boundary.
fn cap_tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut idx = s.len() - max;
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    // Advance to the start of the next line so we don't cut mid-sentence.
    let idx = s[idx..].find('\n').map(|i| idx + i + 1).unwrap_or(idx);
    format!("…(earlier transcript truncated)…\n{}", &s[idx..])
}

/// Parse the model reply as `RawSummary`, tolerating fences / surrounding prose.
fn parse_summary_json(reply: &str) -> Option<RawSummary> {
    if let Ok(parsed) = serde_json::from_str::<RawSummary>(reply.trim()) {
        return Some(parsed);
    }
    let slice = extract_json_object(reply)?;
    serde_json::from_str::<RawSummary>(&slice).ok()
}

/// Return the last top-level `{ … }` object in `text`, ignoring braces inside
/// string literals. Handles models that wrap JSON in prose or ```json fences.
fn extract_json_object(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    let mut best: Option<(usize, usize)> = None;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        best = Some((s, i + 1));
                    }
                }
            }
            _ => {}
        }
    }
    best.map(|(s, e)| text[s..e].to_string())
}

/// Collapse the model's label to a single trimmed line, dequoted and length-capped.
fn sanitize_label(raw: &str) -> String {
    let line = raw
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let cleaned = line
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`'))
        .trim();
    cleaned.chars().take(40).collect()
}

fn non_empty(s: String) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
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
    fn render_transcript_labels_roles_and_skips_blanks() {
        let turns = vec![
            turn("user", "Let's ship Friday."),
            turn("assistant", "Noted."),
            turn("user", "   "),
        ];
        let rendered = render_transcript(&turns);
        assert!(rendered.contains("Participant: Let's ship Friday."));
        assert!(rendered.contains("Assistant: Noted."));
        // The blank turn produces no extra "Participant:" line.
        assert_eq!(rendered.matches("Participant:").count(), 1);
    }

    #[test]
    fn render_transcript_truncates_long_input_and_keeps_tail() {
        // Build a transcript well over the cap so the head is dropped.
        let filler: Vec<BackendMeetTurn> = (0..4_000)
            .map(|i| turn("user", &format!("padding line {i}")))
            .collect();
        let mut turns = filler;
        turns.push(turn("assistant", "FINAL DECISION: ship Friday."));
        let rendered = render_transcript(&turns);
        assert!(rendered.len() <= MAX_TRANSCRIPT_BYTES + 64);
        assert!(rendered.starts_with("…(earlier transcript truncated)…\n"));
        // The most recent turn (where decisions land) survives truncation.
        assert!(rendered.contains("FINAL DECISION: ship Friday."));
    }

    #[test]
    fn cap_tail_keeps_tail_on_char_boundary() {
        let s = "héllo\nwörld\nfoo\nbar\n";
        let capped = cap_tail(s, 8);
        assert!(capped.starts_with("…(earlier transcript truncated)…\n"));
        assert!(capped.ends_with("bar\n"));
    }

    #[test]
    fn cap_tail_returns_input_unchanged_when_within_cap() {
        let s = "short transcript\n";
        assert_eq!(cap_tail(s, 1_000), s);
    }

    #[tokio::test]
    async fn generate_meeting_summary_rejects_empty_transcript() {
        // All turns blank → nothing to summarise. This guards the early-return
        // path that runs *before* any config/provider work, so it stays a pure,
        // network-free unit test.
        let turns = vec![turn("user", "   "), turn("assistant", "\n")];
        let err = generate_meeting_summary(&turns, Some("m1"))
            .await
            .expect_err("blank transcript should error");
        assert!(err.contains("no usable turns"), "unexpected error: {err}");
    }

    /// Scripted provider that returns a fixed reply, so the full
    /// generate → parse → map path can be exercised without any network.
    struct ScriptedProvider {
        reply: String,
    }

    #[async_trait::async_trait]
    impl crate::openhuman::inference::provider::Provider for ScriptedProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(self.reply.clone())
        }
    }

    #[tokio::test]
    async fn generate_meeting_summary_parses_and_maps_provider_reply() {
        // Inject a scripted provider via the factory test override so
        // `create_chat_provider` hands back our mock instead of resolving a
        // real provider — the call stays network-free. The guard clears the
        // override on drop.
        let reply = "Here you go:\n```json\n{\"label\":\"Q3 Roadmap\",\
            \"headline\":\"Agreed to ship Friday.\",\
            \"key_points\":[\"Ship Friday\",\"QA owns sign-off\"],\
            \"action_items\":[{\"description\":\"Send release notes\",\
            \"kind\":\"executable\",\"tool_name\":\"gmail\",\"assignee\":\"Sam\"},\
            {\"description\":\"Book retro\",\"kind\":\"advisory\",\
            \"tool_name\":null,\"assignee\":null}]}\n```";
        let _guard =
            crate::openhuman::inference::provider::factory::test_provider_override::install(
                std::sync::Arc::new(ScriptedProvider {
                    reply: reply.to_string(),
                }),
            );

        let turns = vec![
            turn("user", "Let's ship Friday."),
            turn("assistant", "Noted, QA owns sign-off."),
        ];
        let generated = generate_meeting_summary(&turns, Some("meet-42"))
            .await
            .expect("scripted reply should parse");

        // Label is sanitised from the model reply.
        assert_eq!(generated.label, "Q3 Roadmap");
        // correlation_id flows through to the summary's meeting_id.
        assert_eq!(generated.summary.meeting_id, "meet-42");
        assert_eq!(generated.summary.headline, "Agreed to ship Friday.");
        assert_eq!(generated.summary.key_points.len(), 2);
        // Action items are mapped to executable/advisory with tool/assignee.
        assert_eq!(generated.summary.action_items.len(), 2);
        let exec = &generated.summary.action_items[0];
        assert!(matches!(exec.kind, ActionItemKind::Executable));
        assert_eq!(exec.tool_name.as_deref(), Some("gmail"));
        assert_eq!(exec.assignee.as_deref(), Some("Sam"));
        let advisory = &generated.summary.action_items[1];
        assert!(matches!(advisory.kind, ActionItemKind::Advisory));
        assert!(advisory.tool_name.is_none());
    }

    #[test]
    fn parse_summary_json_direct() {
        let reply = r#"{"label":"Q3 Roadmap","headline":"Agreed to ship Friday.","key_points":["Ship Friday"],"action_items":[]}"#;
        let parsed = parse_summary_json(reply).expect("parse");
        assert_eq!(parsed.label, "Q3 Roadmap");
        assert_eq!(parsed.key_points.len(), 1);
    }

    #[test]
    fn parse_summary_json_from_fenced_block_with_prose() {
        let reply = "Here is the summary:\n```json\n{\"label\":\"Hiring Sync\",\"headline\":\"Two offers approved.\",\"key_points\":[\"Approve offers\"],\"action_items\":[{\"description\":\"Send offer\",\"kind\":\"executable\",\"tool_name\":\"gmail\",\"assignee\":\"Sam\"}]}\n```\nDone.";
        let parsed = parse_summary_json(reply).expect("parse");
        assert_eq!(parsed.label, "Hiring Sync");
        assert_eq!(parsed.action_items.len(), 1);
        assert_eq!(parsed.action_items[0].kind.as_deref(), Some("executable"));
    }

    #[test]
    fn parse_summary_json_tolerates_braces_inside_strings() {
        // A key point containing literal braces must not confuse object
        // boundary detection in `extract_json_object`.
        let reply = "noise {not json} more\n{\"label\":\"Infra\",\"headline\":\"Migrate config { } blocks\",\"key_points\":[\"Use {env} vars\"],\"action_items\":[]} trailing";
        let parsed = parse_summary_json(reply).expect("parse");
        assert_eq!(parsed.label, "Infra");
        assert_eq!(parsed.key_points, vec!["Use {env} vars".to_string()]);
    }

    #[test]
    fn parse_summary_json_returns_none_for_garbage() {
        assert!(parse_summary_json("no json here at all").is_none());
        assert!(parse_summary_json("").is_none());
    }

    #[test]
    fn extract_json_object_picks_last_top_level_object() {
        // Prose may contain an example object before the real reply; the last
        // top-level object wins.
        let text = "example: {\"a\":1} answer: {\"b\":2}";
        assert_eq!(extract_json_object(text).as_deref(), Some("{\"b\":2}"));
        assert!(extract_json_object("no braces").is_none());
    }

    #[test]
    fn action_item_executable_preserves_tool_and_assignee() {
        let raw = RawActionItem {
            description: "Send invite".to_string(),
            kind: Some("  Executable ".to_string()),
            tool_name: Some("gmail".to_string()),
            assignee: Some(" Sam ".to_string()),
        };
        let item = ActionItem::from(raw);
        assert!(matches!(item.kind, ActionItemKind::Executable));
        assert_eq!(item.tool_name.as_deref(), Some("gmail"));
        assert_eq!(item.assignee.as_deref(), Some("Sam"));
    }

    #[test]
    fn action_item_kind_defaults_to_advisory() {
        let raw = RawActionItem {
            description: " Follow up ".to_string(),
            kind: None,
            tool_name: Some("  ".to_string()),
            assignee: None,
        };
        let item = ActionItem::from(raw);
        assert!(matches!(item.kind, ActionItemKind::Advisory));
        assert_eq!(item.description, "Follow up");
        assert!(item.tool_name.is_none());
    }

    #[test]
    fn sanitize_label_dequotes_and_caps() {
        assert_eq!(sanitize_label("\"Q3 Roadmap\"\nextra"), "Q3 Roadmap");
        let long = "A".repeat(60);
        assert_eq!(sanitize_label(&long).chars().count(), 40);
    }

    #[test]
    fn format_summary_markdown_includes_sections() {
        let summary = MeetingSummary {
            meeting_id: "m1".into(),
            headline: "Agreed to ship Friday.".into(),
            key_points: vec!["Ship Friday".into(), "QA owns sign-off".into()],
            action_items: vec![
                ActionItem {
                    description: "Send release notes".into(),
                    kind: ActionItemKind::Executable,
                    tool_name: Some("gmail".into()),
                    assignee: Some("Sam".into()),
                },
                ActionItem {
                    description: "Book retro".into(),
                    kind: ActionItemKind::Advisory,
                    tool_name: None,
                    assignee: None,
                },
            ],
            generated_at_ms: 0,
        };
        let md = format_summary_markdown(&summary, "Q3 Roadmap");
        assert!(md.contains("**Topic:** Q3 Roadmap"));
        assert!(md.contains("### Key points"));
        assert!(md.contains("- Ship Friday"));
        assert!(md.contains("### Action items"));
        assert!(md.contains("actionable via `gmail`"));
        assert!(md.contains("_(owner: Sam)_"));
        // Advisory item carries no "actionable" suffix.
        assert!(md.contains("- Book retro\n"));
    }

    #[test]
    fn format_summary_markdown_omits_empty_sections() {
        let summary = MeetingSummary {
            meeting_id: "m1".into(),
            headline: String::new(),
            key_points: vec![],
            action_items: vec![],
            generated_at_ms: 0,
        };
        let md = format_summary_markdown(&summary, "");
        assert!(md.starts_with("## Meeting summary"));
        assert!(!md.contains("### Key points"));
        assert!(!md.contains("### Action items"));
        assert!(!md.contains("**Topic:**"));
    }
}
