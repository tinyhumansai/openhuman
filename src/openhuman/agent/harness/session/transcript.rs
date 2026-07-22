//! Session transcript persistence for KV cache stability.
//!
//! **Source of truth**: `session_raw/{stem}.jsonl` — a *flat* directory.
//!
//! Each JSONL file starts with a single metadata line (identified by an
//! `_meta` key) followed by one JSON object per record. On every
//! write the companion `.md` file is re-rendered for human readability
//! under `sessions/{YYYY_MM_DD}/{stem}.md`; it is **never** read back —
//! all round-trip / resume logic uses the JSONL.
//!
//! ## Append-only log (Phase A — transcript-derived view)
//!
//! The JSONL is an **append-only event log**. [`write_transcript`] still
//! full-rewrites (used by one-shot writers: migrations, sub-agent runners,
//! tests), but the incremental session persistence path
//! (`persist_session_transcript`) uses [`append_transcript_turn`], which
//! never rewrites existing lines. It classifies the incoming logical
//! message set against what was last persisted:
//!
//! - **Pure extension** (previously-persisted messages are a prefix of the
//!   new set): only the new tail message lines are appended.
//! - **Reduction / rewrite** (context reduction dropped or replaced earlier
//!   turns — the previously-persisted set is *not* a prefix): a single
//!   `{"kind":"compaction","replacement":[…]}` record is appended carrying
//!   the full reduced message set. Earlier turns stay on disk untouched.
//!
//! Cumulative `_meta` totals are kept fresh without rewriting the file by
//! appending a fresh `{"_meta":{…}}` line each turn; readers take the **last**
//! `_meta` line as authoritative (line 1 remains a valid fallback for old
//! cores that only read the header).
//!
//! ### Two read paths
//!
//! - **Model context** ([`read_transcript`] / `read_transcript_jsonl`) replays
//!   the log: message lines accumulate, a `compaction` record **replaces** the
//!   accumulator with its `replacement`, and `interrupted:true` partial lines
//!   are **skipped** (they never entered the model's context). The result is
//!   byte-identical to what the old full-rewrite approach produced.
//! - **Display** ([`read_transcript_display`]) returns **every** record in file
//!   order — pre-compaction history, compaction markers, and interrupted
//!   partials — so the UI projection (Phase B) can render the full timeline.
//!
//! ### Compatibility
//!
//! Existing files (zero compaction records, no `version`) read identically.
//! New record kinds and fields are additive: [`MessageLine`] carries a
//! `#[serde(flatten)] _extra` catch-all and `MetaPayload` does not set
//! `deny_unknown_fields`, so an **old** core reading a **new** file skips
//! unknown-kind lines (a compaction record fails the `role`/`content`
//! requirement and is logged+skipped) rather than crashing. The `_meta`
//! carries a `version` field (`TRANSCRIPT_SCHEMA_VERSION`) for future readers.
//!
//! ## Storage layout
//!
//! ```text
//! {workspace}/session_raw/{stem}.jsonl              ← source of truth (flat)
//! {workspace}/sessions/YYYY_MM_DD/{stem}.md         ← human-readable view
//! ```
//!
//! `stem` is `{unix_ts}_{agent_id}` for a root session, or
//! `{parent_chain}__{unix_ts}_{agent_id}` for a sub-agent. Because the
//! stem starts with the unix timestamp at agent-build time, a directory
//! listing of `session_raw/` is naturally sorted by creation time and
//! `find_latest_transcript` becomes O(scan one dir, filter by suffix)
//! — it does not depend on the calendar date, so a session that's been
//! idle for weeks resumes the same way as one from yesterday.
//!
//! ## Backward compatibility
//!
//! Older releases wrote into `session_raw/DDMMYYYY/{stem}.jsonl` (and
//! the legacy `sessions/DDMMYYYY/{stem}.md`). [`find_latest_transcript`]
//! falls back to scanning those date-grouped dirs when the flat
//! directory yields nothing, so users upgrading don't lose resume.
//!
//! ## JSONL schema
//!
//! **Line 1 (meta):**
//! ```json
//! {"_meta":{"agent":"code_executor","dispatcher":"native","created":"...","updated":"...","turn_count":3,"input_tokens":5000,"output_tokens":1200,"cached_input_tokens":3500,"charged_amount_usd":0.0045,"thread_id":"thr_abc123"}}
//! ```
//!
//! **Message lines:**
//! ```json
//! {"role":"system","content":"..."}
//! {"role":"user","content":"..."}
//! {"role":"assistant","content":"...","model":"claude-...","usage":{"input":1234,"output":567,"cached_input":1000,"cost_usd":0.0012},"ts":"2026-04-17T..."}
//! {"role":"tool","content":"..."}
//! ```
//!
//! Only `role` and `content` are required. All other fields are optional.
//! UI-visible rows may also carry a stable `id` and `extra_metadata` so
//! the session transcript can eventually replace the separate thread
//! message log without losing message-level addressing.

use crate::openhuman::inference::provider::ChatMessage;
use crate::openhuman::inference::provider::ToolCall;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};

// ── Types ────────────────────────────────────────────────────────────

/// Per-message usage figures attributed to the last assistant turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageUsage {
    pub input: u64,
    pub output: u64,
    pub cached_input: u64,
    #[serde(default)]
    pub context_window: u64,
    pub cost_usd: f64,
}

/// Usage + provenance for one provider response, attached to the last
/// assistant message in a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnUsage {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    pub usage: MessageUsage,
    /// RFC-3339 timestamp of the response.
    #[serde(default)]
    pub ts: String,
    /// Raw reasoning/thinking content returned by thinking models. This is
    /// persisted as metadata so the later transcript view can show the model's
    /// thoughts without depending on the live stream still being open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// Native tool calls emitted in this provider response, if any. Text-mode
    /// calls remain present in `content` as the raw markup the model emitted.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// One-based engine iteration for this provider response.
    #[serde(default)]
    pub iteration: u32,
}

const TURN_USAGE_METADATA_KEY: &str = "openhuman_turn_usage";

/// `extra_metadata` key carrying a tool-result message's failure marker. The
/// harness folds a tool result into a `role:"tool"` message that drops the
/// per-call failure flag (`ToolResult::is_error`), so the turn loop re-attaches
/// the outcome here — from the captured `ToolCallOutcome` side-channel — before
/// persistence. `extra_metadata` is `#[serde(skip_serializing)]` on
/// [`ChatMessage`], so this never reaches the provider; the transcript writer
/// lifts it onto the additive [`MessageLine::failure`] / `failure_detail` line
/// fields and strips it from the persisted `extra_metadata`.
const TOOL_FAILURE_METADATA_KEY: &str = "openhuman_tool_failure";

/// Stamp a tool-result [`ChatMessage`] with its failure outcome so the
/// transcript writer can persist an explicit failure flag. `detail` is an
/// optional short, single-line reason (e.g. the head of the error output).
/// No-op semantics: pass this only for genuinely failed tool calls.
pub(crate) fn attach_tool_failure_metadata(message: &mut ChatMessage, detail: Option<&str>) {
    let mut payload = serde_json::Map::new();
    payload.insert("failure".to_string(), serde_json::Value::Bool(true));
    if let Some(detail) = detail.map(str::trim).filter(|s| !s.is_empty()) {
        payload.insert(
            "detail".to_string(),
            serde_json::Value::String(detail.to_string()),
        );
    }
    let marker = serde_json::Value::Object(payload);

    match message.extra_metadata.take() {
        Some(serde_json::Value::Object(mut map)) => {
            map.insert(TOOL_FAILURE_METADATA_KEY.to_string(), marker);
            message.extra_metadata = Some(serde_json::Value::Object(map));
        }
        Some(existing) => {
            let mut map = serde_json::Map::new();
            map.insert("value".to_string(), existing);
            map.insert(TOOL_FAILURE_METADATA_KEY.to_string(), marker);
            message.extra_metadata = Some(serde_json::Value::Object(map));
        }
        None => {
            let mut map = serde_json::Map::new();
            map.insert(TOOL_FAILURE_METADATA_KEY.to_string(), marker);
            message.extra_metadata = Some(serde_json::Value::Object(map));
        }
    }
}

