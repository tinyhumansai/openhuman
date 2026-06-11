//! Git-worktree isolation for parallel, edit-capable agent workers.
//!
//! Parallel coding workers spawned via [`super::tools::spawn_parallel_agents`]
//! historically shared one workspace (`Config.action_dir`). Two workers editing
//! overlapping files left the parent with stale assumptions and silent
//! clobbers. This module gives each isolated worker its own `git worktree`
//! checkout of the **user's project repo** (the directory the coding agent
//! edits), so file edits never collide.
//!
//! ## Scope and safety
//!
//! - This targets the **user's project repository** rooted at the agent's
//!   `action_dir` — it never operates on OpenHuman's own source tree.
//! - Every operation validates that `repo_root` is a real git repository
//!   first (via `git rev-parse --is-inside-work-tree`), so a stray path can
//!   never be mutated.
//! - [`remove`] refuses to delete a **dirty** worktree unless `force = true`.
//!   Clean worktrees can be auto-reclaimed; dirty ones require an explicit
//!   user decision (acceptance criterion of #3376).
//!
//! The wrapper shells out to `git` through [`std::process::Command`] with an
//! explicit, validated working directory. It does not inherit ambient git
//! configuration that could redirect operations elsewhere.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Directory (relative to the repo root) under which isolated worker
/// worktrees are created — mirrors Claude Code's `.claude/worktrees/`
/// convention so the layout is familiar and easy to `.gitignore`.
pub const WORKTREE_SUBDIR: &str = ".claude/worktrees";

/// Which ref a new worktree branches from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseRef {
    /// Branch off the repo's current `HEAD` — the worker sees the parent's
    /// in-progress state.
    Head,
    /// Branch off the repository's default branch (origin/HEAD, else the
    /// local default) — the worker starts from a clean, known baseline.
    Fresh,
}

impl BaseRef {
    /// Parse the spawn-request string form. Unknown / empty values default
    /// to [`BaseRef::Head`] (the least-surprising baseline for a worker that
    /// continues the parent's work).
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("fresh") => Self::Fresh,
            _ => Self::Head,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Fresh => "fresh",
        }
    }
}

/// Snapshot of a single worktree's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeStatus {
    /// Absolute path to the worktree checkout.
    pub path: PathBuf,
    /// Branch the worktree currently has checked out (or a detached-HEAD
    /// label like `(detached HEAD)`), if resolvable.
    pub branch: Option<String>,
    /// Whether the worktree has uncommitted changes (staged, unstaged, or
    /// untracked). A dirty worktree must not be auto-removed.
    pub is_dirty: bool,
    /// Paths (relative to the worktree root) that differ from HEAD.
    pub changed_files: Vec<PathBuf>,
}

/// Errors surfaced by the worktree manager. Stringified at the RPC / tool
/// boundary.
#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("path is not inside a git repository: {0}")]
    NotAGitRepo(PathBuf),

    #[error("worktree is dirty and force=false; refusing to remove: {0}")]
    DirtyRefused(PathBuf),

    #[error("git command `{command}` failed: {stderr}")]
    GitFailed { command: String, stderr: String },

    #[error("io error running git: {0}")]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, WorktreeError>;

