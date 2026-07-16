//! Host orchestration for TinyCortex coding-session persona ingestion.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tinycortex::memory::persona::readers::{claude_code, codex, RawSession};
use tinycortex::memory::persona::state::FileStateStore;
use tinycortex::memory::persona::{PersonaConfig, Pipeline, RunMode};
use walkdir::WalkDir;

use crate::openhuman::config::Config;

const DEFAULT_MAX_SESSIONS: usize = 100;
const MAX_MAX_SESSIONS: usize = 1_000;
const MAX_STATUS_SESSION_FILES: usize = 1_000;
const MAX_STATUS_SESSION_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_STATUS_TOTAL_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodingSessionSourceStatus {
    pub kind: String,
    pub available: bool,
    pub session_files: usize,
    pub evidence_units: usize,
    pub invalid_files: usize,
    pub scan_truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodingSessionIngestRequest {
    #[serde(default)]
    pub backfill: bool,
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
}

fn default_max_sessions() -> usize {
    DEFAULT_MAX_SESSIONS
}

#[derive(Debug, Clone, Serialize)]
pub struct CodingSessionIngestResponse {
    pub mode: String,
    pub files_seen: usize,
    pub sessions_processed: usize,
    pub sessions_skipped: usize,
    pub sessions_failed: usize,
    pub evidence_units: usize,
    pub observations: usize,
    pub budget_hit: bool,
    pub pack_path: Option<String>,
}

fn roots_from_environment() -> (PathBuf, PathBuf) {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let claude_home = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    (claude_home.join("projects"), codex_home.join("sessions"))
}

fn source_status(
    kind: &str,
    root: &Path,
    max_files: usize,
    discover: impl Fn(&Path, usize) -> (Vec<PathBuf>, bool),
    read: impl Fn(&Path) -> anyhow::Result<RawSession>,
) -> CodingSessionSourceStatus {
    let (files, mut scan_truncated) = discover(root, max_files);
    if scan_truncated {
        tracing::debug!(
            source = kind,
            max_files,
            "[memory_persona] coding session status scan capped"
        );
    }
    let mut evidence_units = 0;
    let mut invalid_files = 0;
    let mut bytes_scheduled = 0_u64;
    for path in &files {
        if let Ok(metadata) = path.metadata() {
            let file_bytes = metadata.len();
            if file_bytes > MAX_STATUS_SESSION_FILE_BYTES
                || bytes_scheduled.saturating_add(file_bytes) > MAX_STATUS_TOTAL_BYTES
            {
                scan_truncated = true;
                tracing::debug!(
                    source = kind,
                    file_bytes,
                    bytes_scheduled,
                    max_file_bytes = MAX_STATUS_SESSION_FILE_BYTES,
                    max_total_bytes = MAX_STATUS_TOTAL_BYTES,
                    reason = "status-byte-budget",
                    "[memory_persona] skipped coding session during bounded status scan"
                );
                continue;
            }
            bytes_scheduled += file_bytes;
        }
        match read(path) {
            Ok(session) => evidence_units += session.evidence.len(),
            Err(_error) => {
                invalid_files += 1;
                tracing::debug!(
                    source = kind,
                    reason = "read-or-parse-failed",
                    "[memory_persona] skipped unreadable coding session"
                );
            }
        }
    }
    CodingSessionSourceStatus {
        kind: kind.to_string(),
        available: root.is_dir(),
        session_files: files.len(),
        evidence_units,
        invalid_files,
        scan_truncated,
    }
}

fn discover_session_files(
    root: &Path,
    max_files: usize,
    is_candidate: impl Fn(&Path) -> bool,
) -> (Vec<PathBuf>, bool) {
    let mut files = Vec::with_capacity(max_files.min(64));
    // Keep traversal unsorted: `sort_by_file_name` buffers and sorts every
    // directory before yielding its first entry, which defeats `max_files`
    // for users with very large Codex day or Claude project directories.
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if !is_candidate(path) {
            continue;
        }
        if files.len() == max_files {
            return (files, true);
        }
        files.push(path.to_path_buf());
    }
    (files, false)
}

fn discover_claude_sessions(root: &Path, max_files: usize) -> (Vec<PathBuf>, bool) {
    discover_session_files(root, max_files, |path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
    })
}

fn discover_codex_sessions(root: &Path, max_files: usize) -> (Vec<PathBuf>, bool) {
    discover_session_files(root, max_files, |path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rollout-"))
    })
}

