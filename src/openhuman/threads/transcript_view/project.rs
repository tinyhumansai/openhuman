//! Project raw session-transcript records into typed display items.
//!
//! Turns the append-only log's [`DisplayRecord`]s (message lines, compaction
//! markers, interrupted partials) into the frontend's chat vocabulary
//! ([`DisplayItem`]), sanitizing injected scaffolding as it goes. Sub-agent
//! sibling files are discovered and nested one level deep.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use crate::openhuman::agent::harness::session::transcript::{
    self, CompactionMarker, DisplayMessage, DisplayRecord,
};

use super::types::{DisplayItem, ProjectedTranscript, ToolCallFailure, ToolCallStatus};

const LOG_PREFIX: &str = "[threads][transcript]";

/// Max sub-agent nesting depth the projection descends. The plan calls for
/// one level of recursion; we allow a small bound so a delegated worker that
/// itself delegates still surfaces, without unbounded fan-out.
const MAX_SUBAGENT_DEPTH: usize = 3;

/// The scaffolding line injected onto every user message (see
/// `agent::prompts::current_datetime_line`). Stripped at projection so the UI
/// shows the user's actual words, not the per-turn time stamp.
const DATETIME_PREFIX: &str = "Current Date & Time:";

/// A legacy/alternate channel-context prefix. Kept for defensiveness; the
/// live injector currently only prepends [`DATETIME_PREFIX`].
const CHANNEL_CONTEXT_PREFIX: &str = "[Channel context]";

/// Resolve a thread's root transcript, discover its sub-agent siblings, and
/// project everything into display items. Returns `None` when the thread has
/// no root transcript yet (brand-new thread / first turn not persisted).
pub fn project_thread(workspace_dir: &Path, thread_id: &str) -> Option<ProjectedTranscript> {
    let (root_path, sub_paths) = resolve_files(workspace_dir, thread_id)?;
    Some(project_from_files(thread_id, &root_path, &sub_paths))
}

/// Resolve the on-disk file set backing a thread's transcript view: the root
/// transcript path plus every sub-agent sibling file. `None` when the thread
/// has no root transcript yet. Exposed so the cache can key on these paths
/// (and their mtimes/lengths) without re-projecting.
pub fn resolve_files(workspace_dir: &Path, thread_id: &str) -> Option<(PathBuf, Vec<PathBuf>)> {
    let root_path = transcript::find_root_transcript_for_thread(workspace_dir, thread_id)?;
    let root_stem = root_path.file_stem()?.to_str()?.to_string();
    let sub_paths = discover_subagent_files(workspace_dir, &root_stem);
    Some((root_path, sub_paths))
}

/// Project a thread from an already-resolved file set (root + sub-agent
/// siblings). Missing/unreadable files degrade to empty rather than failing.
pub fn project_from_files(
    thread_id: &str,
    root_path: &Path,
    sub_paths: &[PathBuf],
) -> ProjectedTranscript {
    let root_stem = root_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();

    log::debug!(
        "{LOG_PREFIX} projecting thread={thread_id} root={} subagent_files={}",
        root_path.display(),
        sub_paths.len()
    );

    // Read the root display records once: they feed both the top-level items
    // and the per-turn timestamp ranges used to anchor sub-agent trails.
    let (mut items, segments) = match transcript::read_transcript_display(root_path) {
        Ok(d) => (project_records(&d.records), turn_segments(&d.records)),
        Err(err) => {
            log::warn!(
                "{LOG_PREFIX} failed to read root transcript {}: {err}",
                root_path.display()
            );
            (Vec::new(), Vec::new())
        }
    };

    let subagents = build_subagent_items(sub_paths, &root_stem, 0, &segments);
    log::debug!(
        "{LOG_PREFIX} projected thread={thread_id} top_level_items={} subagents={}",
        items.len(),
        subagents.len()
    );
    items.extend(subagents);

    ProjectedTranscript {
        thread_id: thread_id.to_string(),
        items,
    }
}

/// Discover every sub-agent transcript file for `root_stem` under
/// `session_raw/`. Sub-agent stems are `{root_stem}__…`; results are sorted so
/// the timestamp-prefixed suffixes order by creation time.
fn discover_subagent_files(workspace_dir: &Path, root_stem: &str) -> Vec<PathBuf> {
    let raw_dir = workspace_dir.join("session_raw");
    let prefix = format!("{root_stem}__");
    let Ok(entries) = fs::read_dir(&raw_dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem.starts_with(&prefix))
        })
        .collect();
    paths.sort();
    paths
}

