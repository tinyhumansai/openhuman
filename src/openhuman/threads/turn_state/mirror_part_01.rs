use crate::openhuman::agent::progress::AgentProgress;

use super::store::TurnStateStore;
use super::types::{
    PersistedToolFailure, SubagentActivity, SubagentToolCall, SubagentTranscriptItem,
    ToolTimelineEntry, ToolTimelineStatus, TranscriptItem, TurnLifecycle, TurnPhase, TurnState,
};

const MIRROR_LOG_PREFIX: &str = "[threads:turn_state:mirror]";

/// Upper bound on the tool result text persisted per timeline row. The
/// snapshot file is rewritten in full at every tool boundary, so this is
/// deliberately tighter than the 256 KiB live-socket cap — it bounds the
/// per-flush rewrite while still giving the rehydrated "View processing"
/// panel a meaningful result preview.
const MAX_PERSISTED_TOOL_OUTPUT: usize = 64 * 1024;

/// Bytes reserved within the cap for the truncation marker so the final
/// persisted payload (content + marker) never exceeds
/// [`MAX_PERSISTED_TOOL_OUTPUT`].
const TRUNCATION_MARKER_BUDGET: usize = 80;

/// Upper bound on a single persisted transcript prose item (one coalesced
/// narration or reasoning block, parent or sub-agent). A runaway reasoning
/// stream would otherwise grow one item without bound and bloat every
/// full-file snapshot rewrite. Tighter than [`MAX_PERSISTED_TOOL_OUTPUT`]
/// because a turn can accumulate many prose items.
const MAX_PERSISTED_TRANSCRIPT_ITEM: usize = 16 * 1024;

/// Marker appended once when a transcript prose item is truncated at its cap.
const TRANSCRIPT_TRUNCATION_MARKER: &str = "\n…[truncated]";

/// Upper bound on the child tool arguments persisted per sub-agent call.
/// Sized like [`MAX_PERSISTED_TRANSCRIPT_ITEM`] rather than the looser
/// [`MAX_PERSISTED_TOOL_OUTPUT`]: a sub-agent turn accumulates many child
/// calls and the snapshot file is rewritten in full at every tool boundary,
/// so one `write_file`-shaped payload must not dominate every rewrite.
const MAX_PERSISTED_TOOL_ARGS: usize = 16 * 1024;

/// Append `delta` to a coalescing transcript prose buffer, enforcing
/// [`MAX_PERSISTED_TRANSCRIPT_ITEM`] on a char boundary and stamping a one-time
/// truncation marker the first time the cap is hit. Once at the cap, further
/// deltas are dropped (the marker is already present). Used by both the parent
/// and sub-agent transcript coalescers so a single streamed block stays bounded.
fn append_capped_transcript_text(text: &mut String, delta: &str) {
    if delta.is_empty() {
        return;
    }
    if text.len() >= MAX_PERSISTED_TRANSCRIPT_ITEM {
        // Already at the cap but more content is arriving — ensure the marker is
        // present exactly once so the truncation is visible even when deltas
        // land exactly on the boundary (never straddling it).
        if !text.ends_with(TRANSCRIPT_TRUNCATION_MARKER) {
            text.push_str(TRANSCRIPT_TRUNCATION_MARKER);
        }
        return;
    }
    let remaining = MAX_PERSISTED_TRANSCRIPT_ITEM - text.len();
    if delta.len() <= remaining {
        text.push_str(delta);
        return;
    }
    let mut end = remaining;
    while end > 0 && !delta.is_char_boundary(end) {
        end -= 1;
    }
    text.push_str(&delta[..end]);
    text.push_str(TRANSCRIPT_TRUNCATION_MARKER);
}

/// Cap `output` for snapshot persistence, slicing on a char boundary and
/// appending a truncation marker when content was dropped. Returns `None`
/// for empty output (payload capture off) so the field serializes away.
fn cap_persisted_output(output: &str) -> Option<String> {
    if output.is_empty() {
        return None;
    }
    if output.len() <= MAX_PERSISTED_TOOL_OUTPUT {
        return Some(output.to_string());
    }
    let mut end = MAX_PERSISTED_TOOL_OUTPUT.saturating_sub(TRUNCATION_MARKER_BUDGET);
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    let omitted = output.len() - end;
    Some(format!(
        "{}\n…[truncated {omitted} bytes of tool output]",
        &output[..end]
    ))
}

/// Cap `arguments` for snapshot persistence. Returns `None` for a null
/// payload, which would otherwise persist as a meaningless "Input: null" row.
///
/// A null is not the same as "this call had no input": on the tinyagents path
/// the *started* event always carries `Value::Null` and the captured arguments
/// only arrive with `SubagentToolCallCompleted`, so the completion arm
/// backfills what this returns `None` for at start.
///
/// A payload that serialises within [`MAX_PERSISTED_TOOL_ARGS`] is kept
/// verbatim, so the rehydrated row renders the same structured input the live
/// row did. An oversized one degrades to a truncated *string* carrying the
/// same marker shape [`cap_persisted_output`] uses: the prefix is what the
/// reader actually wants ("what did it search for?"), and a string is honest
/// about no longer being parseable JSON.
fn cap_persisted_args(arguments: &serde_json::Value) -> Option<serde_json::Value> {
    if arguments.is_null() {
        return None;
    }
    let rendered = arguments.to_string();
    if rendered.len() <= MAX_PERSISTED_TOOL_ARGS {
        return Some(arguments.clone());
    }
    let mut end = MAX_PERSISTED_TOOL_ARGS.saturating_sub(TRUNCATION_MARKER_BUDGET);
    while end > 0 && !rendered.is_char_boundary(end) {
        end -= 1;
    }
    let omitted = rendered.len() - end;
    Some(serde_json::Value::String(format!(
        "{}\n…[truncated {omitted} bytes of tool arguments]",
        &rendered[..end]
    )))
}

/// In-process cursor that keeps the authoritative [`TurnState`] in sync
/// with the agent loop and writes it through to a [`TurnStateStore`].
pub struct TurnStateMirror {
    store: TurnStateStore,
    state: TurnState,
    /// Set to `true` once we observe `TurnCompleted` so `finish` knows
    /// to delete the snapshot rather than mark it interrupted.
    turn_completed: bool,
    /// Monotonic ordering key for [`TranscriptItem`]s. Round alone can't
    /// order narration vs thinking vs tool calls *within* one iteration, so
    /// every transcript push stamps and increments this.
    next_seq: u32,
    /// Separate monotonic ordering key for [`ToolTimelineEntry::seq`] — the flat
    /// timeline is an independent projection from the interleaved transcript, so
    /// it gets its own space (sharing `next_seq` would leave gaps in the
    /// transcript's contiguous ordering).
    next_tool_seq: u64,
}
