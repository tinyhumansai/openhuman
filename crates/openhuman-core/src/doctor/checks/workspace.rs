//! Read-only workspace checks and a bounded, disposable write probe.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use crate::doctor::{DoctorCheck, DoctorCtx, DoctorTier, Finding, FindingSeverity};

const WRITABLE_ID: &str = "workspace/writable";
const LAYOUT_ID: &str = "workspace/layout";
const LOW_DISK_SPACE_MB: u64 = 512;

pub(super) fn checks() -> Vec<Arc<dyn DoctorCheck>> {
    vec![Arc::new(WorkspaceWritable), Arc::new(WorkspaceLayout)]
}

struct WorkspaceWritable;

#[async_trait]
impl DoctorCheck for WorkspaceWritable {
    fn id(&self) -> &str {
        WRITABLE_ID
    }

    fn category(&self) -> &str {
        "workspace"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let workspace = &ctx.config().workspace_dir;
        if !workspace.is_dir() {
            return vec![workspace_error(
                "The configured workspace directory is missing or is not a directory.",
            )];
        }

        if crate::storage::probe_directory_writable(workspace).is_ok() {
            tracing::debug!(target: "doctor", check = WRITABLE_ID, outcome = "writable");
            Vec::new()
        } else {
            tracing::debug!(target: "doctor", check = WRITABLE_ID, outcome = "unwritable");
            vec![workspace_error(
                "The configured workspace directory is not writable.",
            )]
        }
    }
}

fn workspace_error(message: &str) -> Finding {
    let mut finding = Finding::error(
        WRITABLE_ID,
        message,
        "Grant write access to the workspace directory, then rerun doctor.",
    );
    finding.config_path = Some("workspace_dir".to_owned());
    tracing::debug!(target: "doctor", check = WRITABLE_ID, outcome = "error");
    finding
}

struct WorkspaceLayout;

#[async_trait]
impl DoctorCheck for WorkspaceLayout {
    fn id(&self) -> &str {
        LAYOUT_ID
    }

    fn category(&self) -> &str {
        "workspace"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let workspace = &ctx.config().workspace_dir;
        let disk_space_mb = available_disk_space_mb(workspace);
        let findings = layout_findings(workspace, disk_space_mb);
        tracing::debug!(
            target: "doctor",
            check = LAYOUT_ID,
            finding_count = findings.len(),
            "workspace layout inspected"
        );
        findings
    }
}

fn layout_findings(workspace: &Path, available_mb: Option<u64>) -> Vec<Finding> {
    if !workspace.is_dir() {
        return Vec::new();
    }

    let mut findings = Vec::new();
    let memory = workspace.join("memory");
    if !memory.is_dir() {
        findings.push(layout_warning(
            "The workspace memory directory is missing.",
            "Create the workspace `memory/` directory before using local memory features.",
            "workspace_dir/memory",
        ));
    }

    let system_prompt = workspace.join("SYSTEM.md");
    if !system_prompt.is_file() {
        findings.push(layout_warning(
            "The workspace SYSTEM.md file is missing.",
            "Add a SYSTEM.md file to the workspace if its agents need a workspace prompt.",
            "workspace_dir/SYSTEM.md",
        ));
    }

    if available_mb.is_some_and(|free| free < LOW_DISK_SPACE_MB) {
        findings.push(layout_warning(
            "The filesystem containing the workspace has less than 512 MB free.",
            "Free disk space on the filesystem that contains the workspace.",
            "workspace_dir",
        ));
    }
    findings
}

fn layout_warning(message: &str, fix_hint: &str, config_path: &str) -> Finding {
    let mut finding = Finding::new(LAYOUT_ID, FindingSeverity::Warn, message, fix_hint);
    finding.config_path = Some(config_path.to_owned());
    finding
}

fn available_disk_space_mb(path: &Path) -> Option<u64> {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|disk| canonical.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(|disk| disk.available_space() / (1024 * 1024))
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