/// Build nested [`DisplayItem::Subagent`] items for the direct children of
/// `parent_stem` among `all_sub_paths`, recursing one level per depth up to
/// [`MAX_SUBAGENT_DEPTH`]. Attachment is flat (ordered by file timestamp): the
/// transcript doesn't record a robust delegation-call → file link, so we nest
/// by stem lineage rather than guessing the parent tool call.
///
/// Each item is anchored to a parent turn via [`anchor_request_id`] so the
/// frontend can render the trail under the turn that spawned it rather than the
/// most recent turn. `segments` are the root turns' start timestamps.
fn build_subagent_items(
    all_sub_paths: &[PathBuf],
    parent_stem: &str,
    depth: usize,
    segments: &[(String, i64)],
) -> Vec<DisplayItem> {
    if depth >= MAX_SUBAGENT_DEPTH {
        return Vec::new();
    }
    let child_prefix = format!("{parent_stem}__");
    let mut out = Vec::new();
    for path in all_sub_paths {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(rest) = stem.strip_prefix(&child_prefix) else {
            continue;
        };
        // Direct child only — no further `__` in the remainder.
        if rest.contains("__") {
            continue;
        }
        let display = match transcript::read_transcript_display(path) {
            Ok(d) => d,
            Err(err) => {
                log::warn!(
                    "{LOG_PREFIX} failed to read sub-agent transcript {}: {err}",
                    path.display()
                );
                continue;
            }
        };
        let mut items = project_records(&display.records);
        items.extend(build_subagent_items(
            all_sub_paths,
            stem,
            depth + 1,
            segments,
        ));
        // Prefer the archetype id from meta; fall back to the stem suffix.
        let id = if display.meta.agent_name.is_empty() {
            rest.to_string()
        } else {
            display.meta.agent_name.clone()
        };
        // Anchor to the parent turn active at the sub-agent's spawn time.
        let request_id = anchor_request_id(child_spawn_unix(rest), segments);
        log::debug!("{LOG_PREFIX} subagent id={id} stem={rest} anchored request_id={request_id:?}");
        out.push(DisplayItem::Subagent {
            id,
            request_id,
            items,
        });
    }
    out
}

/// The root turns' start timestamps as `(request_id, unix_seconds)` in file
/// order — one entry per turn boundary. Built from the first timestamped line
/// of each `request_id` run. Turns whose lines carry no `request_id` or no
/// parseable timestamp contribute nothing (legacy/CLI transcripts yield an
/// empty list, so sub-agents there stay unanchored).
fn turn_segments(records: &[DisplayRecord]) -> Vec<(String, i64)> {
    let mut segments: Vec<(String, i64)> = Vec::new();
    let mut last_request_id: Option<String> = None;
    for record in records {
        let DisplayRecord::Message(msg) = record else {
            continue;
        };
        let (Some(rid), Some(ts)) = (msg.request_id.as_deref(), msg.ts.as_deref()) else {
            continue;
        };
        if last_request_id.as_deref() == Some(rid) {
            continue;
        }
        let Some(unix) = parse_rfc3339_unix(ts) else {
            continue;
        };
        segments.push((rid.to_string(), unix));
        last_request_id = Some(rid.to_string());
    }
    segments
}

/// Extract a sub-agent's spawn unix timestamp (seconds) from its file-stem
/// suffix. Stems are `{unix_ts}_{agent_id}`; the leading integer is the agent
/// build/spawn time (see the transcript module's stem docs). `None` for
/// non-numeric legacy stems.
fn child_spawn_unix(stem_suffix: &str) -> Option<i64> {
    stem_suffix
        .split('_')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
}

/// Parse an RFC-3339 timestamp into unix seconds.
fn parse_rfc3339_unix(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.timestamp())
}