/// Run `git <args>` in `cwd`, returning trimmed stdout on success.
fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    tracing::debug!(
        cwd = %cwd.display(),
        args = ?args,
        "[worktree] git_invoke"
    );
    let output = Command::new("git").current_dir(cwd).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        tracing::debug!(
            cwd = %cwd.display(),
            args = ?args,
            stderr = %stderr,
            "[worktree] git_failed"
        );
        return Err(WorktreeError::GitFailed {
            command: format!("git {}", args.join(" ")),
            stderr,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Like [`git`] but returns stdout **without** trimming, preserving the
/// leading whitespace that porcelain v1 status lines depend on for column
/// alignment. Trailing newline is left intact; callers iterate `.lines()`.
fn git_raw(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(cwd).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(WorktreeError::GitFailed {
            command: format!("git {}", args.join(" ")),
            stderr,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Validate that `repo_root` is inside a real git work tree. Returns the
/// repository's top-level directory on success.
fn validate_repo_root(repo_root: &Path) -> Result<PathBuf> {
    if !repo_root.exists() {
        return Err(WorktreeError::NotAGitRepo(repo_root.to_path_buf()));
    }
    // `--is-inside-work-tree` prints "true" when inside a work tree. We then
    // resolve the canonical top level so all later operations anchor on it.
    let inside = git(repo_root, &["rev-parse", "--is-inside-work-tree"])
        .map_err(|_| WorktreeError::NotAGitRepo(repo_root.to_path_buf()))?;
    if inside.trim() != "true" {
        return Err(WorktreeError::NotAGitRepo(repo_root.to_path_buf()));
    }
    let top = git(repo_root, &["rev-parse", "--show-toplevel"])
        .map_err(|_| WorktreeError::NotAGitRepo(repo_root.to_path_buf()))?;
    Ok(PathBuf::from(top.trim()))
}

/// Resolve the repo's default branch ref for `BaseRef::Fresh`. Prefers
/// `origin/HEAD`; falls back to the local `HEAD` symbolic ref, then `main`.
fn resolve_fresh_base(repo_top: &Path) -> String {
    // origin/HEAD → e.g. "origin/main"
    if let Ok(sym) = git(
        repo_top,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        if !sym.is_empty() {
            return sym;
        }
    }
    // Local default branch name.
    if let Ok(head) = git(repo_top, &["symbolic-ref", "--short", "HEAD"]) {
        if !head.is_empty() {
            return head;
        }
    }
    "main".to_string()
}

/// Create an isolated worktree for `run_id` under
/// `<repo>/.claude/worktrees/<run_id>`, branching off `base_ref`.
///
/// The new branch is named `worker/<run_id>`. Returns the worktree's status
/// snapshot (freshly created, so `is_dirty = false`).
pub fn create(repo_root: &Path, run_id: &str, base_ref: BaseRef) -> Result<WorktreeStatus> {
    let repo_top = validate_repo_root(repo_root)?;
    let run_slug = sanitize_run_id(run_id);
    let worktree_path = repo_top.join(WORKTREE_SUBDIR).join(&run_slug);
    let branch = format!("worker/{run_slug}");

    let base = match base_ref {
        BaseRef::Head => "HEAD".to_string(),
        BaseRef::Fresh => resolve_fresh_base(&repo_top),
    };

    tracing::debug!(
        repo = %repo_top.display(),
        worktree = %worktree_path.display(),
        branch = %branch,
        base = %base,
        base_ref = base_ref.as_str(),
        "[worktree] create_start"
    );

    // Ensure the parent dir exists; `git worktree add` creates the leaf.
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let worktree_str = worktree_path.to_string_lossy().to_string();
    git(
        &repo_top,
        &["worktree", "add", "-b", &branch, &worktree_str, &base],
    )?;

    tracing::debug!(
        worktree = %worktree_path.display(),
        branch = %branch,
        "[worktree] create_done"
    );

    status(&repo_top, &worktree_path)
}

/// List all worktrees registered on the repo at `repo_root`, parsed from
/// `git worktree list --porcelain`.
pub fn list(repo_root: &Path) -> Result<Vec<WorktreeStatus>> {
    let repo_top = validate_repo_root(repo_root)?;
    let porcelain = git(&repo_top, &["worktree", "list", "--porcelain"])?;
    let mut out = Vec::new();
    let mut cur_path: Option<PathBuf> = None;
    let mut cur_branch: Option<String> = None;

    let mut flush = |path: &mut Option<PathBuf>, branch: &mut Option<String>| {
        if let Some(p) = path.take() {
            // status() re-derives dirty + changed files for the worktree.
            let (is_dirty, changed_files) = dirty_state(&p).unwrap_or((false, Vec::new()));
            out.push(WorktreeStatus {
                path: p,
                branch: branch.take(),
                is_dirty,
                changed_files,
            });
        } else {
            *branch = None;
        }
    };

    for line in porcelain.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            // New record begins — flush the previous one.
            flush(&mut cur_path, &mut cur_branch);
            cur_path = Some(PathBuf::from(rest.trim()));
        } else if let Some(rest) = line.strip_prefix("branch ") {
            // "branch refs/heads/foo" → "foo"
            cur_branch = Some(
                rest.trim()
                    .strip_prefix("refs/heads/")
                    .unwrap_or(rest.trim())
                    .to_string(),
            );
        } else if line.trim() == "detached" {
            cur_branch = Some("(detached HEAD)".to_string());
        }
    }
    flush(&mut cur_path, &mut cur_branch);

    tracing::debug!(
        repo = %repo_top.display(),
        count = out.len(),
        "[worktree] list_done"
    );
    Ok(out)
}

/// Branch + dirty + changed-file snapshot for a single worktree.
pub fn status(repo_root: &Path, worktree_path: &Path) -> Result<WorktreeStatus> {
    validate_repo_root(repo_root)?;
    if !worktree_path.exists() {
        return Err(WorktreeError::NotAGitRepo(worktree_path.to_path_buf()));
    }
    let branch = git(worktree_path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|b| {
            if b == "HEAD" {
                "(detached HEAD)".to_string()
            } else {
                b
            }
        });
    let (is_dirty, changed_files) = dirty_state(worktree_path)?;
    Ok(WorktreeStatus {
        path: worktree_path.to_path_buf(),
        branch,
        is_dirty,
        changed_files,
    })
}

/// Human-readable diff stat of a worktree's working changes vs HEAD,
/// including untracked files. Returns an empty string for a clean worktree.
pub fn diff_summary(repo_root: &Path, worktree_path: &Path) -> Result<String> {
    validate_repo_root(repo_root)?;
    // `--stat` over both staged and unstaged changes vs HEAD.
    let stat = git(worktree_path, &["diff", "HEAD", "--stat"])?;
    let untracked = git(
        worktree_path,
        &["ls-files", "--others", "--exclude-standard"],
    )?;
    let mut parts = Vec::new();
    if !stat.is_empty() {
        parts.push(stat);
    }
    if !untracked.is_empty() {
        let lines: Vec<String> = untracked
            .lines()
            .map(|l| format!(" {l} (untracked)"))
            .collect();
        parts.push(lines.join("\n"));
    }
    Ok(parts.join("\n"))
}

/// Remove a worktree. **Refuses to remove a dirty worktree unless
/// `force = true`** — the core safety guarantee of #3376.
///
/// On `force = true`, a dirty worktree is removed with `git worktree remove
/// --force`. The associated `worker/<run_id>` branch is left intact so the
/// caller can still inspect or merge the work.
pub fn remove(repo_root: &Path, worktree_path: &Path, force: bool) -> Result<()> {
    let repo_top = validate_repo_root(repo_root)?;
    let (is_dirty, _changed) = dirty_state(worktree_path).unwrap_or((false, Vec::new()));

    tracing::debug!(
        repo = %repo_top.display(),
        worktree = %worktree_path.display(),
        is_dirty,
        force,
        "[worktree] remove_start"
    );

    if is_dirty && !force {
        tracing::warn!(
            worktree = %worktree_path.display(),
            "[worktree] remove_refused_dirty"
        );
        return Err(WorktreeError::DirtyRefused(worktree_path.to_path_buf()));
    }

    let worktree_str = worktree_path.to_string_lossy().to_string();
    let mut args = vec!["worktree", "remove", &worktree_str];
    if force {
        args.push("--force");
    }
    git(&repo_top, &args)?;
    tracing::debug!(
        worktree = %worktree_path.display(),
        "[worktree] remove_done"
    );
    Ok(())
}

/// Compute `(is_dirty, changed_files)` for a worktree via
/// `git status --porcelain`. A worktree is dirty when any tracked file is
/// modified/staged or any untracked (non-ignored) file is present.
///
/// Uses the raw (un-trimmed) command output: porcelain v1 lines are
/// column-aligned (`XY␠PATH`, status = 2 chars), so a global `.trim()` would
/// corrupt the leading space of a `␠M file` line and shift the path. We parse
/// the raw bytes and slice the fixed 3-char `XY␠` prefix per line.
fn dirty_state(worktree_path: &Path) -> Result<(bool, Vec<PathBuf>)> {
    let porcelain = git_raw(worktree_path, &["status", "--porcelain"])?;
    let mut changed = Vec::new();
    for line in porcelain.lines() {
        // Porcelain v1 line: "XY <path>" — status is exactly 2 chars, then a
        // single separator space, so the path always starts at byte index 3.
        if line.len() > 3 {
            let path = line[3..].trim_end();
            // Rename lines look like "old -> new"; record the new path.
            let path = path.rsplit(" -> ").next().unwrap_or(path);
            changed.push(PathBuf::from(path));
        }
    }
    changed.sort();
    changed.dedup();
    Ok((!changed.is_empty(), changed))
}

/// Sanitize a `run_id` into a filesystem-safe single path segment.
fn sanitize_run_id(run_id: &str) -> String {
    let cleaned: String = run_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "worker".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Detect changed files touched by more than one sibling worker.
///
/// Input maps each worker's id to the set of files it changed (relative
/// paths, as produced by [`WorktreeStatus::changed_files`]). Output maps each
/// overlapping file to the sorted list of worker ids that touched it — empty
/// when there is no overlap. Used to surface a pre-merge conflict warning.
pub fn detect_overlaps(
    per_worker: &[(String, Vec<PathBuf>)],
) -> std::collections::BTreeMap<PathBuf, Vec<String>> {
    use std::collections::BTreeMap;
    let mut by_file: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for (worker_id, files) in per_worker {
        let mut seen = std::collections::BTreeSet::new();
        for f in files {
            if seen.insert(f.clone()) {
                by_file
                    .entry(f.clone())
                    .or_default()
                    .push(worker_id.clone());
            }
        }
    }
    by_file
        .into_iter()
        .filter_map(|(file, mut workers)| {
            workers.sort();
            workers.dedup();
            if workers.len() > 1 {
                Some((file, workers))
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "worktree_tests.rs"]
mod tests;
