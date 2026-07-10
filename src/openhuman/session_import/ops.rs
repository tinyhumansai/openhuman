//! The importer: scan → plan → (optionally) write TinyAgents store records.
//!
//! Sources are never mutated or deleted. All writes land under
//! `{workspace}/tinyagents_store/`:
//!
//! - `kv/sessions/{session_key}.json` — compatibility descriptor;
//! - `kv/migration_items/{sha256(source)}.json` — per-item idempotency ledger;
//! - `kv/migrations/session_import_v1.json` — global run marker;
//! - `journal/session.{stem}.messages.jsonl` — message journal
//!   (`StoreRecord` per line via TinyAgents `JsonlAppendStore`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::json;
use sha2::{Digest, Sha256};
use tinyagents::harness::store::{AppendStore, FileStore, JsonlAppendStore, Store};

use crate::openhuman::agent::harness::session::transcript::{
    read_transcript, read_transcript_legacy_md, SessionTranscript,
};

use super::convert::{
    build_descriptor, effective_thread_id, journal_messages, parent_session_key, stream_name,
};
use super::scan::{discover_sources, SourceItem};
use super::types::{
    DescriptorSource, ImportOptions, ImportSummary, ItemAction, ItemLedgerRecord, ItemReport,
    SourceKind, IMPORT_VERSION, JOURNAL_SUBDIR, KV_SUBDIR, MARKER_KEY, NS_MIGRATIONS,
    NS_MIGRATION_ITEMS, NS_SESSIONS,
};

/// Root of the TinyAgents store tree inside a workspace.
pub fn store_root(workspace: &Path) -> PathBuf {
    workspace.join("tinyagents_store")
}

/// The KV + journal store handles opened over a workspace's TinyAgents store
/// tree (`{workspace}/tinyagents_store/{kv,journal}`), plus the journal root
/// for stream-file layout math.
pub(crate) struct SessionStores {
    pub kv: FileStore,
    pub journal: JsonlAppendStore,
    pub journal_root: PathBuf,
}

/// Open the KV + journal stores under `{workspace}/tinyagents_store/{kv,journal}`.
///
/// Shared by the one-time importer ([`run_import`]) and the live dual-write
/// ([`super::live::write_live_turn`]) so both use the exact same store layout and
/// records land in the same place regardless of who wrote them.
pub(crate) fn open_session_stores(workspace: &Path) -> SessionStores {
    let root = store_root(workspace);
    let journal_root = root.join(JOURNAL_SUBDIR);
    SessionStores {
        kv: FileStore::new(root.join(KV_SUBDIR)),
        journal: JsonlAppendStore::new(&journal_root),
        journal_root,
    }
}

/// Best-effort run-ledger links read from `session_db/sessions.db`
/// (read-only; missing DB or table is not an error).
#[derive(Debug, Default)]
struct RunLedgerLinks {
    /// thread id → sorted, deduped `agent_runs.id`s touching it.
    runs_by_thread: HashMap<String, Vec<String>>,
    /// worker thread id → `agent_runs.parent_thread_id` (for lineage checks).
    parent_thread_by_worker: HashMap<String, String>,
}

