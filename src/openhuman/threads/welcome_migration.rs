//! One-shot workspace migration for artifacts created by the removed
//! welcome-agent onboarding flow.
//!
//! This migration handles two legacy footprints:
//!
//! 1. Threads created by the old React onboarding flow carried the
//!    `"onboarding"` label. That label no longer has any runtime meaning
//!    and just makes the thread look special in storage.
//! 2. Session transcripts for welcome-agent chats were persisted under
//!    `welcome*` agent names. Now that all chats route to the orchestrator,
//!    those transcripts should be normalized to `orchestrator*` naming so
//!    future inspection and resume surfaces don't imply the deleted agent
//!    still exists.
//!
//! The migration is idempotent and guarded by a marker file under
//! `state/migrations/`.

use std::fs;
use std::path::Path;

use crate::openhuman::memory_conversations as conversations;
use serde_json::{json, Value};

const MIGRATION_MARKER: &str = "state/migrations/welcome_to_orchestrator_v1.done";
const WELCOME_THREAD_LABEL: &str = "onboarding";
const WELCOME_AGENT_PREFIX: &str = "welcome";
const ORCHESTRATOR_AGENT_PREFIX: &str = "orchestrator";

#[derive(Debug, Default, Clone)]
pub struct WelcomeMigrationResult {
    pub threads_updated: usize,
    pub transcripts_updated: usize,
    pub transcript_files_renamed: usize,
    pub markdown_files_renamed: usize,
    pub already_done: bool,
}

pub fn migrate_welcome_agent_artifacts(
    workspace_dir: &Path,
) -> Result<WelcomeMigrationResult, String> {
    let marker = workspace_dir.join(MIGRATION_MARKER);
    if marker.exists() {
        log::debug!(
            "[migration::welcome-to-orchestrator] marker present at {} — skipping",
            marker.display()
        );
        return Ok(WelcomeMigrationResult {
            already_done: true,
            ..WelcomeMigrationResult::default()
        });
    }

    let mut result = WelcomeMigrationResult::default();
    let thread_result = migrate_welcome_thread_labels(workspace_dir)?;
    result.threads_updated = thread_result.threads_updated;
    let transcript_result = migrate_welcome_transcripts(workspace_dir)?;
    result.transcripts_updated = transcript_result.transcripts_updated;
    result.transcript_files_renamed = transcript_result.transcript_files_renamed;
    result.markdown_files_renamed = transcript_result.markdown_files_renamed;

    if thread_result.failures > 0 || transcript_result.failures > 0 {
        return Err(format!(
            "[migration::welcome-to-orchestrator] partial migration: thread_failures={} transcript_failures={}",
            thread_result.failures, transcript_result.failures
        ));
    }

    write_marker(&marker)?;

    log::info!(
        "[migration::welcome-to-orchestrator] complete: threads_updated={} transcripts_updated={} transcript_files_renamed={} markdown_files_renamed={}",
        result.threads_updated,
        result.transcripts_updated,
        result.transcript_files_renamed,
        result.markdown_files_renamed
    );

    Ok(result)
}

struct ThreadLabelMigrationResult {
    threads_updated: usize,
    failures: usize,
}

fn migrate_welcome_thread_labels(
    workspace_dir: &Path,
) -> Result<ThreadLabelMigrationResult, String> {
    log::debug!(
        "[migration::welcome-to-orchestrator] scanning workspace={} for legacy '{}' thread label",
        workspace_dir.display(),
        WELCOME_THREAD_LABEL
    );

    let threads = conversations::list_threads(workspace_dir.to_path_buf())
        .map_err(|e| format!("[migration::welcome-to-orchestrator] list_threads failed: {e}"))?;

    let mut threads_updated = 0usize;
    let mut failures = 0usize;
    for thread in &threads {
        if !thread
            .labels
            .iter()
            .any(|label| label == WELCOME_THREAD_LABEL)
        {
            continue;
        }

        let new_labels: Vec<String> = thread
            .labels
            .iter()
            .filter(|label| label.as_str() != WELCOME_THREAD_LABEL)
            .cloned()
            .collect();

        let updated_at = chrono::Utc::now().to_rfc3339();
        match conversations::update_thread_labels(
            workspace_dir.to_path_buf(),
            &thread.id,
            new_labels,
            &updated_at,
        ) {
            Ok(_) => {
                threads_updated += 1;
                log::info!(
                    "[migration::welcome-to-orchestrator] stripped '{}' label from thread_id={}",
                    WELCOME_THREAD_LABEL,
                    thread.id
                );
            }
            Err(err) => {
                failures += 1;
                log::warn!(
                    "[migration::welcome-to-orchestrator] failed to update thread_id={}: {err}",
                    thread.id
                );
            }
        }
    }

    Ok(ThreadLabelMigrationResult {
        threads_updated,
        failures,
    })
}