/// Pop the tool-failure marker out of a cloned `extra_metadata` map, returning
/// `Some((true, detail))` when it was present. Strips the key so it is not
/// duplicated into the persisted `extra_metadata` alongside the top-level
/// `failure` line field. Legacy lines without the marker return `None`.
fn take_tool_failure(extra: &mut Option<serde_json::Value>) -> Option<(bool, Option<String>)> {
    let serde_json::Value::Object(map) = extra.as_mut()? else {
        return None;
    };
    let marker = map.remove(TOOL_FAILURE_METADATA_KEY)?;
    // If removing the marker emptied the object, drop `extra_metadata` entirely
    // so a legacy-identical line stays legacy-identical.
    if map.is_empty() {
        *extra = None;
    }
    let detail = marker
        .get("detail")
        .and_then(|d| d.as_str())
        .map(str::to_string);
    Some((true, detail))
}

/// Schema version stamped on the `_meta` header line. Bumped when the JSONL
/// record shape changes in a way future readers may need to branch on. `0`
/// (absent) denotes pre-append-only files written before this field existed.
pub const TRANSCRIPT_SCHEMA_VERSION: u32 = 1;

/// Discriminator value for a compaction record's `kind` field.
const COMPACTION_KIND: &str = "compaction";

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

pub(crate) fn attach_turn_usage_metadata(message: &mut ChatMessage, turn_usage: &TurnUsage) {
    let Ok(payload) = serde_json::to_value(turn_usage) else {
        log::warn!("[transcript] failed to serialize turn usage metadata");
        return;
    };

    match message.extra_metadata.take() {
        Some(serde_json::Value::Object(mut map)) => {
            map.insert(TURN_USAGE_METADATA_KEY.to_string(), payload);
            message.extra_metadata = Some(serde_json::Value::Object(map));
        }
        Some(existing) => {
            let mut map = serde_json::Map::new();
            map.insert("value".to_string(), existing);
            map.insert(TURN_USAGE_METADATA_KEY.to_string(), payload);
            message.extra_metadata = Some(serde_json::Value::Object(map));
        }
        None => {
            let mut map = serde_json::Map::new();
            map.insert(TURN_USAGE_METADATA_KEY.to_string(), payload);
            message.extra_metadata = Some(serde_json::Value::Object(map));
        }
    }
}

pub(crate) fn turn_usage_extra_metadata(turn_usage: &TurnUsage) -> Option<serde_json::Value> {
    let mut message = ChatMessage::assistant("");
    attach_turn_usage_metadata(&mut message, turn_usage);
    message.extra_metadata
}

fn turn_usage_from_metadata(message: &ChatMessage) -> Option<TurnUsage> {
    let payload = message
        .extra_metadata
        .as_ref()?
        .get(TURN_USAGE_METADATA_KEY)?;
    serde_json::from_value(payload.clone()).ok()
}

/// Metadata header for a session transcript file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptMeta {
    pub agent_name: String,
    /// Canonical registry id for the agent that produced this transcript.
    /// `agent_name` may be per-thread renamed for file names; this remains the
    /// stable archetype id when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Coarse runtime kind (`root`, `subagent`, `extractor`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    pub dispatcher: String,
    /// Provider label used for the most recent recorded response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model id used for the most recent recorded response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub created: String,
    pub updated: String,
    pub turn_count: usize,
    /// Cumulative input tokens across all provider calls this session.
    pub input_tokens: u64,
    /// Cumulative output tokens across all provider calls this session.
    pub output_tokens: u64,
    /// Cumulative input tokens served from the KV cache.
    pub cached_input_tokens: u64,
    /// Cumulative amount charged in USD.
    pub charged_amount_usd: f64,
    /// Backend-side LLM thread identifier (the `thread_id` forwarded on
    /// `/openai/v1/chat/completions` so the OpenHuman backend can group
    /// `InferenceLog` entries and align KV-cache keys with the same logical
    /// chat thread the user sees in the UI). `None` for runs that don't
    /// originate from a thread-scoped channel (e.g. CLI-only sessions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Sub-agent task id, when this transcript belongs to a spawned worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// A parsed session transcript: metadata + exact message array.
#[derive(Debug, Clone)]
pub struct SessionTranscript {
    pub meta: TranscriptMeta,
    pub messages: Vec<ChatMessage>,
}

// ── Internal JSONL types ─────────────────────────────────────────────

/// The `_meta` line serialisation shape.
#[derive(Serialize, Deserialize)]
struct MetaLine {
    #[serde(rename = "_meta")]
    meta: MetaPayload,
}

#[derive(Serialize, Deserialize)]
struct MetaPayload {
    /// Schema version of the transcript record format (see
    /// [`TRANSCRIPT_SCHEMA_VERSION`]). Absent (deserialises to `0`) on files
    /// written before the append-only migration.
    #[serde(default)]
    version: u32,
    agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_type: Option<String>,
    dispatcher: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    created: String,
    updated: String,
    turn_count: usize,
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    charged_amount_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
}