/// Anchor a sub-agent to the parent turn that was active at its `child_unix`
/// spawn time: the last turn segment whose start is `<= child_unix`.
///
/// Fallbacks (documented heuristic, since sub-agent files carry no explicit
/// delegation-call back-link):
/// - No segments (legacy/CLI root): `None` — the item stays unanchored and the
///   frontend leaves it under the current turn cursor, as before.
/// - Unknown spawn time (non-numeric stem): the newest turn (best effort).
/// - Spawn time precedes every turn start: the first turn.
fn anchor_request_id(child_unix: Option<i64>, segments: &[(String, i64)]) -> Option<String> {
    if segments.is_empty() {
        return None;
    }
    let Some(child_unix) = child_unix else {
        return segments.last().map(|(rid, _)| rid.clone());
    };
    let mut chosen = &segments[0];
    for seg in segments {
        if seg.1 <= child_unix {
            chosen = seg;
        }
    }
    Some(chosen.0.clone())
}

/// Project one file's display records into display items, in file order.
///
/// - System lines are dropped (they carry the tool-policy preamble and other
///   scaffolding that must never render as a chat item).
/// - `reasoning_content` on an assistant line becomes a [`DisplayItem::Reasoning`]
///   preceding its message.
/// - Assistant `tool_calls` register pending [`DisplayItem::ToolCall`]s; a later
///   `role:"tool"` line pairs to one by id, falling back to FIFO order.
/// - Interrupted partials and compaction markers pass through as their items.
/// - A [`DisplayItem::TurnBoundary`] is emitted whenever `request_id` changes.
pub fn project_records(records: &[DisplayRecord]) -> Vec<DisplayItem> {
    let mut items: Vec<DisplayItem> = Vec::new();
    // Pending tool calls awaiting a result line: (call_id, index into `items`).
    let mut pending: VecDeque<(String, usize)> = VecDeque::new();
    let mut last_request_id: Option<String> = None;

    for record in records {
        match record {
            DisplayRecord::Message(msg) => {
                maybe_emit_turn_boundary(msg, &mut last_request_id, &mut items);
                project_message(msg, &mut items, &mut pending);
            }
            DisplayRecord::Compaction(marker) => {
                project_compaction(marker, &mut items);
                // A compaction supersedes prior context; drop stale pending
                // pairings so a post-compaction result never binds to them.
                pending.clear();
            }
        }
    }
    items
}

fn maybe_emit_turn_boundary(
    msg: &DisplayMessage,
    last_request_id: &mut Option<String>,
    items: &mut Vec<DisplayItem>,
) {
    let Some(rid) = msg.request_id.as_deref() else {
        return;
    };
    if last_request_id.as_deref() != Some(rid) {
        items.push(DisplayItem::TurnBoundary {
            request_id: rid.to_string(),
        });
        *last_request_id = Some(rid.to_string());
    }
}

fn project_message(
    msg: &DisplayMessage,
    items: &mut Vec<DisplayItem>,
    pending: &mut VecDeque<(String, usize)>,
) {
    // Interrupted partial: display-only, carries its own thinking.
    if msg.interrupted {
        items.push(DisplayItem::InterruptedPartial {
            text: msg.message.content.clone(),
            thinking: msg.reasoning_content.clone(),
        });
        return;
    }

    match msg.message.role.as_str() {
        "system" => {
            // Scaffolding (tool-policy preamble, etc.) — never a display item.
            log::debug!("{LOG_PREFIX} sanitize: dropped system line from projection");
        }
        "user" => {
            let raw = msg.message.content.clone();
            let sanitized = sanitize_user_content(&raw);
            if sanitized.is_some() {
                log::debug!("{LOG_PREFIX} sanitize: stripped injected prefix from user message");
            }
            items.push(DisplayItem::UserMessage {
                content: raw,
                display_content: sanitized,
                request_id: msg.request_id.clone(),
            });
        }
        "assistant" => project_assistant(msg, items, pending),
        "tool" => project_tool_result(msg, items, pending),
        other => {
            log::debug!("{LOG_PREFIX} projecting unknown role {other:?} as assistant message");
            items.push(DisplayItem::AssistantMessage {
                content: msg.message.content.clone(),
                interim: false,
                request_id: msg.request_id.clone(),
                model: msg.turn_usage.as_ref().map(|tu| tu.model.clone()),
                iteration: msg.iteration,
            });
        }
    }
}