#[derive(Default)]
struct TranscriptMigrationResult {
    transcripts_updated: usize,
    transcript_files_renamed: usize,
    markdown_files_renamed: usize,
    failures: usize,
}

fn migrate_welcome_transcripts(workspace_dir: &Path) -> Result<TranscriptMigrationResult, String> {
    let raw_root = workspace_dir.join("session_raw");
    if !raw_root.is_dir() {
        return Ok(TranscriptMigrationResult::default());
    }

    let entries = fs::read_dir(&raw_root)
        .map_err(|e| format!("[migration::welcome-to-orchestrator] read_dir failed: {e}"))?;

    let mut result = TranscriptMigrationResult::default();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }

        let Some((old_agent_name, new_agent_name, new_meta_line)) =
            rewrite_meta_line_if_welcome_chat(&path)
        else {
            continue;
        };

        let old_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("invalid transcript filename: {}", path.display()))?
            .to_string();

        let new_stem = renamed_stem(&old_stem, &old_agent_name, &new_agent_name);
        let mut destination = path.clone();
        if new_stem != old_stem {
            destination = path.with_file_name(format!("{new_stem}.jsonl"));
        }

        if destination != path && destination.exists() {
            result.failures += 1;
            log::warn!(
                "[migration::welcome-to-orchestrator] destination already exists for {} — updating metadata in place",
                destination.display()
            );
            // Keep legacy artifact untouched so a future retry can still
            // detect and rename it if the collision is removed.
            continue;
        }

        if let Err(err) = write_rewritten_meta(&path, &new_meta_line, destination.as_path()) {
            result.failures += 1;
            log::warn!("[migration::welcome-to-orchestrator] {err}");
            continue;
        }
        result.transcripts_updated += 1;
        if destination != path {
            result.transcript_files_renamed += 1;
            rename_markdown_companions(workspace_dir, &old_stem, &new_stem, &mut result);
        }
    }

    Ok(result)
}

fn rewrite_meta_line_if_welcome_chat(path: &Path) -> Option<(String, String, String)> {
    let contents = fs::read_to_string(path).ok()?;
    let (first_line, _) = match contents.split_once('\n') {
        Some(parts) => parts,
        None => (contents.as_str(), ""),
    };

    let mut meta: Value = serde_json::from_str(first_line).ok()?;
    let meta_obj = meta.get("_meta")?.as_object()?;
    let thread_id = meta_obj.get("thread_id")?.as_str()?.trim();
    if thread_id.is_empty() {
        return None;
    }

    let agent = meta_obj.get("agent")?.as_str()?.trim().to_string();
    let new_agent_name = rewrite_agent_name(&agent)?;
    let meta_obj = meta.get_mut("_meta")?.as_object_mut()?;
    meta_obj.insert("agent".to_string(), json!(new_agent_name.clone()));
    let serialized = serde_json::to_string(&meta).ok()?;
    Some((agent, new_agent_name, serialized))
}

fn rewrite_agent_name(agent: &str) -> Option<String> {
    if agent == WELCOME_AGENT_PREFIX {
        return Some(ORCHESTRATOR_AGENT_PREFIX.to_string());
    }
    if let Some(suffix) = agent.strip_prefix(&format!("{WELCOME_AGENT_PREFIX}_")) {
        return Some(format!("{ORCHESTRATOR_AGENT_PREFIX}_{suffix}"));
    }
    None
}