/// One message line in the JSONL — only `role` and `content` are required.
/// All other fields are optional; unknown fields are flattened to preserve
/// forward-compatibility.
#[derive(Serialize, Deserialize)]
struct MessageLine {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    role: String,
    content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra_metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<MessageUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iteration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ts: Option<String>,
    /// Turn boundary marker: the web-chat `request_id` this message belongs to,
    /// when available. Stamped on every line of a turn so the display projection
    /// can group a turn's messages. Absent for CLI / non-request-scoped runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    /// `true` when this line is a *partial* assistant answer captured because
    /// the turn was interrupted/cancelled mid-stream. Present for **display
    /// only** — the model-context reader skips these so a resumed context never
    /// carries a truncated answer.
    #[serde(default, skip_serializing_if = "is_false")]
    interrupted: bool,
    /// `true` when this tool-result line's tool call **failed**
    /// (`ToolResult::is_error`). Additive + optional: legacy lines and every
    /// non-tool line omit it and default to success. Lifted from the tool
    /// message's failure metadata by [`build_message_line`]; consumed by the
    /// display projection to render an error tool row instead of success.
    #[serde(default, skip_serializing_if = "is_false")]
    failure: bool,
    /// Optional short, single-line reason for a failed tool call (the head of
    /// the error output). Present only alongside `failure: true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure_detail: Option<String>,
    /// Absorb any unknown fields so forward-compat reads don't error.
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

/// A compaction record: `{"kind":"compaction","replacement":[…]}`.
///
/// Appended when the harness reduces context (post-compaction / trim) so the
/// model-context reader can reconstruct the reduced set without the file being
/// destructively rewritten. `replacement` is the **full** logical message set
/// that supersedes everything before it — an explicit replacement list
/// (mirroring Codex's `Compacted { replacement_history }`) rather than
/// surviving-message ids, because our writer already holds the reduced
/// `messages` slice on each persist call and message ids are optional, so an
/// id-reference scheme would be less robust for no gain.
#[derive(Serialize, Deserialize)]
struct CompactionLine {
    kind: String,
    replacement: Vec<MessageLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

// ── Display read types ───────────────────────────────────────────────

/// One message in a display projection, carrying the turn-boundary + partial
/// flags the model-context [`SessionTranscript`] discards.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub message: ChatMessage,
    /// `true` when this is an interrupted partial answer (display only).
    pub interrupted: bool,
    /// Turn boundary marker (`request_id`), when stamped.
    pub request_id: Option<String>,
    pub iteration: Option<u32>,
    pub ts: Option<String>,
    /// Usage/provenance for assistant messages that carried it.
    pub turn_usage: Option<TurnUsage>,
    /// Raw reasoning/thinking captured for this line, when present. Mirrors the
    /// line's `reasoning_content` directly so it survives even on lines without
    /// full turn-usage provenance (e.g. an interrupted partial, which carries no
    /// provider/model/usage). Prefer this over digging into [`Self::turn_usage`]
    /// for display: it is populated from `turn_usage.reasoning_content` too.
    pub reasoning_content: Option<String>,
    /// `true` when this is a **failed** tool-result line (`ToolResult::is_error`
    /// at execution time). The display projection renders an error tool row
    /// instead of success. Always `false` for non-tool lines and legacy files.
    pub failure: bool,
    /// Optional short reason for a failed tool call (present only with
    /// `failure: true`).
    pub failure_detail: Option<String>,
}

/// A compaction marker in a display projection.
#[derive(Debug, Clone)]
pub struct CompactionMarker {
    /// The reduced message set this compaction installed as the new context.
    pub replacement: Vec<DisplayMessage>,
    pub ts: Option<String>,
    pub request_id: Option<String>,
}

/// One record in a display projection, in file order.
#[derive(Debug, Clone)]
pub enum DisplayRecord {
    Message(DisplayMessage),
    Compaction(CompactionMarker),
}

/// A display projection of a transcript: **all** records, including
/// pre-compaction history, compaction markers, and interrupted partials.
#[derive(Debug, Clone)]
pub struct DisplaySessionTranscript {
    pub meta: TranscriptMeta,
    pub records: Vec<DisplayRecord>,
}

// ── Write ─────────────────────────────────────────────────────────────

/// Build the serialised `_meta` header line for `meta`, stamping the current
/// [`TRANSCRIPT_SCHEMA_VERSION`].
fn meta_payload_from(meta: &TranscriptMeta) -> MetaPayload {
    MetaPayload {
        version: TRANSCRIPT_SCHEMA_VERSION,
        agent: meta.agent_name.clone(),
        agent_id: meta.agent_id.clone(),
        agent_type: meta.agent_type.clone(),
        dispatcher: meta.dispatcher.clone(),
        provider: meta.provider.clone(),
        model: meta.model.clone(),
        created: meta.created.clone(),
        updated: meta.updated.clone(),
        turn_count: meta.turn_count,
        input_tokens: meta.input_tokens,
        output_tokens: meta.output_tokens,
        cached_input_tokens: meta.cached_input_tokens,
        charged_amount_usd: meta.charged_amount_usd,
        thread_id: meta.thread_id.clone(),
        task_id: meta.task_id.clone(),
    }
}

fn meta_line_json(meta: &TranscriptMeta) -> Result<String> {
    let meta_line = MetaLine {
        meta: meta_payload_from(meta),
    };
    serde_json::to_string(&meta_line).context("serialise transcript meta header")
}

/// Build a [`MessageLine`] for `msg`, folding in `turn_usage` (assistant rows)
/// and stamping the `request_id` turn boundary when supplied.
fn build_message_line(
    msg: &ChatMessage,
    turn_usage: Option<&TurnUsage>,
    request_id: Option<&str>,
    interrupted: bool,
) -> MessageLine {
    let assistant_usage = if msg.role == "assistant" {
        turn_usage
    } else {
        None
    };
    // Lift any tool-failure marker off a cloned `extra_metadata` onto the
    // additive top-level `failure` / `failure_detail` line fields, stripping it
    // so it is not persisted twice.
    let mut extra_metadata = msg.extra_metadata.clone();
    let (failure, failure_detail) = match take_tool_failure(&mut extra_metadata) {
        Some((failed, detail)) => (failed, detail),
        None => (false, None),
    };
    MessageLine {
        id: msg.id.clone(),
        role: msg.role.clone(),
        content: msg.content.clone(),
        extra_metadata,
        provider: assistant_usage.map(|tu| tu.provider.clone()),
        model: assistant_usage.map(|tu| tu.model.clone()),
        usage: assistant_usage.map(|tu| tu.usage.clone()),
        reasoning_content: assistant_usage.and_then(|tu| tu.reasoning_content.clone()),
        tool_calls: assistant_usage.and_then(|tu| {
            if tu.tool_calls.is_empty() {
                None
            } else {
                Some(tu.tool_calls.clone())
            }
        }),
        iteration: assistant_usage.map(|tu| tu.iteration),
        ts: assistant_usage.map(|tu| tu.ts.clone()),
        request_id: request_id.map(str::to_string),
        interrupted,
        failure,
        failure_detail,
        _extra: HashMap::new(),
    }
}

/// Serialise `messages` into JSONL message lines, attributing
/// `last_assistant_turn_usage` (or per-message embedded usage) to the last
/// assistant row and stamping `request_id` on every line.
fn serialise_message_lines(
    messages: &[ChatMessage],
    last_assistant_turn_usage: Option<&TurnUsage>,
    request_id: Option<&str>,
    buf: &mut String,
) -> Result<()> {
    let last_assistant_idx = messages.iter().rposition(|m| m.role == "assistant");
    for (i, msg) in messages.iter().enumerate() {
        let turn_usage = if Some(i) == last_assistant_idx {
            last_assistant_turn_usage
                .cloned()
                .or_else(|| turn_usage_from_metadata(msg))
        } else {
            turn_usage_from_metadata(msg)
        };
        let line = build_message_line(msg, turn_usage.as_ref(), request_id, false);
        let line_json =
            serde_json::to_string(&line).with_context(|| format!("serialise message line {i}"))?;
        buf.push_str(&line_json);
        buf.push('\n');
    }
    Ok(())
}

/// Write JSONL as source of truth **and** re-render the companion `.md`.
///
/// `jsonl_path` must end in `.jsonl`; the `.md` companion is derived by
/// swapping the extension. **Full rewrite** on every call — this is the
/// one-shot writer used by migrations, the sub-agent runners, and tests.
/// The incremental session-persistence path uses [`append_transcript_turn`]
/// instead, which never rewrites existing lines.
pub fn write_transcript(
    jsonl_path: &Path,
    messages: &[ChatMessage],
    meta: &TranscriptMeta,
    last_assistant_turn_usage: Option<&TurnUsage>,
) -> Result<()> {
    if let Some(parent) = jsonl_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create transcript dir {}", parent.display()))?;
    }

    // ── JSONL ────────────────────────────────────────────────────────
    let mut jsonl_buf = String::new();
    jsonl_buf.push_str(&meta_line_json(meta)?);
    jsonl_buf.push('\n');
    serialise_message_lines(messages, last_assistant_turn_usage, None, &mut jsonl_buf)?;

    fs::write(jsonl_path, jsonl_buf.as_bytes())
        .with_context(|| format!("write transcript {}", jsonl_path.display()))?;

    log::debug!(
        "[transcript] wrote {} messages (jsonl, full rewrite) to {}",
        messages.len(),
        jsonl_path.display()
    );

    render_md_companion(jsonl_path, messages, meta, last_assistant_turn_usage);
    Ok(())
}

/// Append this turn's delta to an **append-only** transcript, never rewriting
/// existing lines.
///
/// `prev_persisted` is the logical message set the previous call left on disk
/// (empty on the first call for a fresh file). The incoming `messages` is the
/// current full logical set for this turn:
///
/// - **Pure extension** (`prev_persisted` is a prefix of `messages`): only the
///   new tail is appended as message lines.
/// - **Reduction / rewrite** (context reduction changed or dropped earlier
///   turns): a single `compaction` record carrying the full reduced
///   `messages` is appended; earlier lines are left untouched on disk.
///
/// A fresh `_meta` line is appended so cumulative totals stay current without a
/// full rewrite. The `.md` companion is re-rendered from `messages` (derived
/// view — always the reduced/current set). Returns nothing; the caller updates
/// its tracked `prev_persisted` to `messages` on success.
///
/// `request_id` (when available from the web-chat path) is stamped on every
/// appended line as a turn boundary marker.
pub fn append_transcript_turn(
    jsonl_path: &Path,
    prev_persisted: &[ChatMessage],
    messages: &[ChatMessage],
    meta: &TranscriptMeta,
    turn_usage: Option<&TurnUsage>,
    request_id: Option<&str>,
) -> Result<()> {
    if let Some(parent) = jsonl_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create transcript dir {}", parent.display()))?;
    }

    let file_exists = jsonl_path.exists();

    // First write for this file: create it with meta + all message lines.
    if !file_exists {
        let mut buf = String::new();
        buf.push_str(&meta_line_json(meta)?);
        buf.push('\n');
        serialise_message_lines(messages, turn_usage, request_id, &mut buf)?;
        fs::write(jsonl_path, buf.as_bytes())
            .with_context(|| format!("create transcript {}", jsonl_path.display()))?;
        log::debug!(
            "[transcript] created append-only transcript with {} message(s) at {}",
            messages.len(),
            jsonl_path.display()
        );
        render_md_companion(jsonl_path, messages, meta, turn_usage);
        return Ok(());
    }

    // Subsequent writes: diff against the previously-persisted logical set.
    let common = common_prefix_len(prev_persisted, messages);
    let mut buf = String::new();

    if common == prev_persisted.len() {
        // Pure extension — append only the new tail.
        let tail = &messages[common..];
        log::debug!(
            "[transcript] append: extending on-disk set (prev={}, new={}, appending {} tail line(s)) {}",
            prev_persisted.len(),
            messages.len(),
            tail.len(),
            jsonl_path.display()
        );
        serialise_message_lines(tail, turn_usage, request_id, &mut buf)?;
    } else {
        // Reduction / rewrite — the on-disk set is no longer a prefix. Append a
        // compaction record carrying the full reduced context so the
        // model-context reader can replay it, without destroying earlier lines.
        log::debug!(
            "[transcript] append: context reduced (prev={}, new={}, common_prefix={}) — writing compaction record {}",
            prev_persisted.len(),
            messages.len(),
            common,
            jsonl_path.display()
        );
        let last_assistant_idx = messages.iter().rposition(|m| m.role == "assistant");
        let replacement: Vec<MessageLine> = messages
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                let tu = if Some(i) == last_assistant_idx {
                    turn_usage
                        .cloned()
                        .or_else(|| turn_usage_from_metadata(msg))
                } else {
                    turn_usage_from_metadata(msg)
                };
                build_message_line(msg, tu.as_ref(), request_id, false)
            })
            .collect();
        let compaction = CompactionLine {
            kind: COMPACTION_KIND.to_string(),
            replacement,
            ts: Some(chrono::Utc::now().to_rfc3339()),
            request_id: request_id.map(str::to_string),
            _extra: HashMap::new(),
        };
        let line = serde_json::to_string(&compaction).context("serialise compaction record")?;
        buf.push_str(&line);
        buf.push('\n');
    }

    // Refresh cumulative meta by appending a new `_meta` line (readers take the
    // last one). Keeps append-only + O(1)-per-turn (no full-file rewrite).
    buf.push_str(&meta_line_json(meta)?);
    buf.push('\n');

    append_bytes(jsonl_path, buf.as_bytes())?;
    render_md_companion(jsonl_path, messages, meta, turn_usage);
    Ok(())
}

