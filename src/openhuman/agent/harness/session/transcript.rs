//! Session transcript persistence for KV cache stability.
//!
//! **Source of truth**: `session_raw/{stem}.jsonl` — a *flat* directory.
//!
//! Each JSONL file starts with a single metadata line (identified by an
//! `_meta` key) followed by one JSON object per `ChatMessage`. On every
//! write the companion `.md` file is re-rendered for human readability
//! under `sessions/{YYYY_MM_DD}/{stem}.md`; it is **never** read back —
//! all round-trip / resume logic uses the JSONL.
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
//! {"_meta":{"agent":"code_executor","dispatcher":"native","created":"...","updated":"...","turn_count":3,"input_tokens":5000,"output_tokens":1200,"cached_input_tokens":3500,"charged_amount_usd":0.0045}}
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
//! Unknown fields on read are ignored (forward-compat).

use crate::openhuman::providers::ChatMessage;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    pub cost_usd: f64,
}

/// Usage + provenance for one provider response, attached to the last
/// assistant message in a turn.
#[derive(Debug, Clone)]
pub struct TurnUsage {
    pub model: String,
    pub usage: MessageUsage,
    /// RFC-3339 timestamp of the response.
    pub ts: String,
}

/// Metadata header for a session transcript file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptMeta {
    pub agent_name: String,
    pub dispatcher: String,
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
    agent: String,
    dispatcher: String,
    created: String,
    updated: String,
    turn_count: usize,
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    charged_amount_usd: f64,
}

/// One message line in the JSONL — only `role` and `content` are required.
/// All other fields are optional; unknown fields are flattened to preserve
/// forward-compatibility.
#[derive(Serialize, Deserialize)]
struct MessageLine {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<MessageUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ts: Option<String>,
    /// Absorb any unknown fields so forward-compat reads don't error.
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

// ── Write ─────────────────────────────────────────────────────────────

/// Write JSONL as source of truth **and** re-render the companion `.md`.
///
/// `jsonl_path` must end in `.jsonl`; the `.md` companion is derived by
/// swapping the extension. Full rewrite on every call (not append) so
/// that context-reduction that removed earlier messages is reflected
/// immediately.
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

    // Line 1: meta header.
    let meta_line = MetaLine {
        meta: MetaPayload {
            agent: meta.agent_name.clone(),
            dispatcher: meta.dispatcher.clone(),
            created: meta.created.clone(),
            updated: meta.updated.clone(),
            turn_count: meta.turn_count,
            input_tokens: meta.input_tokens,
            output_tokens: meta.output_tokens,
            cached_input_tokens: meta.cached_input_tokens,
            charged_amount_usd: meta.charged_amount_usd,
        },
    };
    let meta_json =
        serde_json::to_string(&meta_line).context("serialise transcript meta header")?;
    jsonl_buf.push_str(&meta_json);
    jsonl_buf.push('\n');

    // Identify the index of the last assistant message so we can attach
    // per-turn usage to it.
    let last_assistant_idx = messages.iter().rposition(|m| m.role == "assistant");

    for (i, msg) in messages.iter().enumerate() {
        // Only the last assistant message carries usage/model/ts; every
        // other line has those fields omitted. Pattern-match both
        // options together so there's no separate unwrap.
        let line = match (last_assistant_idx, last_assistant_turn_usage) {
            (Some(idx), Some(tu)) if idx == i => MessageLine {
                role: msg.role.clone(),
                content: msg.content.clone(),
                model: Some(tu.model.clone()),
                usage: Some(tu.usage.clone()),
                ts: Some(tu.ts.clone()),
                _extra: HashMap::new(),
            },
            _ => MessageLine {
                role: msg.role.clone(),
                content: msg.content.clone(),
                model: None,
                usage: None,
                ts: None,
                _extra: HashMap::new(),
            },
        };

        let line_json =
            serde_json::to_string(&line).with_context(|| format!("serialise message line {i}"))?;
        jsonl_buf.push_str(&line_json);
        jsonl_buf.push('\n');
    }

