
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

    // Single scan: split the thread's transcripts into root (orchestrator) and
    // `__` sub-agent files. Root totals stay the parent's; sub-agent files are
    // grouped by archetype for the per-agent breakdown.
    let mut root_matches: Vec<PathBuf> = Vec::new();
    let mut sub_matches: Vec<PathBuf> = Vec::new();
    for raw_dir in raw_session_dirs(workspace_dir) {
        let Ok(entries) = fs::read_dir(&raw_dir) else {
            continue;
        };
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
    }

    if root_matches.is_empty() && sub_matches.is_empty() {
        return None;
    }
    root_matches.sort_by(|left, right| left.file_name().cmp(&right.file_name()));

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
    resolve_keyed_transcript_path_in_dir(&raw_dir, stem)
}

pub fn resolve_keyed_transcript_path_in_dir(raw_dir: &Path, stem: &str) -> Result<PathBuf> {
    fs::create_dir_all(raw_dir)
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
    find_latest_transcript_in_subdir(workspace_dir, "session_raw", agent_name)
}

/// Find the most recent transcript inside a session's configured raw subtree.
/// Scoped profile sessions must never fall back to shared transcripts; the
/// legacy date-grouped/markdown fallback applies only to `session_raw`.
pub fn find_latest_transcript_in_subdir(
    workspace_dir: &Path,
    session_raw_subdir: &str,
    agent_name: &str,
) -> Option<PathBuf> {
    let sanitized = sanitize_agent_name(agent_name);
    let raw_root = workspace_dir.join(session_raw_subdir);
    let sessions_root = workspace_dir.join("sessions");

    // Primary path: flat session_raw/ directory. The stem-suffix scan
    // is naturally date-independent, so an idle thread resumes the same
    // way today as it did weeks ago.
    if raw_root.is_dir() {
        if let Some(path) = latest_in_dir(&raw_root, &sanitized) {
            return Some(path);
        }
    }

    if session_raw_subdir != "session_raw" {
        return None;
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