/// Append a partial assistant answer, flagged `interrupted: true`, captured
/// when a streaming turn was cancelled/interrupted before completion.
///
/// **Display only**: the model-context reader skips interrupted lines, so a
/// resumed context never carries a truncated answer. Does not affect the
/// caller's tracked `prev_persisted` (nothing about the logical model context
/// changed). No-op when `partial_content` is empty.
pub fn append_interrupted_partial(
    jsonl_path: &Path,
    partial_content: &str,
    request_id: Option<&str>,
    iteration: Option<u32>,
    reasoning_content: Option<&str>,
) -> Result<()> {
    if partial_content.is_empty() {
        return Ok(());
    }
    if let Some(parent) = jsonl_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create transcript dir {}", parent.display()))?;
    }
    let mut line = build_message_line(
        &ChatMessage::assistant(partial_content),
        None,
        request_id,
        true,
    );
    line.iteration = iteration;
    line.reasoning_content = reasoning_content
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    line.ts = Some(chrono::Utc::now().to_rfc3339());
    let mut buf = serde_json::to_string(&line).context("serialise interrupted partial line")?;
    buf.push('\n');
    append_bytes(jsonl_path, buf.as_bytes())?;
    log::debug!(
        "[transcript] appended interrupted partial ({} chars, request_id={:?}) to {}",
        partial_content.len(),
        request_id,
        jsonl_path.display()
    );
    Ok(())
}

/// Longest common prefix length between two message slices, comparing on the
/// stable, serialised fields (`role`, `content`, `id`). `ChatMessage` does not
/// derive `PartialEq`, and `extra_metadata` is intentionally excluded because
/// it is enriched (turn usage) between the in-memory history and the persisted
/// line, which must not count as a divergence.
fn common_prefix_len(a: &[ChatMessage], b: &[ChatMessage]) -> usize {
    a.iter()
        .zip(b.iter())
        .take_while(|(x, y)| x.role == y.role && x.content == y.content && x.id == y.id)
        .count()
}

/// Append raw bytes to a file, opening in append mode (O(1), no read-back).
fn append_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open transcript for append {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("append transcript {}", path.display()))?;
    Ok(())
}

/// Re-render the derived `.md` companion from the current (reduced) message set.
///
/// Best-effort — the JSONL is the source of truth; a companion write failure is
/// logged and swallowed so it can never take down state persistence.
fn render_md_companion(
    jsonl_path: &Path,
    messages: &[ChatMessage],
    meta: &TranscriptMeta,
    last_assistant_turn_usage: Option<&TurnUsage>,
) {
    let last_assistant_idx = messages.iter().rposition(|m| m.role == "assistant");
    let mut owned_usage: Vec<(usize, TurnUsage)> = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        let usage = if Some(idx) == last_assistant_idx {
            last_assistant_turn_usage
                .cloned()
                .or_else(|| turn_usage_from_metadata(msg))
        } else {
            turn_usage_from_metadata(msg)
        };
        if let Some(usage) = usage {
            owned_usage.push((idx, usage));
        }
    }
    let per_msg_usage: HashMap<usize, &TurnUsage> = owned_usage
        .iter()
        .map(|(idx, usage)| (*idx, usage))
        .collect();

    let md_path = md_companion_path(jsonl_path);
    if let Some(parent) = md_path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            log::warn!(
                "[transcript] failed to create md companion dir {}: {err}",
                parent.display()
            );
            return;
        }
    }
    let md = render_markdown(messages, meta, &per_msg_usage);
    if let Err(err) = fs::write(&md_path, md.as_bytes()) {
        log::warn!(
            "[transcript] failed to write markdown companion {}: {err}",
            md_path.display()
        );
        return;
    }
    log::debug!(
        "[transcript] wrote markdown companion to {}",
        md_path.display()
    );
}

// ── Read ─────────────────────────────────────────────────────────────

/// Read a session transcript.
///
/// **Primary path**: reads the `.jsonl` source of truth.
/// **Fallback**: if the `.jsonl` does not exist but the legacy `.md` does
/// (migration path — old sessions), reads it via the legacy HTML-comment
/// parser and returns a `SessionTranscript` with default meta where the
/// `.md` format didn't track a field.
pub fn read_transcript(path: &Path) -> Result<SessionTranscript> {
    // Route by extension first: a legacy `.md` path (returned by
    // `find_latest_transcript` when only legacy files exist) must go to
    // the legacy parser, never to the JSONL parser.
    if path.extension().and_then(|s| s.to_str()) == Some("md") {
        log::debug!(
            "[transcript] reading legacy .md transcript: {}",
            path.display()
        );
        return read_transcript_legacy_md(path);
    }

    if path.exists() {
        read_transcript_jsonl(path)
    } else {
        // Fallback: try the .md sibling (legacy one-release compat).
        let md_path = path.with_extension("md");
        if md_path.exists() {
            log::debug!(
                "[transcript] .jsonl not found, falling back to legacy .md: {}",
                md_path.display()
            );
            read_transcript_legacy_md(&md_path)
        } else {
            // Neither exists — propagate the original jsonl error.
            read_transcript_jsonl(path)
        }
    }
}

/// Convert a parsed `MetaPayload` into the public [`TranscriptMeta`].
fn meta_from_payload(mp: MetaPayload) -> TranscriptMeta {
    TranscriptMeta {
        agent_name: mp.agent,
        agent_id: mp.agent_id,
        agent_type: mp.agent_type,
        dispatcher: mp.dispatcher,
        provider: mp.provider,
        model: mp.model,
        created: mp.created,
        updated: mp.updated,
        turn_count: mp.turn_count,
        input_tokens: mp.input_tokens,
        output_tokens: mp.output_tokens,
        cached_input_tokens: mp.cached_input_tokens,
        charged_amount_usd: mp.charged_amount_usd,
        thread_id: mp.thread_id,
        task_id: mp.task_id,
    }
}

/// Recover the [`TurnUsage`] a message line carried (assistant rows only).
fn turn_usage_from_line(ml: &MessageLine) -> Option<TurnUsage> {
    match (
        ml.provider.clone(),
        ml.model.clone(),
        ml.usage.clone(),
        ml.ts.clone(),
    ) {
        (Some(provider), Some(model), Some(usage), Some(ts)) if ml.role == "assistant" => {
            Some(TurnUsage {
                provider,
                model,
                usage,
                ts,
                reasoning_content: ml.reasoning_content.clone(),
                tool_calls: ml.tool_calls.clone().unwrap_or_default(),
                iteration: ml.iteration.unwrap_or_default(),
            })
        }
        _ => None,
    }
}