/// Run the import. Never mutates sources; per-item failures become warnings.
pub async fn run_import(workspace: &Path, opts: &ImportOptions) -> Result<ImportSummary> {
    let SessionStores {
        kv,
        journal,
        journal_root,
    } = open_session_stores(workspace);

    log::info!(
        "[session-import] start workspace={} dry_run={} only={:?} force={}",
        workspace.display(),
        opts.dry_run,
        opts.only,
        opts.force
    );

    // Global marker fast path: a completed full import skips the scan
    // entirely, unless targeted (--only), forced, or a dry-run plan.
    let full_scan = opts.only.is_none();
    if full_scan && !opts.force && !opts.dry_run {
        if let Ok(Some(marker)) = kv.get(NS_MIGRATIONS, MARKER_KEY).await {
            log::info!("[session-import] marker present, nothing to do: {marker}");
            return Ok(ImportSummary {
                already_done: true,
                ..Default::default()
            });
        }
    }

    let only_pattern = match opts.only.as_deref() {
        Some(raw) => {
            Some(glob::Pattern::new(raw).with_context(|| format!("invalid --only glob: {raw:?}"))?)
        }
        None => None,
    };

    let (sources, scan_warnings) = discover_sources(workspace);
    let mut summary = ImportSummary {
        dry_run: opts.dry_run,
        warnings: scan_warnings,
        ..Default::default()
    };
    for w in &summary.warnings {
        log::warn!("[session-import] scan: {w}");
    }

    let sources: Vec<SourceItem> = sources
        .into_iter()
        .filter(|s| only_pattern.as_ref().is_none_or(|p| p.matches(&s.stem)))
        .collect();
    summary.scanned = sources.len();
    log::info!("[session-import] scanned {} source(s)", summary.scanned);

    // Pass 1: read every transcript so lineage cross-checks can see sibling
    // metadata. Read failures are recorded and retried as Markdown when a
    // companion exists.
    let mut parsed: HashMap<String, SessionTranscript> = HashMap::new();
    let mut read_failures: HashMap<String, String> = HashMap::new();
    for item in &sources {
        match read_source(item) {
            Ok(t) => {
                parsed.insert(item.stem.clone(), t);
            }
            Err(err) => {
                read_failures.insert(item.stem.clone(), format!("{err:#}"));
            }
        }
    }

    let links = read_run_ledger_links(workspace, &mut summary.warnings);
    let imported_at = chrono::Utc::now().to_rfc3339();

    // Pass 2: plan + write per item.
    for item in &sources {
        let report = process_item(
            item,
            &parsed,
            &read_failures,
            &links,
            &kv,
            &journal,
            &journal_root,
            &imported_at,
            opts,
        )
        .await;

        match report.action {
            ItemAction::Imported => {
                summary.imported += 1;
                summary.messages_written += report.messages;
            }
            ItemAction::WouldImport => summary.imported += 1,
            ItemAction::SkippedUnchanged => summary.skipped += 1,
            ItemAction::Failed => summary.failed += 1,
        }
        if opts.verbose {
            log::info!(
                "[session-import] {:?} stem={} source={} messages={} warnings={}",
                report.action,
                report.session_key,
                report.source,
                report.messages,
                report.warnings.len()
            );
        } else {
            log::debug!(
                "[session-import] {:?} stem={} source={}",
                report.action,
                report.session_key,
                report.source
            );
        }
        summary.items.push(report);
    }

    // Global marker only after a full, non-dry scan.
    if full_scan && !opts.dry_run {
        let marker = json!({
            "version": IMPORT_VERSION,
            "imported_at": imported_at,
            "scanned": summary.scanned,
            "imported": summary.imported,
            "skipped": summary.skipped,
            "failed": summary.failed,
            "messages_written": summary.messages_written,
            "warnings": summary.warnings.len(),
        });
        if let Err(err) = kv.put(NS_MIGRATIONS, MARKER_KEY, marker).await {
            summary
                .warnings
                .push(format!("failed to write global marker: {err}"));
        }
    }

    log::info!(
        "[session-import] done scanned={} imported={} skipped={} failed={} messages={} dry_run={}",
        summary.scanned,
        summary.imported,
        summary.skipped,
        summary.failed,
        summary.messages_written,
        summary.dry_run
    );
    Ok(summary)
}

