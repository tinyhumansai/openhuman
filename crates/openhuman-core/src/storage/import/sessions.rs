//! Sessions, transcripts, turn states and the run ledger.
//!
//! Before storage ports, workspaces kept history and ledger rows in SQLite,
//! transcripts under `session_raw/`, and turn snapshots under
//! `memory/conversations/turn_states/`. OpenHuman imports their decoded rows
//! through TinyAgents' current ports; readers are read-only and id-preserving,
//! and only-if-absent. This module handles:
//!
//! - **Quiescence.** The importer refuses a database with a WAL or journal
//!   sidecar, so `sessions.db` is first copied to a consistent scratch
//!   snapshot ([`Snapshot`], `VACUUM INTO`); the real file is only ever
//!   opened read-only.
//! - **Whose sessions.** The workspace belongs to the operator. The run
//!   ledger and the turn states land in the operator's default agent
//!   ([`DEFAULT_AGENT`](crate::storage::sessions::DEFAULT_AGENT)), where
//!   the desktop's own work resolves them. A transcript stem names the agent
//!   definition that wrote it, but only as a digest, so each stem is matched
//!   against the digests of the known definitions
//!   ([`crate::agent_registry::host_definition::AgentDefinitionRegistry`]) and
//!   imported into that definition's scope, which is where the session host
//!   resumes it. A stem that matches none lands in the default agent.
//! - **Once.** A marker (like the other domains') is written when the whole
//!   import finished without skipping a record; a partial run is retried at
//!   the next boot and, being only-if-absent, never duplicates.
//!
//! Legacy files are never modified or deleted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tinyagents_session::port::{SessionStoreProvider, TurnStates};
use tinyagents_session::run_ledger::history::{SessionHistoryImport, SessionHistoryStore};
use tinyagents_session::run_ledger::RunLedgerStore;
use tinyagents_session::run_ledger::{AgentFamilyImport, ImportCounts, LegacyLedgerRows};
use tinyagents_session::transcript::{
    session_stem, DisplayMessage, DisplayRecord, SessionRef, TranscriptHistory, TranscriptMessage,
    TranscriptMeta, TranscriptPartial, TranscriptTurn,
};
use tinyagents_session::turn_state::TurnState;
use tinyagents_session::DriverSessionStores;

use super::tinyflows::{finish, marked};
use super::{Outcome, Report, Snapshot};
use crate::config::Config;
use crate::storage::sessions::DEFAULT_AGENT;
use crate::storage::StorageBackend;

mod agents;
pub(super) use agents::{agent_of, known_agents};

/// The marker name of the session import.
pub const SESSIONS_DOMAIN: &str = "sessions";

/// `session_db/sessions.db`, relative to the workspace.
const SESSIONS_DB: &str = "session_db/sessions.db";

/// Runs the session import for `config`'s workspace.
pub fn run_with(backend: &Arc<dyn StorageBackend>, config: &Config) -> Vec<Report> {
    vec![finish(SESSIONS_DOMAIN, import(backend, config))]
}

