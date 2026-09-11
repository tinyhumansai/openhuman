use crate::openhuman::agent::messages::ChatMessage;
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
    let message_reasoning = (msg.role == "assistant")
        .then(|| {
            extra_metadata
                .as_ref()
                .and_then(|meta| {
                    meta.get(crate::openhuman::agent::message_convert::REASONING_EXT_KEY)
                })
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .flatten();
    let native_envelope = (msg.role == "assistant")
        .then(|| serde_json::from_str::<serde_json::Value>(&msg.content).ok())
        .flatten();
    let envelope_reasoning = native_envelope
        .as_ref()
        .and_then(|value| value.get("reasoning_content"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let envelope_tool_calls = native_envelope
        .as_ref()
        .and_then(|value| value.get("tool_calls"))
        .and_then(|value| serde_json::from_value::<Vec<ToolCall>>(value.clone()).ok())
        .filter(|calls| !calls.is_empty());
    MessageLine {
        id: msg.id.clone(),
        role: msg.role.clone(),
        content: msg.content.clone(),
        extra_metadata,
        provider: assistant_usage.map(|tu| tu.provider.clone()),
        model: assistant_usage.map(|tu| tu.model.clone()),
        usage: assistant_usage.map(|tu| tu.usage.clone()),
        reasoning_content: message_reasoning
            .or(envelope_reasoning)
            .or_else(|| assistant_usage.and_then(|tu| tu.reasoning_content.clone())),
        tool_calls: envelope_tool_calls.or_else(|| {
            assistant_usage.and_then(|tu| {
                if tu.tool_calls.is_empty() {
                    None
                } else {
                    Some(tu.tool_calls.clone())
                }
            })
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