/// Read one source with the appropriate reader.
fn read_source(item: &SourceItem) -> Result<SessionTranscript> {
    match item.kind {
        SourceKind::Jsonl | SourceKind::JsonlLegacyDir => read_transcript(&item.path)
            .with_context(|| format!("read_transcript({})", item.relative)),
        SourceKind::Markdown => read_transcript_legacy_md(&item.path)
            .with_context(|| format!("read_transcript_legacy_md({})", item.relative)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_item(
    item: &SourceItem,
    parsed: &HashMap<String, SessionTranscript>,
    read_failures: &HashMap<String, String>,
    links: &RunLedgerLinks,
    kv: &FileStore,
    journal: &JsonlAppendStore,
    journal_root: &Path,
    imported_at: &str,
    opts: &ImportOptions,
) -> ItemReport {
    let mut report = ItemReport {
        session_key: item.stem.clone(),
        source: item.relative.clone(),
        kind: item.kind,
        action: ItemAction::Failed,
        stream: None,
        thread_id: None,
        messages: 0,
        warnings: Vec::new(),
    };

    if let Some(err) = read_failures.get(&item.stem) {
        report.warnings.push(format!("unreadable source: {err}"));
        log::warn!(
            "[session-import] failed stem={} source={}: {err}",
            item.stem,
            item.relative
        );
        return report;
    }
    let Some(transcript) = parsed.get(&item.stem) else {
        report
            .warnings
            .push("internal: parsed transcript missing".into());
        return report;
    };

    let stream = stream_name(&item.stem);
    let (thread_id, synthesized) =
        effective_thread_id(&item.stem, transcript.meta.thread_id.as_deref());
    if synthesized {
        report
            .warnings
            .push(format!("no thread_id in _meta; synthesized {thread_id}"));
    }
    report.stream = Some(stream.clone());
    report.thread_id = Some(thread_id.clone());
    report.messages = transcript.messages.len();

    // Lineage cross-check: stem chain is the write-time truth; a
    // disagreeing run-ledger parent is only a warning.
    if let Some(parent_stem) = parent_session_key(&item.stem) {
        if let (Some(ledger_parent), Some(parent_transcript)) = (
            links.parent_thread_by_worker.get(&thread_id),
            parsed.get(&parent_stem),
        ) {
            if let Some(parent_thread) = parent_transcript.meta.thread_id.as_deref() {
                if ledger_parent != parent_thread {
                    report.warnings.push(format!(
                        "run-ledger parent thread {ledger_parent} disagrees with stem-chain \
                         parent thread {parent_thread}; keeping the stem chain"
                    ));
                }
            }
        }
    }

    // Idempotency: skip unchanged sources unless forced.
    let item_key = ledger_key(&item.relative);
    let (size, mtime_ms) = file_fingerprint(&item.path);
    if !opts.force {
        if let Ok(Some(prior)) = kv.get(NS_MIGRATION_ITEMS, &item_key).await {
            if let Ok(prior) = serde_json::from_value::<ItemLedgerRecord>(prior) {
                if prior.version == IMPORT_VERSION
                    && prior.size == size
                    && prior.mtime_ms == mtime_ms
                {
                    report.action = ItemAction::SkippedUnchanged;
                    return report;
                }
            }
        }
    }

    if opts.dry_run {
        report.action = ItemAction::WouldImport;
        return report;
    }

    // Re-import overwrites: the append store has no truncate, so drop the
    // stream file (layout: `{journal_root}/{stream}.jsonl`) before writing.
    let stream_file = journal_root.join(format!("{stream}.jsonl"));
    if stream_file.exists() {
        if let Err(err) = std::fs::remove_file(&stream_file) {
            report
                .warnings
                .push(format!("cannot reset journal stream {stream}: {err}"));
            return report;
        }
    }

    for (idx, record) in journal_messages(transcript).iter().enumerate() {
        let value = match serde_json::to_value(record) {
            Ok(v) => v,
            Err(err) => {
                report
                    .warnings
                    .push(format!("message {idx} not serializable: {err}"));
                return report;
            }
        };
        if let Err(err) = journal.append(&stream, value).await {
            report
                .warnings
                .push(format!("journal append failed at message {idx}: {err}"));
            return report;
        }
    }

    let run_ids = links
        .runs_by_thread
        .get(&thread_id)
        .cloned()
        .unwrap_or_default();
    let descriptor = build_descriptor(
        &item.stem,
        transcript,
        thread_id,
        synthesized,
        run_ids,
        DescriptorSource {
            jsonl: matches!(item.kind, SourceKind::Jsonl | SourceKind::JsonlLegacyDir)
                .then(|| item.relative.clone()),
            md: match item.kind {
                SourceKind::Markdown => Some(item.relative.clone()),
                _ => item.md_companion.clone(),
            },
        },
        imported_at.to_string(),
        report.warnings.len(),
    );
    let descriptor_key = super::convert::sanitize_store_name(&item.stem);
    let descriptor_value = match serde_json::to_value(&descriptor) {
        Ok(v) => v,
        Err(err) => {
            report
                .warnings
                .push(format!("descriptor not serializable: {err}"));
            return report;
        }
    };
    if let Err(err) = kv.put(NS_SESSIONS, &descriptor_key, descriptor_value).await {
        report
            .warnings
            .push(format!("descriptor write failed: {err}"));
        return report;
    }

    let ledger = ItemLedgerRecord {
        version: IMPORT_VERSION,
        session_key: item.stem.clone(),
        source: item.relative.clone(),
        size,
        mtime_ms,
        stream,
        messages: report.messages,
        imported_at: imported_at.to_string(),
    };
    match serde_json::to_value(&ledger) {
        Ok(v) => {
            if let Err(err) = kv.put(NS_MIGRATION_ITEMS, &item_key, v).await {
                report
                    .warnings
                    .push(format!("item ledger write failed: {err}"));
                return report;
            }
        }
        Err(err) => {
            report
                .warnings
                .push(format!("item ledger not serializable: {err}"));
            return report;
        }
    }

    report.action = ItemAction::Imported;
    report
}

/// Item-ledger key: hex sha256 of the workspace-relative source path.
fn ledger_key(relative: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(relative.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// `(size, mtime_ms)` fingerprint; `(0, 0)` when unreadable.
fn file_fingerprint(path: &Path) -> (u64, u64) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (0, 0);
    };
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    (meta.len(), mtime_ms)
}

/// Read run-ledger links from `session_db/sessions.db`, read-only.
///
/// Missing file/table → empty links, no warning (fresh workspaces are
/// normal). Query errors are warnings, never failures.
fn read_run_ledger_links(workspace: &Path, warnings: &mut Vec<String>) -> RunLedgerLinks {
    let db_path = workspace.join("session_db").join("sessions.db");
    if !db_path.exists() {
        return RunLedgerLinks::default();
    }
    let conn = match rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(err) => {
            warnings.push(format!("run ledger unreadable ({err}); skipping run links"));
            return RunLedgerLinks::default();
        }
    };

    let mut links = RunLedgerLinks::default();
    let mut stmt =
        match conn.prepare("SELECT id, parent_thread_id, worker_thread_id FROM agent_runs") {
            Ok(s) => s,
            Err(_) => return RunLedgerLinks::default(), // table absent: nothing to link
        };
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    });
    let rows = match rows {
        Ok(r) => r,
        Err(err) => {
            warnings.push(format!(
                "run ledger query failed ({err}); skipping run links"
            ));
            return RunLedgerLinks::default();
        }
    };
    for row in rows.flatten() {
        let (id, parent_thread, worker_thread) = row;
        for thread in [parent_thread.as_deref(), worker_thread.as_deref()]
            .into_iter()
            .flatten()
        {
            let entry = links.runs_by_thread.entry(thread.to_string()).or_default();
            if !entry.contains(&id) {
                entry.push(id.clone());
            }
        }
        if let (Some(worker), Some(parent)) = (worker_thread, parent_thread) {
            links.parent_thread_by_worker.insert(worker, parent);
        }
    }
    for ids in links.runs_by_thread.values_mut() {
        ids.sort();
    }
    links
}