fn import(backend: &Arc<dyn StorageBackend>, config: &Config) -> Result<Outcome> {
    let workspace = config.workspace_dir.clone();
    let db = workspace.join(SESSIONS_DB);
    let raw = workspace.join("session_raw");
    let turns = workspace
        .join("memory")
        .join("conversations")
        .join("turn_states");
    marked(backend, &workspace, SESSIONS_DOMAIN, |condition| {
        if !db.is_file() && !raw.is_dir() && !turns.is_dir() {
            return Ok(None);
        }
        let claimed_backend = super::super::import_state::backend_with_condition(
            Arc::clone(backend),
            condition.clone(),
        );
        let factory: tinyagents_session::port::AgentStorageFactory = Arc::new(move |agent_id| {
            let scope = crate::storage::scope_for_agent(agent_id);
            let storage = claimed_backend.for_scope(&scope)?;
            Ok(tinyagents_storage_kit::ScopedStorage::new(
                storage,
                (*scope).clone(),
            ))
        });
        let provider = DriverSessionStores::new(factory)
            .map_err(|error| anyhow::anyhow!("start the session store bridge: {error}"))?
            .recover_on_open(false);
        let mut found = 0usize;
        let mut written = 0usize;
        let mut skipped = 0usize;

        if db.is_file() {
            let snapshot = Snapshot::of(&db, SESSIONS_DB)
                .context("[sessions] copy sessions.db for reading")?;
            let mut rows = super::session_db::SessionDatabase::read(snapshot.db())
                .context("[sessions] read the session database snapshot")?;
            sanitize_legacy_system_messages(&mut rows.messages);
            let fingerprint =
                source_fingerprint((&rows.agent_runs, &rows.events, &rows.telemetry))?;
            let history_batch = SessionHistoryImport {
                sessions: rows.sessions,
                messages: rows.messages,
                tool_calls: rows.tool_calls,
            };
            let ledger = provider
                .driver_run_ledger(DEFAULT_AGENT)
                .map_err(|error| anyhow::anyhow!("open the run ledger: {error}"))?;
            let history = crate::storage::block_on_anyhow(async move {
                ledger
                    .import_session_history(history_batch)
                    .await
                    .map_err(|error| anyhow::anyhow!("{error}"))
            })
            .context("[sessions] import classic session history")?;

            let agent_batch = AgentFamilyImport {
                operation_id: workspace_operation_id(&workspace),
                source_fingerprint: fingerprint,
                runs: rows.agent_runs,
                events: rows.events,
                telemetry: rows.telemetry,
            };
            let ledger = provider
                .driver_run_ledger(DEFAULT_AGENT)
                .map_err(|error| anyhow::anyhow!("open the run ledger: {error}"))?;
            let agent = crate::storage::block_on_anyhow(async move {
                ledger
                    .import_agent_family(agent_batch)
                    .await
                    .map_err(|error| anyhow::anyhow!("{error}"))
            })
            .context("[sessions] import classic agent run family")?;

            let legacy_batch = LegacyLedgerRows {
                workflow_runs: rows.workflows,
                teams: rows.teams,
                members: rows.members,
                tasks: rows.tasks,
                tool_effects: rows.tool_effects,
            };
            let ledger = provider
                .driver_run_ledger(DEFAULT_AGENT)
                .map_err(|error| anyhow::anyhow!("open the run ledger: {error}"))?;
            let legacy = crate::storage::block_on_anyhow(async move {
                ledger
                    .import_legacy_rows(legacy_batch)
                    .await
                    .map_err(|error| anyhow::anyhow!("{error}"))
            })
            .context("[sessions] import classic ledger rows")?;
            let (database_found, database_written) = database_totals([
                history.sessions,
                history.messages,
                history.tool_calls,
                agent.agent_runs,
                agent.events,
                agent.telemetry,
                legacy.workflow_runs,
                legacy.teams,
                legacy.members,
                legacy.tasks,
                legacy.tool_effects,
            ]);
            found += database_found;
            written += database_written;
        }

        if raw.is_dir() {
            let (f, w, s) = import_transcript_groups(&provider, &workspace)?;
            found += f;
            written += w;
            skipped += s;
        }

        if turns.is_dir() {
            let stores = provider
                .try_for_agent(DEFAULT_AGENT)
                .map_err(|error| anyhow::anyhow!("open the turn states: {error}"))?;
            let report = import_turn_states(&turns, stores.turn_states.as_ref())
                .context("[sessions] import turn states")?;
            found += report.read;
            written += report.imported;
            skipped += report.unreadable;
        }
        Ok(Some((found, written, skipped)))
    })
}

/// The destination ports report source rows per stored family. FTS rows are a
/// derived index and do not represent additional legacy records.
fn database_totals(counts: [ImportCounts; 11]) -> (usize, usize) {
    counts.into_iter().fold(
        (0, 0),
        |(found, written), ImportCounts { read, imported, .. }| (found + read, written + imported),
    )
}

fn source_fingerprint<T: serde::Serialize>(rows: T) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(&rows).context("serialize decoded legacy agent rows")?;
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
}

fn workspace_operation_id(workspace: &Path) -> String {
    use sha2::{Digest, Sha256};
    let identity = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    format!(
        "workspace-sessions-v1:{}",
        hex::encode(Sha256::digest(identity.to_string_lossy().as_bytes()))
    )
}

fn sanitize_legacy_system_messages(messages: &mut [tinyagents_session::types::SessionMessage]) {
    for message in messages {
        if message.role == "system" {
            if let Some(cleaned) =
                crate::config::migrations::phase_out_profile_md::strip_profile_md_block(
                    &message.content,
                )
            {
                message.content = cleaned;
            }
        }
    }
}

#[derive(Default)]
struct TurnStateReport {
    read: usize,
    imported: usize,
    unreadable: usize,
}

