
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

/// Find the newest root transcript whose metadata declares `thread_id`, across
/// the shared `session_raw/` store and every profile-scoped
/// `session_raw-<id>/` store.
///
/// Root transcripts live directly under `session_raw/` and do not carry
/// the `__` separator used for sub-agent siblings. This helper is the
/// bridge PR-2 can use to route UI thread reads to the canonical root
/// transcript without accidentally folding delegated worker transcripts
/// into the main chat timeline.
pub fn find_root_transcript_for_thread(workspace_dir: &Path, thread_id: &str) -> Option<PathBuf> {
    find_root_transcripts_for_thread(workspace_dir, thread_id).pop()
}

pub fn find_root_transcripts_for_thread(
    workspace_dir: &Path,
    thread_id: &str,
) -> Vec<PathBuf> {
    let mut matches = Vec::new();
    for raw_dir in raw_session_dirs(workspace_dir) {
        matches.extend(root_transcripts_for_thread_in_dir(&raw_dir, thread_id));
    }
    // Already chronological within each directory (see
    // `root_transcripts_for_thread_in_dir`); re-key across directories on the
    // same `created` stamp rather than the file name.
    matches.sort_by_cached_key(|path| {
        let created = read_transcript(path)
            .ok()
            .map(|transcript| transcript.meta.created)
            .unwrap_or_default();
        (created, path.clone())
    });
    matches
}

fn raw_session_dirs(workspace_dir: &Path) -> Vec<PathBuf> {
    let mut raw_dirs = vec![raw_session_dir(workspace_dir)];
    if let Ok(entries) = fs::read_dir(workspace_dir) {
        raw_dirs.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.strip_prefix("session_raw-")
                            .is_some_and(|suffix| !suffix.is_empty())
                    })
        }));
    }
    raw_dirs.sort();
    raw_dirs
}

pub fn find_root_transcript_for_thread_in_dir(raw_dir: &Path, thread_id: &str) -> Option<PathBuf> {
    root_transcripts_for_thread_in_dir(raw_dir, thread_id).pop()
}

fn root_transcripts_for_thread_in_dir(raw_dir: &Path, thread_id: &str) -> Vec<PathBuf> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return Vec::new();
    }

    let Ok(entries) = fs::read_dir(raw_dir) else {
        return Vec::new();
    };
    // Keyed by `meta.created` so the order is chronological rather than
    // lexicographic. Modern stems are `{unix_ts}_{agent_id}` and sort the same
    // either way, but a legacy `{agent}_{index}` root encodes no time at all —
    // and because digits sort before letters, every legacy root sorted *after*
    // every modern one regardless of when it was written. `project_from_files`
    // concatenates these in order, so that reordered the rendered view and
    // could attach a sub-agent trail to the wrong turn.
    let mut matches: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|s| s.to_str()) == Some("jsonl")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|stem| !stem.contains("__"))
        })
        .filter_map(|path| match read_transcript(&path) {
            Ok(transcript) if transcript.meta.thread_id.as_deref() == Some(thread_id) => {
                Some((transcript.meta.created.clone(), path))
            }
            Ok(_) => None,
            Err(err) => {
                log::warn!(
                    "[transcript] skipping unreadable root transcript candidate {}: {err}",
                    path.display()
                );
                None
            }
        })
        .collect();

    // Path is the tiebreak so the order stays total and deterministic when two
    // transcripts share a `created` stamp.
    matches.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    matches.into_iter().map(|(_, path)| path).collect()
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