fn project_assistant(
    msg: &DisplayMessage,
    items: &mut Vec<DisplayItem>,
    pending: &mut VecDeque<(String, usize)>,
) {
    // Reasoning precedes the message it belongs to.
    if let Some(reasoning) = msg.reasoning_content.as_deref() {
        if !reasoning.trim().is_empty() {
            items.push(DisplayItem::Reasoning {
                text: reasoning.to_string(),
            });
        }
    }

    let tool_calls = msg
        .turn_usage
        .as_ref()
        .map(|tu| tu.tool_calls.as_slice())
        .unwrap_or_default();
    let interim = !tool_calls.is_empty();

    // The assistant's prose (if any) shows before its tool calls.
    if !msg.message.content.trim().is_empty() {
        items.push(DisplayItem::AssistantMessage {
            content: msg.message.content.clone(),
            interim,
            request_id: msg.request_id.clone(),
            model: msg.turn_usage.as_ref().map(|tu| tu.model.clone()),
            iteration: msg.iteration,
        });
    }

    for call in tool_calls {
        let args = parse_tool_args(&call.arguments);
        items.push(DisplayItem::ToolCall {
            call_id: call.id.clone(),
            name: call.name.clone(),
            args,
            result: None,
            status: ToolCallStatus::Running,
            failure: None,
        });
        pending.push_back((call.id.clone(), items.len() - 1));
    }
}

fn project_tool_result(
    msg: &DisplayMessage,
    items: &mut Vec<DisplayItem>,
    pending: &mut VecDeque<(String, usize)>,
) {
    let result = msg.message.content.clone();
    // A failed tool line (`ToolResult::is_error`, stamped at persistence) pairs
    // to an error row with a failure payload instead of a false success.
    let (status, failure) = if msg.failure {
        (
            ToolCallStatus::Error,
            Some(ToolCallFailure {
                detail: msg.failure_detail.clone(),
            }),
        )
    } else {
        (ToolCallStatus::Success, None)
    };
    // Pair by explicit call id first, else FIFO.
    let idx = msg
        .message
        .id
        .as_deref()
        .and_then(|id| take_pending_by_id(pending, id))
        .or_else(|| pending.pop_front().map(|(_, idx)| idx));

    if let Some(idx) = idx {
        if let Some(DisplayItem::ToolCall {
            result: slot,
            status: status_slot,
            failure: failure_slot,
            ..
        }) = items.get_mut(idx)
        {
            *slot = Some(result);
            *status_slot = status;
            *failure_slot = failure;
            return;
        }
    }

    // Orphan result (no matching assistant tool_call recorded) — surface it as
    // a best-effort completed tool row so the output is not lost.
    log::debug!("{LOG_PREFIX} tool result with no pending call — emitting orphan tool row");
    items.push(DisplayItem::ToolCall {
        call_id: msg.message.id.clone().unwrap_or_default(),
        name: "tool".to_string(),
        args: None,
        result: Some(result),
        status,
        failure,
    });
}

/// Remove and return the pending entry whose call id matches `id`, if any.
fn take_pending_by_id(pending: &mut VecDeque<(String, usize)>, id: &str) -> Option<usize> {
    let pos = pending.iter().position(|(cid, _)| cid == id)?;
    pending.remove(pos).map(|(_, idx)| idx)
}

fn project_compaction(marker: &CompactionMarker, items: &mut Vec<DisplayItem>) {
    items.push(DisplayItem::Compaction {
        replaced_count: 0,
        kept_count: marker.replacement.len(),
        ts: marker.ts.clone(),
        request_id: marker.request_id.clone(),
    });
}

/// Parse a tool call's raw argument string into JSON when possible; a
/// non-JSON string is wrapped so the frontend still receives structured args.
fn parse_tool_args(raw: &str) -> Option<serde_json::Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => Some(value),
        Err(_) => Some(serde_json::Value::String(trimmed.to_string())),
    }
}

/// Strip the injected scaffolding prefix from a user message, returning the
/// sanitized body only when a prefix was actually present (so the caller can
/// tag rather than mutate — the raw `content` is preserved alongside).
fn sanitize_user_content(content: &str) -> Option<String> {
    let trimmed_start = content.trim_start();
    if trimmed_start.starts_with(DATETIME_PREFIX)
        || trimmed_start.starts_with(CHANNEL_CONTEXT_PREFIX)
    {
        // The injector prepends the scaffolding line followed by a blank line,
        // then the user's actual text. Strip the first paragraph.
        if let Some(idx) = content.find("\n\n") {
            let body = content[idx + 2..].to_string();
            return Some(body);
        }
        // No body after the prefix — the whole message was scaffolding.
        return Some(String::new());
    }
    None
}