/// Reconstruct a [`ChatMessage`] from a message line, re-attaching turn-usage
/// metadata so the round-trip is lossless for the model-context path.
fn message_from_line(ml: MessageLine) -> ChatMessage {
    let turn_usage = turn_usage_from_line(&ml);
    let mut message = ChatMessage {
        id: ml.id,
        role: ml.role,
        content: ml.content,
        extra_metadata: ml.extra_metadata,
    };
    if let Some(turn_usage) = turn_usage.as_ref() {
        attach_turn_usage_metadata(&mut message, turn_usage);
    }
    message
}

/// Classification of one non-empty JSONL line.
enum LineKind {
    Meta(MetaLine),
    Compaction(CompactionLine),
    Message(MessageLine),
}

/// Classify a raw line: a `_meta` header/update, a `compaction` record, or a
/// message line. Returns `Err` only when the line is malformed for its
/// apparent kind; the caller decides whether that is fatal (first line) or a
/// skippable warning (later lines).
fn classify_line(line: &str) -> Result<LineKind, serde_json::Error> {
    // Cheap structural peek. Unknown/other shapes fall through to MessageLine,
    // whose required `role`/`content` gate rejects genuinely foreign lines.
    let value: serde_json::Value = serde_json::from_str(line)?;
    if value.get("_meta").is_some() {
        return serde_json::from_str::<MetaLine>(line).map(LineKind::Meta);
    }
    if value.get("kind").and_then(|k| k.as_str()) == Some(COMPACTION_KIND) {
        return serde_json::from_str::<CompactionLine>(line).map(LineKind::Compaction);
    }
    serde_json::from_str::<MessageLine>(line).map(LineKind::Message)
}

fn read_transcript_jsonl(path: &Path) -> Result<SessionTranscript> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read transcript jsonl {}", path.display()))?;

    let mut meta: Option<TranscriptMeta> = None;
    let mut messages: Vec<ChatMessage> = Vec::new();
    let mut compactions_replayed = 0usize;
    let mut interrupted_skipped = 0usize;

    // Append-only log replay (Phase A): the first non-empty line MUST be the
    // `_meta` header; subsequent lines are messages, `compaction` records
    // (which *replace* the accumulated context), interrupted partials (skipped
    // for the model-context path), or refreshed `_meta` lines (last wins).
    let mut seen_first = false;
    for (line_no, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if !seen_first {
            seen_first = true;
            let ml: MetaLine = serde_json::from_str(line).map_err(|err| {
                anyhow::anyhow!(
                    "first non-empty line of {} (line {}) is not a valid _meta object: {err}",
                    path.display(),
                    line_no + 1,
                )
            })?;
            meta = Some(meta_from_payload(ml.meta));
            continue;
        }

        match classify_line(line) {
            Ok(LineKind::Meta(ml)) => {
                // Refreshed cumulative meta — last one wins.
                meta = Some(meta_from_payload(ml.meta));
            }
            Ok(LineKind::Compaction(cl)) => {
                // Reduction record: the reduced context REPLACES everything
                // accumulated so far, exactly reproducing the old full-rewrite.
                let replacement: Vec<ChatMessage> =
                    cl.replacement.into_iter().map(message_from_line).collect();
                log::debug!(
                    "[transcript] replay: compaction at line {} replaces {} accumulated message(s) with {} (request_id={:?}) in {}",
                    line_no + 1,
                    messages.len(),
                    replacement.len(),
                    cl.request_id,
                    path.display()
                );
                messages = replacement;
                compactions_replayed += 1;
            }
            Ok(LineKind::Message(ml)) => {
                if ml.interrupted {
                    // Display-only partial — never part of the model context.
                    interrupted_skipped += 1;
                    log::debug!(
                        "[transcript] replay: skipping interrupted partial line {} (display only) in {}",
                        line_no + 1,
                        path.display()
                    );
                    continue;
                }
                messages.push(message_from_line(ml));
            }
            Err(err) => {
                log::warn!(
                    "[transcript] skipping malformed/unknown record line {} in {}: {err}",
                    line_no + 1,
                    path.display()
                );
            }
        }
    }

    let meta = meta.with_context(|| {
        format!(
            "missing _meta header line in jsonl transcript {}",
            path.display()
        )
    })?;

    log::debug!(
        "[transcript] loaded {} messages (jsonl, {} compaction(s) replayed, {} interrupted skipped) from {}",
        messages.len(),
        compactions_replayed,
        interrupted_skipped,
        path.display()
    );

    Ok(SessionTranscript { meta, messages })
}

// ── Display read ──────────────────────────────────────────────────────

/// Reconstruct a [`DisplayMessage`] from a message line, preserving the
/// turn-boundary + partial flags the model-context path discards.
fn display_message_from_line(ml: MessageLine) -> DisplayMessage {
    let turn_usage = turn_usage_from_line(&ml);
    let reasoning_content = ml.reasoning_content.clone().or_else(|| {
        turn_usage
            .as_ref()
            .and_then(|tu| tu.reasoning_content.clone())
    });
    DisplayMessage {
        interrupted: ml.interrupted,
        request_id: ml.request_id.clone(),
        iteration: ml.iteration,
        ts: ml.ts.clone(),
        turn_usage,
        reasoning_content,
        failure: ml.failure,
        failure_detail: ml.failure_detail.clone(),
        message: ChatMessage {
            id: ml.id,
            role: ml.role,
            content: ml.content,
            extra_metadata: ml.extra_metadata,
        },
    }
}

/// Read a transcript for **display**: returns *every* record in file order,
/// including pre-compaction history, compaction markers, and interrupted
/// partials — the counterpart to the model-context [`read_transcript`], which
/// collapses the log into the reduced context.
///
/// `meta` reflects the newest `_meta` line (cumulative totals stay current).
pub fn read_transcript_display(path: &Path) -> Result<DisplaySessionTranscript> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read transcript jsonl (display) {}", path.display()))?;

    let mut meta: Option<TranscriptMeta> = None;
    let mut records: Vec<DisplayRecord> = Vec::new();
    let mut seen_first = false;

    for (line_no, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !seen_first {
            seen_first = true;
            let ml: MetaLine = serde_json::from_str(line).map_err(|err| {
                anyhow::anyhow!(
                    "first non-empty line of {} (line {}) is not a valid _meta object: {err}",
                    path.display(),
                    line_no + 1,
                )
            })?;
            meta = Some(meta_from_payload(ml.meta));
            continue;
        }
        match classify_line(line) {
            Ok(LineKind::Meta(ml)) => meta = Some(meta_from_payload(ml.meta)),
            Ok(LineKind::Compaction(cl)) => {
                let replacement = cl
                    .replacement
                    .into_iter()
                    .map(display_message_from_line)
                    .collect();
                records.push(DisplayRecord::Compaction(CompactionMarker {
                    replacement,
                    ts: cl.ts,
                    request_id: cl.request_id,
                }));
            }
            Ok(LineKind::Message(ml)) => {
                records.push(DisplayRecord::Message(display_message_from_line(ml)));
            }
            Err(err) => {
                log::warn!(
                    "[transcript] display: skipping malformed/unknown record line {} in {}: {err}",
                    line_no + 1,
                    path.display()
                );
            }
        }
    }

    let meta = meta.with_context(|| {
        format!(
            "missing _meta header line in jsonl transcript {}",
            path.display()
        )
    })?;

    log::debug!(
        "[transcript] display-loaded {} record(s) from {}",
        records.len(),
        path.display()
    );

    Ok(DisplaySessionTranscript { meta, records })
}

/// Find the newest root `session_raw/*.jsonl` transcript whose metadata
/// declares `thread_id`.
///
/// Root transcripts live directly under `session_raw/` and do not carry
/// the `__` separator used for sub-agent siblings. This helper is the
/// bridge PR-2 can use to route UI thread reads to the canonical root
/// transcript without accidentally folding delegated worker transcripts
/// into the main chat timeline.
pub fn find_root_transcript_for_thread(workspace_dir: &Path, thread_id: &str) -> Option<PathBuf> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return None;
    }

    let raw_dir = raw_session_dir(workspace_dir);
    let entries = fs::read_dir(&raw_dir).ok()?;
    let mut matches: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|s| s.to_str()) == Some("jsonl")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|stem| !stem.contains("__"))
        })
        .filter(|path| match read_transcript(path) {
            Ok(transcript) => transcript.meta.thread_id.as_deref() == Some(thread_id),
            Err(err) => {
                log::warn!(
                    "[transcript] skipping unreadable root transcript candidate {}: {err}",
                    path.display()
                );
                false
            }
        })
        .collect();

    matches.sort();
    matches.pop()
}