/// Imports old per-turn snapshots without changing the workspace files. The
/// flat legacy form is retained alongside the newer per-thread directories.
fn import_turn_states(dir: &Path, destination: &dyn TurnStates) -> Result<TurnStateReport> {
    let mut report = TurnStateReport::default();
    for path in turn_state_files(dir)? {
        let state = match std::fs::read(&path)
            .with_context(|| format!("read turn-state snapshot {}", path.display()))
            .and_then(|bytes| serde_json::from_slice::<TurnState>(&bytes).map_err(Into::into))
        {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(
                    "[sessions] skipping unreadable turn state {}: {error}",
                    path.display()
                );
                report.unreadable += 1;
                continue;
            }
        };
        report.read += 1;
        if destination
            .get_turn(&state.thread_id, &state.request_id)
            .map_err(anyhow::Error::msg)?
            .is_none()
        {
            destination.put(&state).map_err(anyhow::Error::msg)?;
            report.imported += 1;
        }
    }
    Ok(report)
}

fn turn_state_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if std::fs::symlink_metadata(dir)?.file_type().is_symlink() {
        anyhow::bail!("turn-state root {} is a symbolic link", dir.display());
    }
    let mut files = Vec::new();
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("list turn-state directory {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let mut children = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
            children.sort_by_key(std::fs::DirEntry::file_name);
            for child in children {
                let path = child.path();
                if child.file_type()?.is_file()
                    && path.extension().and_then(|extension| extension.to_str()) == Some("json")
                {
                    files.push(path);
                }
            }
        } else if file_type.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("json")
        {
            files.push(path);
        }
    }
    Ok(files)
}

fn decode_transcript(
    path: &Path,
) -> Result<(
    TranscriptMeta,
    Option<serde_json::Value>,
    Vec<DisplayRecord>,
)> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read transcript {}", path.display()))?;
    let text = std::str::from_utf8(&bytes)
        .with_context(|| format!("decode transcript {} as UTF-8", path.display()))?;
    let mut meta = None;
    let mut tools = None;
    let mut records = Vec::new();
    let mut sanitized_system = false;
    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("parse transcript {} line {}", path.display(), line_no + 1))?;
        if let Some(header) = value.get("_meta") {
            let mut header = header.clone();
            if let Some(fields) = header.as_object_mut() {
                if !fields.contains_key("agent_name") {
                    if let Some(agent) = fields.get("agent").cloned() {
                        fields.insert("agent_name".into(), agent);
                    }
                }
            }
            meta =
                Some(serde_json::from_value(header).with_context(|| {
                    format!("decode transcript metadata in {}", path.display())
                })?);
            continue;
        }
        if value.get("kind").and_then(serde_json::Value::as_str) == Some("tools") {
            tools = value.get("tools").cloned();
            continue;
        }
        if value.get("kind").and_then(serde_json::Value::as_str) == Some("compaction") {
            let replacement = value
                .get("replacement")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|row| decode_display_message(row, &mut sanitized_system).ok())
                .collect();
            records.push(DisplayRecord::Compaction(
                tinyagents_session::transcript::CompactionMarker {
                    replacement,
                    ts: value
                        .get("ts")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    request_id: value
                        .get("request_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                },
            ));
            continue;
        }
        if let Ok(message) = decode_display_message(&value, &mut sanitized_system) {
            records.push(DisplayRecord::Message(Box::new(message)));
        }
    }
    let meta =
        meta.ok_or_else(|| anyhow::anyhow!("transcript {} has no _meta header", path.display()))?;
    Ok((meta, tools, records))
}