fn renamed_stem(old_stem: &str, old_agent_name: &str, new_agent_name: &str) -> String {
    let old_agent = sanitize_agent_name(old_agent_name);
    let new_agent = sanitize_agent_name(new_agent_name);

    if old_stem.ends_with(&old_agent) {
        let prefix = &old_stem[..old_stem.len().saturating_sub(old_agent.len())];
        return format!("{prefix}{new_agent}");
    }

    if let Some(rest) = old_stem.strip_prefix(&format!("{old_agent}_")) {
        return format!("{new_agent}_{rest}");
    }

    old_stem.to_string()
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

fn write_rewritten_meta(
    path: &Path,
    new_meta_line: &str,
    destination: &Path,
) -> Result<(), String> {
    let original = fs::read_to_string(path).map_err(|e| {
        format!(
            "[migration::welcome-to-orchestrator] read {} failed: {e}",
            path.display()
        )
    })?;
    let rest = match original.split_once('\n') {
        Some((_, rest)) => rest,
        None => "",
    };

    let mut rewritten = String::with_capacity(new_meta_line.len() + rest.len() + 1);
    rewritten.push_str(new_meta_line);
    if !rest.is_empty() {
        rewritten.push('\n');
        rewritten.push_str(rest);
    }

    fs::write(path, rewritten).map_err(|e| {
        format!(
            "[migration::welcome-to-orchestrator] write {} failed: {e}",
            path.display()
        )
    })?;

    if destination != path {
        fs::rename(path, destination).map_err(|e| {
            format!(
                "[migration::welcome-to-orchestrator] rename {} -> {} failed: {e}",
                path.display(),
                destination.display()
            )
        })?;
        log::info!(
            "[migration::welcome-to-orchestrator] renamed transcript {} -> {}",
            path.display(),
            destination.display()
        );
    } else {
        log::info!(
            "[migration::welcome-to-orchestrator] rewrote transcript metadata in {}",
            path.display()
        );
    }

    Ok(())
}

fn rename_markdown_companions(
    workspace_dir: &Path,
    old_stem: &str,
    new_stem: &str,
    result: &mut TranscriptMigrationResult,
) {
    let sessions_root = workspace_dir.join("sessions");
    if !sessions_root.is_dir() {
        return;
    }

    let Ok(entries) = fs::read_dir(&sessions_root) else {
        return;
    };

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let old_path = dir.join(format!("{old_stem}.md"));
        if !old_path.exists() {
            continue;
        }
        let new_path = dir.join(format!("{new_stem}.md"));
        if new_path.exists() {
            result.failures += 1;
            log::warn!(
                "[migration::welcome-to-orchestrator] markdown destination already exists at {} — leaving {} in place",
                new_path.display(),
                old_path.display()
            );
            continue;
        }
        if let Err(err) = fs::rename(&old_path, &new_path) {
            result.failures += 1;
            log::warn!(
                "[migration::welcome-to-orchestrator] failed to rename markdown companion {} -> {}: {}",
                old_path.display(),
                new_path.display(),
                err
            );
            continue;
        }
        result.markdown_files_renamed += 1;
    }
}