/// Aggregated token/cost usage for a chat thread, summed across **all** of the
/// thread's root session transcripts (a thread reopened across days/restarts
/// produces several files). `last_turn_*`, `model`, and `updated` come from the
/// newest transcript so the UI can render a context-window gauge for the most
/// recent turn. Returns `None` when no transcript exists yet (a brand-new
/// thread with no completed turns).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadUsageSummary {
    /// Orchestrator (parent) token totals — the root transcript(s) only. Root
    /// transcripts never include sub-agent calls (those go to a separate
    /// observer + their own `__` transcript files); see [`Self::subagents`].
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
    pub turn_count: usize,
    /// Input/output tokens of the most recent assistant turn (context gauge).
    pub last_turn_input_tokens: u64,
    pub last_turn_output_tokens: u64,
    /// Model that served the most recent turn, if recorded.
    pub model: Option<String>,
    /// RFC-3339 `updated` of the newest transcript.
    pub updated: String,
    /// Per-archetype sub-agent spend, reconstructed from the thread's `__`
    /// sub-agent transcripts (grouped by `agent_name`).
    pub subagents: Vec<SubagentArchetypeUsage>,
}

/// One sub-agent archetype's summed spend within a thread (e.g. all `coder`
/// runs). `model` is the model that served one of its runs, used to price it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubagentArchetypeUsage {
    pub agent_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    /// How many sub-agent runs of this archetype contributed.
    pub runs: usize,
    pub model: Option<String>,
}

/// Parse the authoritative `_meta` of a root transcript JSONL.
///
/// Append-only files carry the immutable header on line 1 plus a refreshed
/// `_meta` line per turn (cumulative totals). The **last** `_meta` line wins,
/// so a multi-turn session reports its running totals — not just the first
/// turn's. Falls back to line 1 for legacy single-header files.
fn read_transcript_meta_only(path: &Path) -> Option<TranscriptMeta> {
    let raw = fs::read_to_string(path).ok()?;
    let mut latest: Option<TranscriptMeta> = None;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(ml) = serde_json::from_str::<MetaLine>(line) {
            latest = Some(meta_from_payload(ml.meta));
        } else if latest.is_none() {
            // The first non-empty line must be a valid meta header.
            return None;
        }
    }
    latest
}