pub fn coding_session_status_for_roots(
    claude_root: &Path,
    codex_root: &Path,
) -> Vec<CodingSessionSourceStatus> {
    tracing::debug!("[memory_persona] coding session scan: entry");
    let statuses = vec![
        source_status(
            "claude_code",
            claude_root,
            MAX_STATUS_SESSION_FILES,
            discover_claude_sessions,
            claude_code::read_session,
        ),
        source_status(
            "codex",
            codex_root,
            MAX_STATUS_SESSION_FILES,
            discover_codex_sessions,
            codex::read_session,
        ),
    ];
    tracing::debug!(
        files = statuses
            .iter()
            .map(|status| status.session_files)
            .sum::<usize>(),
        evidence = statuses
            .iter()
            .map(|status| status.evidence_units)
            .sum::<usize>(),
        invalid = statuses
            .iter()
            .map(|status| status.invalid_files)
            .sum::<usize>(),
        "[memory_persona] coding session scan: exit"
    );
    statuses
}

pub fn coding_session_status() -> Vec<CodingSessionSourceStatus> {
    let (claude_root, codex_root) = roots_from_environment();
    coding_session_status_for_roots(&claude_root, &codex_root)
}

pub async fn ingest_coding_sessions(
    config: &Config,
    request: CodingSessionIngestRequest,
) -> anyhow::Result<CodingSessionIngestResponse> {
    let (claude_root, codex_root) = roots_from_environment();
    let max_sessions = request.max_sessions.clamp(1, MAX_MAX_SESSIONS);
    let mode = if request.backfill {
        RunMode::Backfill
    } else {
        RunMode::Incremental
    };
    tracing::info!(
        mode = if request.backfill {
            "backfill"
        } else {
            "incremental"
        },
        max_sessions,
        "[memory_persona] coding session ingestion: entry"
    );

    let memory_config = super::memory_config_from(config, config.workspace_dir.clone());
    let mut persona = PersonaConfig::with_home(
        dirs::home_dir()
            .as_deref()
            .unwrap_or_else(|| Path::new(".")),
        "OpenHuman user",
    );
    persona.claude_code_root = Some(claude_root);
    persona.codex_root = Some(codex_root);
    // This product surface is deliberately scoped to coding-session history.
    // Repository history and instruction files can be wired separately with
    // their own disclosure and cost controls.
    persona.project_roots.clear();
    persona.global_instruction_files.clear();
    persona.author_emails.clear();
    persona.run_budget.max_sessions = max_sessions;
    persona.run_budget.max_llm_calls = max_sessions as u32;

    let provider = super::build_chat_provider(config).inspect_err(|error| {
        tracing::error!(
            error = %error,
            "[memory_persona] coding session ingestion: build_chat_provider failed"
        );
    })?;
    let summariser = super::HostSummariser::new(config.clone());
    let store = FileStateStore::open_in_workspace(&config.workspace_dir).inspect_err(|error| {
        tracing::error!(
            error = %error,
            "[memory_persona] coding session ingestion: open state store failed"
        );
    })?;
    let report = Pipeline {
        config: &memory_config,
        persona: &persona,
        provider: provider.as_ref(),
        summariser: &summariser,
        store: &store,
    }
    .run(mode)
    .await
    .inspect_err(|error| {
        tracing::error!(
            error = %error,
            "[memory_persona] coding session ingestion: pipeline run failed"
        );
    })?;

    tracing::info!(
        files_seen = report.files_seen,
        sessions_processed = report.sessions_processed,
        sessions_failed = report.sessions_failed,
        evidence_units = report.evidence_units,
        observations = report.observations,
        budget_hit = report.budget_hit,
        "[memory_persona] coding session ingestion: exit"
    );
    Ok(CodingSessionIngestResponse {
        mode: report.mode,
        files_seen: report.files_seen,
        sessions_processed: report.sessions_processed,
        sessions_skipped: report.sessions_skipped,
        sessions_failed: report.sessions_failed,
        evidence_units: report.evidence_units,
        observations: report.observations,
        budget_hit: report.budget_hit,
        pack_path: report.pack_path,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn scans_codex_and_claude_sessions_and_filters_machine_content() {
        let temp = tempdir().unwrap();
        let claude = temp.path().join("claude");
        let codex = temp.path().join("codex/2026/07/14");
        fs::create_dir_all(&claude).unwrap();
        fs::create_dir_all(&codex).unwrap();
        fs::write(
            claude.join("session.jsonl"),
            concat!(
                "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"machine\"}]}}\n",
                "{\"type\":\"user\",\"sessionId\":\"c1\",\"cwd\":\"/repo\",\"timestamp\":\"2026-07-14T00:00:00Z\",\"message\":{\"content\":\"Prefer small modules\"}}\n"
            ),
        )
        .unwrap();
        fs::write(
            codex.join("rollout-test.jsonl"),
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"x1\",\"cwd\":\"/repo\"}}\n",
                "{\"type\":\"response_item\",\"timestamp\":\"2026-07-14T00:00:00Z\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",\"content\":[{\"type\":\"input_text\",\"text\":\"secret scaffolding\"}]}}\n",
                "{\"type\":\"response_item\",\"timestamp\":\"2026-07-14T00:00:01Z\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Run focused tests first\"}]}}\n"
            ),
        )
        .unwrap();

        let statuses = coding_session_status_for_roots(&claude, &temp.path().join("codex"));
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].session_files, 1);
        assert_eq!(statuses[0].evidence_units, 1);
        assert_eq!(statuses[1].session_files, 1);
        assert_eq!(statuses[1].evidence_units, 1);
        assert_eq!(statuses[0].invalid_files + statuses[1].invalid_files, 0);
    }

    #[test]
    fn status_scan_stops_parsing_at_the_configured_limit() {
        let paths = vec![PathBuf::from("one"), PathBuf::from("two")];
        let reads = std::cell::Cell::new(0);
        let status = source_status(
            "fixture",
            Path::new("."),
            1,
            |_, max_files| (paths[..max_files].to_vec(), paths.len() > max_files),
            |_| {
                reads.set(reads.get() + 1);
                Ok(RawSession::new(
                    tinycortex::memory::persona::types::EvidenceSource::new(
                        tinycortex::memory::persona::types::PersonaSourceKind::Codex,
                    ),
                ))
            },
        );

        assert_eq!(reads.get(), 1);
        assert_eq!(status.session_files, 1);
        assert!(status.scan_truncated);
    }

    #[test]
    fn bounded_discovery_stops_after_finding_one_extra_candidate_without_ordering() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("a.jsonl"), "").unwrap();
        fs::write(temp.path().join("b.jsonl"), "").unwrap();
        fs::write(temp.path().join("ignored.txt"), "").unwrap();

        let (files, truncated) = discover_claude_sessions(temp.path(), 1);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].extension().unwrap(), "jsonl");
        assert!(truncated);
    }

    #[test]
    fn status_scan_skips_oversized_sessions_without_parsing_them() {
        let temp = tempdir().unwrap();
        let oversized = temp.path().join("oversized.jsonl");
        let small = temp.path().join("small.jsonl");
        let file = fs::File::create(&oversized).unwrap();
        file.set_len(MAX_STATUS_SESSION_FILE_BYTES + 1).unwrap();
        fs::write(&small, "{}\n").unwrap();
        let reads = std::cell::Cell::new(0);

        let status = source_status(
            "fixture",
            temp.path(),
            2,
            |_, _| (vec![oversized.clone(), small.clone()], false),
            |_| {
                reads.set(reads.get() + 1);
                Ok(RawSession::new(
                    tinycortex::memory::persona::types::EvidenceSource::new(
                        tinycortex::memory::persona::types::PersonaSourceKind::Codex,
                    ),
                ))
            },
        );

        assert_eq!(reads.get(), 1);
        assert_eq!(status.session_files, 2);
        assert_eq!(status.invalid_files, 0);
        assert!(status.scan_truncated);
    }

    #[test]
    fn status_scan_enforces_the_aggregate_byte_budget() {
        let temp = tempdir().unwrap();
        let paths = (0..5)
            .map(|index| {
                let path = temp.path().join(format!("session-{index}.jsonl"));
                let file = fs::File::create(&path).unwrap();
                file.set_len(MAX_STATUS_SESSION_FILE_BYTES).unwrap();
                path
            })
            .collect::<Vec<_>>();
        let reads = std::cell::Cell::new(0);

        let status = source_status(
            "fixture",
            temp.path(),
            paths.len(),
            |_, _| (paths.clone(), false),
            |_| {
                reads.set(reads.get() + 1);
                Ok(RawSession::new(
                    tinycortex::memory::persona::types::EvidenceSource::new(
                        tinycortex::memory::persona::types::PersonaSourceKind::Codex,
                    ),
                ))
            },
        );

        assert_eq!(reads.get(), 4);
        assert_eq!(status.session_files, 5);
        assert!(status.scan_truncated);
    }
}