fn decode_display_message(
    value: &serde_json::Value,
    sanitized_system: &mut bool,
) -> Result<DisplayMessage> {
    // The typed transcript row shape was introduced in row version 2.
    // Keep this importer compatible with older TinyAgents pins that do not
    // expose the format version constant publicly.
    const TYPED_ROW_VERSION: u32 = 2;
    let mut message: TranscriptMessage =
        serde_json::from_value(value.clone()).context("decode transcript message")?;
    let row_version = value
        .get("v")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default() as u32;
    let usage_tool_calls = message.tool_calls.clone();
    if row_version != TYPED_ROW_VERSION {
        message.tool_calls.clear();
        message.tool_call_id = None;
        message.parts = None;
        if row_version < TYPED_ROW_VERSION {
            message = message.normalized();
        }
    }
    if message.role.is_empty() {
        anyhow::bail!("transcript message has no role");
    }
    if message.role == "system" && !*sanitized_system {
        *sanitized_system = true;
        if let Some(cleaned) =
            crate::config::migrations::phase_out_profile_md::strip_profile_md_block(
                &message.content,
            )
        {
            message.content = cleaned;
        }
    }
    let turn_usage = match (
        value.get("provider").and_then(serde_json::Value::as_str),
        value.get("model").and_then(serde_json::Value::as_str),
        value.get("usage"),
        value.get("ts").and_then(serde_json::Value::as_str),
    ) {
        (Some(provider), Some(model), Some(usage), Some(ts)) if message.role == "assistant" => {
            serde_json::from_value(usage.clone()).ok().map(|usage| {
                tinyagents_session::transcript::TurnUsage {
                    provider: provider.to_owned(),
                    model: model.to_owned(),
                    usage,
                    ts: ts.to_owned(),
                    reasoning_content: value
                        .get("reasoning_content")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    tool_calls: usage_tool_calls,
                    iteration: value
                        .get("iteration")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default() as u32,
                }
            })
        }
        _ => None,
    };
    message.turn_usage = turn_usage.clone();
    message.preserve_request_id = true;
    let failed = value
        .get("failure")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let failure_detail = value
        .get("failure_detail")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if failed {
        message.tool_failure = Some(tinyagents_session::transcript::ToolFailure {
            failed,
            detail: failure_detail.clone(),
        });
    }
    let reasoning_content = value
        .get("reasoning_content")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            turn_usage
                .as_ref()
                .and_then(|usage| usage.reasoning_content.clone())
        });
    Ok(DisplayMessage {
        interrupted: value
            .get("interrupted")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        request_id: value
            .get("request_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        iteration: value
            .get("iteration")
            .and_then(serde_json::Value::as_u64)
            .map(|iteration| iteration as u32),
        ts: value
            .get("ts")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        reasoning_content,
        turn_usage,
        failure: failed,
        failure_detail,
        background: value
            .get("background")
            .cloned()
            .and_then(|origin| serde_json::from_value(origin).ok()),
        message,
    })
}

fn stored_transcript_row(display: &DisplayMessage) -> TranscriptMessage {
    let mut row = display.message.clone();
    if let Some(origin) = &display.background {
        let mut metadata = match row.extra_metadata.take() {
            Some(serde_json::Value::Object(metadata)) => metadata,
            Some(value) => serde_json::Map::from_iter([("value".into(), value)]),
            None => serde_json::Map::new(),
        };
        metadata.insert(
            tinyagents_session::transcript::BACKGROUND_METADATA_KEY.into(),
            serde_json::to_value(origin).unwrap_or(serde_json::Value::Null),
        );
        row.extra_metadata = Some(serde_json::Value::Object(metadata));
    }
    row
}

fn replay_transcript(
    history: &dyn TranscriptHistory,
    meta: &TranscriptMeta,
    tools: Option<&serde_json::Value>,
    records: Vec<DisplayRecord>,
) -> Result<usize> {
    let mut logical = history
        .read_session()?
        .map(|session| session.messages)
        .unwrap_or_default();
    let mut group = Vec::<DisplayMessage>::new();
    let mut replayed = 0usize;
    let flush = |logical: &mut Vec<TranscriptMessage>,
                 group: &mut Vec<DisplayMessage>,
                 replayed: &mut usize|
     -> Result<()> {
        if group.is_empty() {
            return Ok(());
        }
        let mut next = logical.clone();
        next.extend(group.iter().map(stored_transcript_row));
        let usage = group
            .iter()
            .rev()
            .find_map(|message| message.turn_usage.as_ref());
        let request_id = group
            .iter()
            .rev()
            .find_map(|message| message.request_id.as_deref());
        history.append_turn(TranscriptTurn {
            prev: logical,
            next: &next,
            meta,
            turn_usage: usage,
            request_id,
            tools: None,
        })?;
        *replayed += group.len();
        *logical = next;
        group.clear();
        Ok(())
    };
    for record in records {
        match record {
            DisplayRecord::Message(message) if message.interrupted => {
                flush(&mut logical, &mut group, &mut replayed)?;
                let partial = TranscriptPartial {
                    content: message.message.content.clone(),
                    reasoning_content: message.reasoning_content.clone(),
                    iteration: message.iteration,
                };
                history.append_turn_with_partial(
                    TranscriptTurn {
                        prev: &logical,
                        next: &logical,
                        meta,
                        turn_usage: None,
                        request_id: message.request_id.as_deref(),
                        tools: None,
                    },
                    Some(&partial),
                )?;
            }
            DisplayRecord::Message(message) => {
                if group
                    .last()
                    .is_some_and(|last| last.request_id != message.request_id)
                {
                    flush(&mut logical, &mut group, &mut replayed)?;
                }
                group.push(*message);
            }
            DisplayRecord::Compaction(marker) => {
                flush(&mut logical, &mut group, &mut replayed)?;
                let usage = marker
                    .replacement
                    .iter()
                    .rev()
                    .find_map(|message| message.turn_usage.clone());
                let request_id = marker.request_id.clone();
                let next = marker
                    .replacement
                    .into_iter()
                    .map(|message| stored_transcript_row(&message))
                    .collect::<Vec<_>>();
                history.append_turn(TranscriptTurn {
                    prev: &logical,
                    next: &next,
                    meta,
                    turn_usage: usage.as_ref(),
                    request_id: request_id.as_deref(),
                    tools: None,
                })?;
                replayed += next.len();
                logical = next;
            }
        }
    }
    flush(&mut logical, &mut group, &mut replayed)?;
    if let Some(tools) = tools {
        history.append_turn(TranscriptTurn {
            prev: &logical,
            next: &logical,
            meta,
            turn_usage: None,
            request_id: None,
            tools: Some(tools),
        })?;
    }
    Ok(replayed)
}