/// Extract the last assistant message's usage + model from a transcript JSONL.
/// Only the final assistant message of a turn carries these (see the JSONL
/// format docs at the top of this module). Compaction records and refreshed
/// `_meta` lines are skipped; a `compaction` record's `replacement` assistant
/// rows are considered so a compacted transcript still surfaces its latest
/// usage.
fn read_last_assistant_usage(path: &Path) -> Option<(MessageUsage, Option<String>)> {
    let raw = fs::read_to_string(path).ok()?;
    let mut result = None;
    let mut seen_first = false;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !seen_first {
            seen_first = true; // first non-empty line is the `_meta` header
            continue;
        }
        match classify_line(line) {
            Ok(LineKind::Message(ml)) if ml.role == "assistant" && !ml.interrupted => {
                if let Some(usage) = ml.usage {
                    result = Some((usage, ml.model));
                }
            }
            Ok(LineKind::Compaction(cl)) => {
                for ml in &cl.replacement {
                    if ml.role == "assistant" {
                        if let Some(usage) = ml.usage.clone() {
                            result = Some((usage, ml.model.clone()));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    result
}

/// Summed token/cost usage for `thread_id` across its root transcripts, or
/// `None` when the thread has no persisted turns yet.
pub fn read_thread_usage_summary(
    workspace_dir: &Path,
    thread_id: &str,
) -> Option<ThreadUsageSummary> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return None;
    }

    let raw_dir = raw_session_dir(workspace_dir);
    let entries = fs::read_dir(&raw_dir).ok()?;

    // Single scan: split the thread's transcripts into root (orchestrator) and
    // `__` sub-agent files. Root totals stay the parent's; sub-agent files are
    // grouped by archetype for the per-agent breakdown.
    let mut root_matches: Vec<PathBuf> = Vec::new();
    let mut sub_matches: Vec<PathBuf> = Vec::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let is_subagent = stem.contains("__");
        let matches_thread = read_transcript_meta_only(&path)
            .map(|m| m.thread_id.as_deref() == Some(thread_id))
            .unwrap_or(false);
        if !matches_thread {
            continue;
        }
        if is_subagent {
            sub_matches.push(path);
        } else {
            root_matches.push(path);
        }
    }

    if root_matches.is_empty() && sub_matches.is_empty() {
        return None;
    }
    root_matches.sort();

    let mut summary = ThreadUsageSummary::default();
    for path in &root_matches {
        if let Some(meta) = read_transcript_meta_only(path) {
            summary.input_tokens = summary.input_tokens.saturating_add(meta.input_tokens);
            summary.output_tokens = summary.output_tokens.saturating_add(meta.output_tokens);
            summary.cached_input_tokens = summary
                .cached_input_tokens
                .saturating_add(meta.cached_input_tokens);
            summary.cost_usd += meta.charged_amount_usd;
            summary.turn_count = summary.turn_count.saturating_add(meta.turn_count);
        }
    }

    // Newest root transcript drives the last-turn gauge + model + updated stamp.
    if let Some(newest) = root_matches.last() {
        if let Some(meta) = read_transcript_meta_only(newest) {
            summary.updated = meta.updated;
        }
        if let Some((usage, model)) = read_last_assistant_usage(newest) {
            summary.last_turn_input_tokens = usage.input;
            summary.last_turn_output_tokens = usage.output;
            summary.model = model;
        }
    }

    // Group sub-agent transcripts by archetype (`agent_name`).
    let mut groups: BTreeMap<String, SubagentArchetypeUsage> = BTreeMap::new();
    for path in &sub_matches {
        let Some(meta) = read_transcript_meta_only(path) else {
            continue;
        };
        let group =
            groups
                .entry(meta.agent_name.clone())
                .or_insert_with(|| SubagentArchetypeUsage {
                    agent_id: meta.agent_name.clone(),
                    ..Default::default()
                });
        group.input_tokens = group.input_tokens.saturating_add(meta.input_tokens);
        group.output_tokens = group.output_tokens.saturating_add(meta.output_tokens);
        group.cached_input_tokens = group
            .cached_input_tokens
            .saturating_add(meta.cached_input_tokens);
        group.runs = group.runs.saturating_add(1);
        if group.model.is_none() {
            if let Some((_, model)) = read_last_assistant_usage(path) {
                group.model = model;
            }
        }
    }
    summary.subagents = groups.into_values().collect();

    Some(summary)
}

// ── Path resolution ──────────────────────────────────────────────────

/// Resolve a transcript path under `session_raw/{stem}.jsonl` — a
/// *flat* directory keyed only by stem. Used by the session-key flow:
/// the stem is `"{unix_ts}_{agent_id}"` for a root session, or
/// `"{parent_chain}__{session_key}"` for a sub-agent, so nested
/// delegations still produce a single flat filename that encodes the
/// parent → child path.
///
/// Creates the directory if needed. Overwrites are intentional: the
/// `Agent` persists the same transcript file across every turn of a
/// session, and every sub-agent spawn gets a unique timestamp in its
/// own key so collisions are effectively impossible.
pub fn resolve_keyed_transcript_path(workspace_dir: &Path, stem: &str) -> Result<PathBuf> {
    let raw_dir = raw_session_dir(workspace_dir);
    fs::create_dir_all(&raw_dir)
        .with_context(|| format!("create session_raw dir {}", raw_dir.display()))?;
    let sanitized = sanitize_stem(stem);
    Ok(raw_dir.join(format!("{sanitized}.jsonl")))
}

/// Sanitize a user-supplied transcript stem so it never escapes the
/// `session_raw/` directory. Allows ASCII alphanumerics plus a small
/// punctuation set (`_`, `-`, `.`); every other byte is replaced with
/// `_`. Empty inputs fall back to `"session"`.
fn sanitize_stem(stem: &str) -> String {
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "session".to_string()
    } else {
        cleaned
    }
}

pub fn resolve_new_transcript_path(workspace_dir: &Path, agent_name: &str) -> Result<PathBuf> {
    let raw_dir = raw_session_dir(workspace_dir);
    fs::create_dir_all(&raw_dir)
        .with_context(|| format!("create session_raw dir {}", raw_dir.display()))?;

    let sanitized = sanitize_agent_name(agent_name);
    let idx_raw = next_index(&raw_dir, &sanitized)?;
    // Also consider today's md companion dir so a stale .md from this
    // session doesn't cause an index collision when only .md exists.
    let md_dir = today_md_session_dir(workspace_dir);
    let idx_md = next_index(&md_dir, &sanitized)?;
    let next_idx = idx_raw.max(idx_md);
    let filename = format!("{}_{}.jsonl", sanitized, next_idx);

    Ok(raw_dir.join(filename))
}

/// Find the most recent transcript for `agent_name`.
///
/// **Primary**: scan the flat `session_raw/` directory and pick the
/// newest matching stem (root sessions only — sub-agents are skipped).
/// **Fallback**: scan the legacy `session_raw/DDMMYYYY/` dirs (today
/// and yesterday) and the legacy `sessions/DDMMYYYY/` markdown dirs so
/// users upgrading from the date-grouped layout don't lose resume.
/// The fallback is one-release transitional and can be removed once
/// existing transcripts have rolled forward.
pub fn find_latest_transcript(workspace_dir: &Path, agent_name: &str) -> Option<PathBuf> {
    let sanitized = sanitize_agent_name(agent_name);
    let raw_root = workspace_dir.join("session_raw");
    let sessions_root = workspace_dir.join("sessions");

    // Primary path: flat session_raw/ directory. The stem-suffix scan
    // is naturally date-independent, so an idle thread resumes the same
    // way today as it did weeks ago.
    if raw_root.is_dir() {
        if let Some(path) = latest_in_dir(&raw_root, &sanitized) {
            return Some(path);
        }
    }

    // Fallback: legacy date-grouped layout (one-release migration
    // window). Today first, then yesterday — matches the previous
    // behaviour so we don't regress while users still have files in
    // the old structure.
    let today = chrono::Local::now().format("%d%m%Y").to_string();
    let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
        .format("%d%m%Y")
        .to_string();

    for date_str in [&today, &yesterday] {
        let raw_dir = raw_root.join(date_str);
        if raw_dir.is_dir() {
            if let Some(path) = latest_in_dir(&raw_dir, &sanitized) {
                return Some(path);
            }
        }
        let legacy_dir = sessions_root.join(date_str);
        if legacy_dir.is_dir() {
            if let Some(path) = latest_in_dir(&legacy_dir, &sanitized) {
                return Some(path);
            }
        }
    }

    None
}

// ── Markdown rendering ────────────────────────────────────────────────

/// Render a human-readable markdown representation of the transcript.
///
/// This output is **for humans only** — it is never read back by the
/// application. All resume / round-trip logic uses the JSONL source of truth.
fn render_markdown(
    messages: &[ChatMessage],
    meta: &TranscriptMeta,
    per_message_usage: &HashMap<usize, &TurnUsage>,
) -> String {
    let mut buf = String::new();

    let _ = writeln!(buf, "# Session transcript — {}", meta.agent_name);
    buf.push('\n');
    let _ = writeln!(buf, "- Dispatcher: {}", meta.dispatcher);
    if let Some(agent_id) = meta.agent_id.as_deref() {
        let _ = writeln!(buf, "- Agent ID: `{agent_id}`");
    }
    if let Some(agent_type) = meta.agent_type.as_deref() {
        let _ = writeln!(buf, "- Agent type: `{agent_type}`");
    }
    if let Some(provider) = meta.provider.as_deref() {
        let _ = writeln!(buf, "- Provider: `{provider}`");
    }
    if let Some(model) = meta.model.as_deref() {
        let _ = writeln!(buf, "- Model: `{model}`");
    }
    if let Some(task_id) = meta.task_id.as_deref() {
        let _ = writeln!(buf, "- Task: `{task_id}`");
    }
    if let Some(tid) = meta.thread_id.as_deref() {
        let _ = writeln!(buf, "- Thread: `{tid}`");
    }
    let _ = writeln!(buf, "- Turns: {}", meta.turn_count);
    if meta.input_tokens > 0 || meta.output_tokens > 0 {
        let cache_pct = if meta.input_tokens > 0 {
            (meta.cached_input_tokens as f64 / meta.input_tokens as f64) * 100.0
        } else {
            0.0
        };
        let _ = writeln!(
            buf,
            "- Tokens: {} in / {} out / {} cached ({:.1}% hit)",
            meta.input_tokens, meta.output_tokens, meta.cached_input_tokens, cache_pct
        );
    }
    if meta.charged_amount_usd > 0.0 {
        let _ = writeln!(buf, "- Charged: ${:.6}", meta.charged_amount_usd);
    }
    let _ = writeln!(buf, "- Updated: {}", meta.updated);

    for (i, msg) in messages.iter().enumerate() {
        buf.push_str("\n---\n\n");

        if let Some(tu) = per_message_usage.get(&i) {
            let _ = writeln!(
                buf,
                "## [{}] · {} · {} in / {} out / {} cached · ${:.6}",
                msg.role,
                tu.model,
                tu.usage.input,
                tu.usage.output,
                tu.usage.cached_input,
                tu.usage.cost_usd
            );
            if !tu.provider.is_empty() || tu.usage.context_window > 0 {
                let _ = writeln!(
                    buf,
                    "_provider: `{}` · iteration: {} · context window: {}_",
                    tu.provider, tu.iteration, tu.usage.context_window
                );
            }
            if let Some(reasoning) = tu.reasoning_content.as_deref().filter(|s| !s.is_empty()) {
                let _ = writeln!(buf, "\n### Thoughts\n\n{reasoning}\n");
            }
        } else {
            let _ = writeln!(buf, "## [{}]", msg.role);
        }

        buf.push('\n');
        buf.push_str(&msg.content);
        buf.push('\n');
    }

    buf
}

// ── Legacy .md reader (one-release migration compat) ─────────────────

/// Read a legacy HTML-comment `.md` transcript. Used as a fallback when
/// only a `.md` exists (no `.jsonl` sibling).
///
/// Returns a `SessionTranscript` with whatever fields the `.md` tracked;
/// fields the old format didn't carry are defaulted.
pub fn read_transcript_legacy_md(path: &Path) -> Result<SessionTranscript> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read legacy transcript {}", path.display()))?;

    let meta = parse_legacy_meta(&raw)
        .with_context(|| format!("parse legacy transcript meta in {}", path.display()))?;

    let messages = parse_legacy_messages(&raw)
        .with_context(|| format!("parse legacy transcript messages in {}", path.display()))?;

    log::debug!(
        "[transcript] loaded {} messages (legacy md) from {}",
        messages.len(),
        path.display()
    );

    Ok(SessionTranscript { meta, messages })
}

const LEGACY_MSG_OPEN_PREFIX: &str = "<!--MSG role=\"";
const LEGACY_MSG_OPEN_SUFFIX: &str = "\"-->";
const LEGACY_MSG_CLOSE: &str = "<!--/MSG-->";
const LEGACY_MSG_CLOSE_ESCAPED: &str = "<!--\\/MSG-->";

fn parse_legacy_meta(raw: &str) -> Result<TranscriptMeta> {
    let header_start = raw
        .find("<!-- session_transcript")
        .context("missing session_transcript header")?;
    let header_end = raw[header_start..]
        .find("-->")
        .context("unclosed session_transcript header")?;
    let header = &raw[header_start..header_start + header_end + 3];

    let get = |key: &str| -> Option<String> {
        header.lines().find_map(|line| {
            let line = line.trim();
            if line.starts_with(&format!("{key}:")) {
                Some(line[key.len() + 1..].trim().to_string())
            } else {
                None
            }
        })
    };

    Ok(TranscriptMeta {
        agent_name: get("agent").unwrap_or_else(|| "unknown".into()),
        dispatcher: get("dispatcher").unwrap_or_else(|| "native".into()),
        agent_id: None,
        agent_type: None,
        provider: None,
        model: None,
        created: get("created").unwrap_or_default(),
        updated: get("updated").unwrap_or_default(),
        turn_count: get("turn_count").and_then(|s| s.parse().ok()).unwrap_or(0),
        input_tokens: get("input_tokens")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        output_tokens: get("output_tokens")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        cached_input_tokens: get("cached_input_tokens")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        charged_amount_usd: get("charged_usd")
            .and_then(|s| s.trim_start_matches('$').parse().ok())
            .unwrap_or(0.0),
        thread_id: get("thread_id").filter(|s| !s.is_empty()),
        task_id: None,
    })
}