fn write_marker(marker: &Path) -> Result<(), String> {
    let Some(parent) = marker.parent() else {
        return Err(format!(
            "[migration::welcome-to-orchestrator] invalid marker path {}",
            marker.display()
        ));
    };
    fs::create_dir_all(parent).map_err(|e| {
        format!(
            "[migration::welcome-to-orchestrator] create marker dir {} failed: {e}",
            parent.display()
        )
    })?;
    fs::write(marker, b"done\n").map_err(|e| {
        format!(
            "[migration::welcome-to-orchestrator] write marker {} failed: {e}",
            marker.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_conversations::{
        ensure_thread, list_threads, CreateConversationThread,
    };
    use tempfile::TempDir;

    fn make_thread(id: &str, labels: Vec<String>) -> CreateConversationThread {
        CreateConversationThread {
            id: id.to_string(),
            title: format!("Test thread {id}"),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            parent_thread_id: None,
            labels: Some(labels),
            personality_id: None,
        }
    }

    fn write_transcript(path: &Path, agent: &str, thread_id: &str) {
        let body = format!(
            "{{\"_meta\":{{\"agent\":\"{agent}\",\"dispatcher\":\"native\",\"created\":\"2026-05-01T00:00:00Z\",\"updated\":\"2026-05-01T00:00:00Z\",\"turn_count\":1,\"input_tokens\":0,\"output_tokens\":0,\"cached_input_tokens\":0,\"charged_amount_usd\":0.0,\"thread_id\":\"{thread_id}\"}}}}\n{{\"role\":\"user\",\"content\":\"hi\"}}\n"
        );
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn migration_updates_thread_labels_and_transcripts() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();

        ensure_thread(
            workspace.to_path_buf(),
            make_thread(
                "welcome-thread",
                vec!["onboarding".into(), "personal".into()],
            ),
        )
        .unwrap();

        let raw = workspace.join("session_raw/1715000000_welcome_thread-abc.jsonl");
        write_transcript(&raw, "welcome_thread-abc", "thread-abc");
        let md = workspace.join("sessions/2026_05_01/1715000000_welcome_thread-abc.md");
        fs::create_dir_all(md.parent().unwrap()).unwrap();
        fs::write(&md, "# Session transcript — welcome_thread-abc\n").unwrap();

        let result = migrate_welcome_agent_artifacts(workspace).unwrap();

        assert_eq!(result.threads_updated, 1);
        assert_eq!(result.transcripts_updated, 1);
        assert_eq!(result.transcript_files_renamed, 1);
        assert_eq!(result.markdown_files_renamed, 1);

        let threads = list_threads(workspace.to_path_buf()).unwrap();
        let thread = threads
            .iter()
            .find(|thread| thread.id == "welcome-thread")
            .unwrap();
        assert_eq!(thread.labels, vec!["personal"]);

        let renamed = workspace.join("session_raw/1715000000_orchestrator_thread-abc.jsonl");
        assert!(renamed.exists(), "renamed transcript should exist");
        let contents = fs::read_to_string(&renamed).unwrap();
        assert!(
            contents.contains("\"agent\":\"orchestrator_thread-abc\""),
            "transcript metadata should be rewritten: {contents}"
        );
        assert!(
            workspace
                .join("sessions/2026_05_01/1715000000_orchestrator_thread-abc.md")
                .exists(),
            "markdown companion should be renamed"
        );
        assert!(workspace.join(MIGRATION_MARKER).exists());
    }

    #[test]
    fn migration_is_idempotent_after_marker() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        fs::create_dir_all(workspace.join("state/migrations")).unwrap();
        fs::write(workspace.join(MIGRATION_MARKER), b"done\n").unwrap();

        let result = migrate_welcome_agent_artifacts(workspace).unwrap();
        assert!(result.already_done);
        assert_eq!(result.threads_updated, 0);
        assert_eq!(result.transcripts_updated, 0);
    }

    #[test]
    fn migration_returns_error_without_marker_when_destination_exists() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();

        let raw = workspace.join("session_raw/1715000000_welcome_thread-abc.jsonl");
        write_transcript(&raw, "welcome_thread-abc", "thread-abc");
        let dest = workspace.join("session_raw/1715000000_orchestrator_thread-abc.jsonl");
        write_transcript(&dest, "orchestrator_thread-abc", "thread-abc");

        let err = migrate_welcome_agent_artifacts(workspace).unwrap_err();

        assert!(
            err.contains("partial migration"),
            "expected partial-migration error, got: {err}"
        );
        let contents = fs::read_to_string(&raw).unwrap();
        assert!(
            contents.contains("\"agent\":\"welcome_thread-abc\""),
            "blocked rename should leave legacy metadata untouched: {contents}"
        );
        assert!(
            !workspace.join(MIGRATION_MARKER).exists(),
            "partial migration must not write marker"
        );
    }
}