/// Every `*.jsonl` under `raw` (flat and date-grouped), flat winning a
/// duplicate stem, as `(stem, path)`.
fn transcript_files(raw: &Path) -> Result<Vec<(String, PathBuf)>> {
    if std::fs::symlink_metadata(raw)?.file_type().is_symlink() {
        anyhow::bail!("transcript root {} is a symbolic link", raw.display());
    }
    let mut flat = Vec::new();
    let mut dated = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(raw)
        .with_context(|| format!("read {}", raw.display()))?
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let mut inner: Vec<_> = std::fs::read_dir(&path)?.collect::<std::io::Result<_>>()?;
            inner.sort_by_key(std::fs::DirEntry::file_name);
            for entry in inner {
                if entry.file_type()?.is_file() {
                    push_jsonl(&mut dated, entry.path());
                }
            }
        } else if file_type.is_file() {
            push_jsonl(&mut flat, path);
        }
    }
    let seen: std::collections::HashSet<String> =
        flat.iter().map(|(stem, _)| stem.clone()).collect();
    flat.extend(dated.into_iter().filter(|(stem, _)| !seen.contains(stem)));
    Ok(flat)
}

fn push_jsonl(out: &mut Vec<(String, PathBuf)>, path: PathBuf) {
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return;
    }
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        out.push((stem.to_owned(), path));
    }
}

/// Imports the transcripts, each into the scope of the agent its stem names.
/// Returns `(found, written, skipped)`.
pub(crate) fn import_transcript_groups(
    provider: &dyn SessionStoreProvider,
    workspace: &Path,
) -> Result<(usize, usize, usize)> {
    let raw = workspace.join("session_raw");
    let known = known_agents();
    let mut groups: HashMap<String, Vec<(String, PathBuf)>> = HashMap::new();
    for (stem, path) in transcript_files(&raw)? {
        let agent = agent_of(&stem, &known).unwrap_or_else(|| DEFAULT_AGENT.to_owned());
        groups.entry(agent).or_default().push((stem, path));
    }
    let (mut found, mut written, mut skipped) = (0, 0, 0);
    for (agent, files) in groups {
        let stores = provider
            .try_for_agent(&agent)
            .map_err(|error| anyhow::anyhow!("open the transcripts of {agent}: {error}"))?;
        for (stem, path) in files {
            let stem_names_known_agent = agent_of(&stem, &known).is_some();
            let (meta, tools, records) = match decode_transcript(&path) {
                Ok(decoded) => decoded,
                Err(error) => {
                    tracing::warn!(
                        "[sessions] skipping unreadable transcript {}: {error}",
                        path.display()
                    );
                    skipped += 1;
                    continue;
                }
            };
            found += 1;
            let destination_stem = if !stem_names_known_agent {
                meta.thread_id
                    .as_deref()
                    .map(|thread_id| session_stem(&SessionRef::scoped(thread_id, &agent)))
                    .unwrap_or(stem)
            } else {
                stem
            };
            let history = stores
                .transcripts
                .open_stem(&destination_stem, meta.clone())?;
            replay_transcript(history.as_ref(), &meta, tools.as_ref(), records)
                .with_context(|| format!("[sessions] replay transcript {}", path.display()))?;
            written += 1;
        }
    }
    Ok((found, written, skipped))
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