fn parse_legacy_messages(raw: &str) -> Result<Vec<ChatMessage>> {
    let mut messages = Vec::new();
    let mut search_from = 0;

    loop {
        let Some(open_start) = raw[search_from..].find(LEGACY_MSG_OPEN_PREFIX) else {
            break;
        };
        let open_start = search_from + open_start;
        let after_prefix = open_start + LEGACY_MSG_OPEN_PREFIX.len();

        let Some(role_end) = raw[after_prefix..].find(LEGACY_MSG_OPEN_SUFFIX) else {
            break;
        };
        let role = raw[after_prefix..after_prefix + role_end].to_string();

        let content_start = after_prefix + role_end + LEGACY_MSG_OPEN_SUFFIX.len();
        let content_start = if raw[content_start..].starts_with('\n') {
            content_start + 1
        } else {
            content_start
        };

        let close_tag = format!("\n{LEGACY_MSG_CLOSE}");
        let Some(content_end_rel) = raw[content_start..].find(&close_tag) else {
            let Some(content_end_rel) = raw[content_start..].find(LEGACY_MSG_CLOSE) else {
                break;
            };
            let content = &raw[content_start..content_start + content_end_rel];
            messages.push(ChatMessage {
                id: None,
                role,
                content: content.replace(LEGACY_MSG_CLOSE_ESCAPED, LEGACY_MSG_CLOSE),
                extra_metadata: None,
            });
            search_from = content_start + content_end_rel + LEGACY_MSG_CLOSE.len();
            continue;
        };

        let content = &raw[content_start..content_start + content_end_rel];
        messages.push(ChatMessage {
            id: None,
            role,
            content: content.replace(LEGACY_MSG_CLOSE_ESCAPED, LEGACY_MSG_CLOSE),
            extra_metadata: None,
        });

        search_from = content_start + content_end_rel + close_tag.len();
    }

    Ok(messages)
}

// ── Private helpers ───────────────────────────────────────────────────

/// Date-grouped directory for human-readable `.md` companions, e.g.
/// `{workspace}/sessions/2026_05_02`. ISO-style `YYYY_MM_DD` so the
/// listing sorts lexicographically by date.
fn today_md_session_dir(workspace_dir: &Path) -> PathBuf {
    let date = chrono::Local::now().format("%Y_%m_%d").to_string();
    workspace_dir.join("sessions").join(date)
}

/// Flat directory for the JSONL source of truth, e.g.
/// `{workspace}/session_raw`. Stems start with `{unix_ts}` so the
/// listing is naturally time-ordered without a date subdirectory.
fn raw_session_dir(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("session_raw")
}

/// Given a `session_raw/{stem}.jsonl` path, derive the companion
/// `sessions/YYYY_MM_DD/{stem}.md` path. The date is taken from the
/// local clock at write time — fine for browsing because the source
/// of truth lives in the flat raw dir; the `.md` is purely a view.
///
/// Legacy `session_raw/DDMMYYYY/{stem}.jsonl` paths (still on disk
/// from older releases until they roll forward) keep their date
/// component when generating the companion so we don't accidentally
/// stamp old transcripts with today's date.
///
/// If no `session_raw` component is present (tests using a flat
/// tempdir), the companion sits alongside as a sibling `.md`.
fn md_companion_path(jsonl_path: &Path) -> PathBuf {
    let components: Vec<_> = jsonl_path.components().collect();

    let raw_idx = components
        .iter()
        .position(|comp| matches!(comp, std::path::Component::Normal(s) if *s == "session_raw"));

    let Some(raw_idx) = raw_idx else {
        return jsonl_path.with_extension("md");
    };

    let mut out = PathBuf::new();
    for comp in &components[..raw_idx] {
        out.push(comp.as_os_str());
    }
    out.push("sessions");

    // Tail after `session_raw`:
    //   * Flat: ["{stem}.jsonl"] — prepend today's YYYY_MM_DD.
    //   * Legacy: ["DDMMYYYY", "{stem}.jsonl"] — keep the existing
    //     date dir so we don't relabel old transcripts.
    let tail = &components[raw_idx + 1..];
    if tail.len() <= 1 {
        out.push(chrono::Local::now().format("%Y_%m_%d").to_string());
    }
    for comp in tail {
        out.push(comp.as_os_str());
    }

    out.with_extension("md")
}

fn sanitize_agent_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Compute the next free index for `agent_prefix` in `dir`.
///
/// Considers both `.jsonl` and `.md` files so that indices stay unique
/// during the one-release migration window when both extensions may exist.
fn next_index(dir: &Path, agent_prefix: &str) -> Result<usize> {
    let prefix = format!("{}_", agent_prefix);
    let mut max_idx: Option<usize> = None;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(&prefix) {
                continue;
            }
            // Accept both extensions.
            let stem_end = if name.ends_with(".jsonl") {
                name.len() - 6
            } else if name.ends_with(".md") {
                name.len() - 3
            } else {
                continue;
            };
            let idx_str = &name[prefix.len()..stem_end];
            if let Ok(idx) = idx_str.parse::<usize>() {
                max_idx = Some(max_idx.map_or(idx, |m: usize| m.max(idx)));
            }
        }
    }

    Ok(max_idx.map_or(0, |m| m + 1))
}

/// Find the latest transcript file for `agent_prefix` in `dir`.
///
/// Prefers `.jsonl` files; falls back to `.md` if no `.jsonl` exists
/// (legacy sessions). When both exist for the same index the `.jsonl`
/// wins.
fn latest_in_dir(dir: &Path, agent_prefix: &str) -> Option<PathBuf> {
    // Two transcript-naming schemes coexist on disk:
    //   * Legacy: `{agent}_{index}.jsonl|.md` — strictly increasing
    //     index, used by the now-removed `resolve_new_transcript_path`.
    //   * Keyed: `{unix_ts}_{agent}.jsonl` (root session) or
    //     `{parent_chain}__{unix_ts}_{agent}.jsonl` (sub-agent). The
    //     root stem starts with `{unix_ts}_{agent}` and has no `__`
    //     prefix segment.
    //
    // For resume we only care about root sessions (sub-agents rebuild
    // from scratch), so we scan for filenames matching either scheme
    // and pick the newest. "Newest" is the largest sort key — indices
    // and unix timestamps both order naturally as integers.
    let legacy_prefix = format!("{}_", agent_prefix);
    let keyed_suffix = format!("_{}", agent_prefix);
    let mut best_jsonl: Option<(u64, PathBuf)> = None;
    let mut best_md: Option<(u64, PathBuf)> = None;

    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Extract the stem minus extension.
        let (stem, is_jsonl) = if let Some(s) = name_str.strip_suffix(".jsonl") {
            (s, true)
        } else if let Some(s) = name_str.strip_suffix(".md") {
            (s, false)
        } else {
            continue;
        };
        // Skip sub-agent transcripts — they carry at least one `__`
        // separator in their stem (e.g.
        // `{orch_key}__{planner_key}`). Root resume never targets a
        // sub-agent's transcript directly.
        if stem.contains("__") {
            continue;
        }
        // Determine sort key. Keyed filenames end with
        // `_{agent_prefix}`: everything before that is the unix
        // timestamp. Legacy filenames start with `{agent_prefix}_`:
        // everything after is the numeric index.
        let sort_key: u64 = if let Some(ts_part) = stem.strip_suffix(&keyed_suffix) {
            match ts_part.parse::<u64>() {
                Ok(ts) => ts,
                Err(_) => continue,
            }
        } else if let Some(idx_part) = stem.strip_prefix(&legacy_prefix) {
            match idx_part.parse::<u64>() {
                Ok(idx) => idx,
                Err(_) => continue,
            }
        } else {
            continue;
        };
        let slot = if is_jsonl {
            &mut best_jsonl
        } else {
            &mut best_md
        };
        if slot.as_ref().is_none_or(|(best, _)| sort_key > *best) {
            *slot = Some((sort_key, entry.path()));
        }
    }

    // Prefer the best .jsonl; fall back to .md if no .jsonl exists.
    match (best_jsonl, best_md) {
        (Some(jsonl), Some(md)) => {
            // Take the one with the higher index; on a tie prefer .jsonl.
            if md.0 > jsonl.0 {
                Some(md.1)
            } else {
                Some(jsonl.1)
            }
        }
        (Some(jsonl), None) => Some(jsonl.1),
        (None, Some(md)) => Some(md.1),
        (None, None) => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