    fs::write(jsonl_path, jsonl_buf.as_bytes())
        .with_context(|| format!("write transcript {}", jsonl_path.display()))?;

    log::debug!(
        "[transcript] wrote {} messages (jsonl) to {}",
        messages.len(),
        jsonl_path.display()
    );

    // ── Companion .md ────────────────────────────────────────────────
    // Build per-message usage index for the renderer (only last assistant).
    let mut per_msg_usage: HashMap<usize, &TurnUsage> = HashMap::new();
    if let (Some(idx), Some(tu)) = (last_assistant_idx, last_assistant_turn_usage) {
        per_msg_usage.insert(idx, tu);
    }

    // The .md companion is a *derived* view — the JSONL above is the
    // source of truth. Failures here must not propagate: a readable-log
    // hiccup shouldn't take down the session's state persistence. Log
    // and move on.
    let md_path = md_companion_path(jsonl_path);
    if let Some(parent) = md_path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            log::warn!(
                "[transcript] failed to create md companion dir {}: {err}",
                parent.display()
            );
            return Ok(());
        }
    }
    let md = render_markdown(messages, meta, &per_msg_usage);
    if let Err(err) = fs::write(&md_path, md.as_bytes()) {
        log::warn!(
            "[transcript] failed to write markdown companion {}: {err}",
            md_path.display()
        );
        return Ok(());
    }

    log::debug!(
        "[transcript] wrote markdown companion to {}",
        md_path.display()
    );

    Ok(())
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

fn read_transcript_jsonl(path: &Path) -> Result<SessionTranscript> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read transcript jsonl {}", path.display()))?;

    let mut meta: Option<TranscriptMeta> = None;
    let mut messages: Vec<ChatMessage> = Vec::new();

    // The JSONL format is positional: line 1 (the first non-empty line)
    // is the `_meta` header; every subsequent non-empty line is a message.
    // This avoids a substring check that could false-positive if message
    // content contains `"_meta"`.
    for (line_no, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if meta.is_none() {
            let ml: MetaLine = serde_json::from_str(line).map_err(|err| {
                anyhow::anyhow!(
                    "first non-empty line of {} (line {}) is not a valid _meta object: {err}",
                    path.display(),
                    line_no + 1,
                )
            })?;
            let mp = ml.meta;
            meta = Some(TranscriptMeta {
                agent_name: mp.agent,
                dispatcher: mp.dispatcher,
                created: mp.created,
                updated: mp.updated,
                turn_count: mp.turn_count,
                input_tokens: mp.input_tokens,
                output_tokens: mp.output_tokens,
                cached_input_tokens: mp.cached_input_tokens,
                charged_amount_usd: mp.charged_amount_usd,
            });
            continue;
        }

        // Message line.
        match serde_json::from_str::<MessageLine>(line) {
            Ok(ml) => {
                messages.push(ChatMessage {
                    role: ml.role,
                    content: ml.content,
                });
            }
            Err(err) => {
                log::warn!(
                    "[transcript] skipping malformed message line {} in {}: {err}",
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
        "[transcript] loaded {} messages (jsonl) from {}",
        messages.len(),
        path.display()
    );

    Ok(SessionTranscript { meta, messages })
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
                role,
                content: content.replace(LEGACY_MSG_CLOSE_ESCAPED, LEGACY_MSG_CLOSE),
            });
            search_from = content_start + content_end_rel + LEGACY_MSG_CLOSE.len();
            continue;
        };

        let content = &raw[content_start..content_start + content_end_rel];
        messages.push(ChatMessage {
            role,
            content: content.replace(LEGACY_MSG_CLOSE_ESCAPED, LEGACY_MSG_CLOSE),
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

    let raw_idx = components.iter().position(
        |comp| matches!(comp, std::path::Component::Normal(s) if *s == "session_raw"),
    );

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
