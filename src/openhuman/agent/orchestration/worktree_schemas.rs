//! Controller schemas + JSON-RPC dispatchers for the git-worktree manager
//! (#3376). Exposes the read + cleanup surface the app needs to list, inspect,
//! diff, and remove the isolated worker worktrees created by
//! [`super::tools::spawn_parallel_agents`].
//!
//! Namespace `worktree`:
//! - `worktree_list`   — list isolated worker worktrees + cross-worker overlaps.
//! - `worktree_status` — branch / dirty / changed-files snapshot for one path.
//! - `worktree_diff`   — human-readable `--stat` diff for one worktree.
//! - `worktree_remove` — remove a worktree (refuses dirty unless `force=true`).
//!
//! Handlers delegate to [`super::worktree`]; no business logic lives here. The
//! repository root every operation anchors on is the agent's `action_dir`
//! (the user's project repo), resolved from the loaded [`Config`].
//!
//! [`Config`]: crate::openhuman::config::schema::types::Config

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::agent::orchestration::worktree::{self, WorktreeError, WorktreeStatus};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

/// Controller schemas exposed by the worktree manager.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("worktree_list"),
        schema_for("worktree_status"),
        schema_for("worktree_diff"),
        schema_for("worktree_remove"),
    ]
}

/// Registered controllers (schema + handler) for the worktree manager.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("worktree_list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schema_for("worktree_status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schema_for("worktree_diff"),
            handler: handle_diff,
        },
        RegisteredController {
            schema: schema_for("worktree_remove"),
            handler: handle_remove,
        },
    ]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "worktree_list" => ControllerSchema {
            namespace: "worktree",
            function: "list",
            description: "List the isolated worker git worktrees under \
                          <repo>/.claude/worktrees, each with branch / dirty / \
                          changed-files state, plus cross-worker file overlaps.",
            inputs: vec![],
            outputs: vec![json_output(
                "result",
                "WorktreeListView: { worktrees: WorktreeStatus[], overlaps: \
                 { file, branches }[] }.",
            )],
        },
        "worktree_status" => ControllerSchema {
            namespace: "worktree",
            function: "status",
            description: "Branch / dirty / changed-files snapshot for a single \
                          worktree by absolute path.",
            inputs: vec![required_str("path", "Absolute worktree checkout path.")],
            outputs: vec![json_output("result", "WorktreeStatus payload.")],
        },
        "worktree_diff" => ControllerSchema {
            namespace: "worktree",
            function: "diff",
            description: "Human-readable `git diff HEAD --stat` (plus untracked \
                          files) for a single worktree. Empty for a clean tree.",
            inputs: vec![required_str("path", "Absolute worktree checkout path.")],
            outputs: vec![json_output("result", "{ summary: string }.")],
        },
        "worktree_remove" => ControllerSchema {
            namespace: "worktree",
            function: "remove",
            description: "Remove a worktree checkout. Refuses a dirty worktree \
                          unless force=true (uncommitted work needs an explicit \
                          user decision). The worker/<id> branch is left intact.",
            inputs: vec![
                required_str("path", "Absolute worktree checkout path."),
                optional_bool(
                    "force",
                    "Remove even when the worktree has uncommitted changes \
                     (default false).",
                ),
            ],
            outputs: vec![json_output("result", "{ removed: bool }.")],
        },
        other => unreachable!("unknown worktree schema: {other}"),
    }
}

/// Resolve the project repo root every worktree op anchors on — the agent's
/// `action_dir` from the loaded config.
async fn repo_root(cid: &str) -> Result<PathBuf, String> {
    let config = config_rpc::load_config_with_timeout()
        .await
        .inspect_err(|err| {
            log::warn!(target: "worktree_rpc", "[worktree_rpc][{cid}] config_failed err={err}");
        })?;
    Ok(config.action_dir.clone())
}

/// Resolve and validate the `path` param for status/diff/remove.
///
/// These ops must only ever target an isolated worker checkout under
/// `<repo>/.claude/worktrees`. Requiring an **absolute, managed** path before
/// delegating keeps `worktree_remove` (and the read ops) from acting on the
/// main checkout or any arbitrary directory the caller passes.
fn require_managed_worktree_path(params: &Map<String, Value>) -> Result<PathBuf, String> {
    let path = params
        .get("path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "missing required param: path".to_string())?;
    if !path.is_absolute() {
        return Err("invalid param `path`: absolute path required".to_string());
    }
    if !is_managed_worktree(&path) {
        return Err("invalid param `path`: not a managed worker worktree".to_string());
    }
    Ok(path)
}

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "worktree_rpc", "[worktree_rpc][{cid}] list.entry");
        let root = repo_root(&cid).await?;
        list_view(&root, &cid)
    })
}

fn handle_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "worktree_rpc", "[worktree_rpc][{cid}] status.entry");
        let root = repo_root(&cid).await?;
        let path = require_managed_worktree_path(&params)?;
        status_view(&root, &path, &cid)
    })
}

fn handle_diff(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "worktree_rpc", "[worktree_rpc][{cid}] diff.entry");
        let root = repo_root(&cid).await?;
        let path = require_managed_worktree_path(&params)?;
        diff_view(&root, &path, &cid)
    })
}

fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "worktree_rpc", "[worktree_rpc][{cid}] remove.entry");
        let root = repo_root(&cid).await?;
        let path = require_managed_worktree_path(&params)?;
        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        remove_view(&root, &path, force, &cid)
    })
}

/// Pure list logic anchored on an already-resolved `repo_root` — split out of
/// [`handle_list`] so it's unit-testable against a real temp repo without the
/// global config load.
fn list_view(root: &Path, cid: &str) -> Result<Value, String> {
    // A non-git action_dir is normal (the user may not have opened a repo).
    // Degrade to an empty list rather than surfacing an error to the panel.
    let all = match worktree::list(root) {
        Ok(list) => list,
        Err(WorktreeError::NotAGitRepo(p)) => {
            log::debug!(target: "worktree_rpc", "[worktree_rpc][{cid}] list.not_a_git_repo path={}", p.display());
            return to_json(json!({ "worktrees": [], "overlaps": [] }));
        }
        Err(err) => {
            let s = err.to_string();
            log::warn!(target: "worktree_rpc", "[worktree_rpc][{cid}] list.error err={s}");
            return Err(s);
        }
    };

    // Only the isolated worker worktrees are management targets — never the
    // main checkout. Filter to those nested under `.claude/worktrees`.
    let worktrees: Vec<WorktreeStatus> = all
        .into_iter()
        .filter(|w| is_managed_worktree(&w.path))
        .collect();

    let overlaps = overlaps_json(&worktrees);
    log::debug!(
        target: "worktree_rpc",
        "[worktree_rpc][{cid}] list.success count={} overlaps={}",
        worktrees.len(),
        overlaps.len()
    );
    to_json(json!({ "worktrees": worktrees, "overlaps": overlaps }))
}

/// Pure status logic anchored on a resolved `repo_root` — see [`list_view`].
fn status_view(root: &Path, path: &Path, cid: &str) -> Result<Value, String> {
    log::debug!(target: "worktree_rpc", "[worktree_rpc][{cid}] status.path={}", path.display());
    let status = worktree::status(root, path).map_err(|e| {
        let s = e.to_string();
        log::warn!(target: "worktree_rpc", "[worktree_rpc][{cid}] status.error err={s}");
        s
    })?;
    to_json(status)
}

/// Pure diff logic anchored on a resolved `repo_root` — see [`list_view`].
fn diff_view(root: &Path, path: &Path, cid: &str) -> Result<Value, String> {
    log::debug!(target: "worktree_rpc", "[worktree_rpc][{cid}] diff.path={}", path.display());
    let summary = worktree::diff_summary(root, path).map_err(|e| {
        let s = e.to_string();
        log::warn!(target: "worktree_rpc", "[worktree_rpc][{cid}] diff.error err={s}");
        s
    })?;
    to_json(json!({ "summary": summary }))
}

/// Pure remove logic anchored on a resolved `repo_root` — see [`list_view`].
fn remove_view(root: &Path, path: &Path, force: bool, cid: &str) -> Result<Value, String> {
    log::debug!(target: "worktree_rpc", "[worktree_rpc][{cid}] remove.path={} force={force}", path.display());
    worktree::remove(root, path, force).map_err(|e| {
        let s = e.to_string();
        log::warn!(target: "worktree_rpc", "[worktree_rpc][{cid}] remove.error err={s}");
        s
    })?;
    log::debug!(target: "worktree_rpc", "[worktree_rpc][{cid}] remove.success path={}", path.display());
    to_json(json!({ "removed": true }))
}

/// `true` when `path` is an isolated worker worktree (nested under the
/// `.claude/worktrees` convention dir), i.e. a manageable cleanup target.
fn is_managed_worktree(path: &Path) -> bool {
    let needle = std::path::Path::new(worktree::WORKTREE_SUBDIR);
    let mut comps = needle.components();
    let (Some(a), Some(b)) = (comps.next(), comps.next()) else {
        return false;
    };
    // Match the consecutive ".claude" / "worktrees" segments anywhere in path.
    let segs: Vec<_> = path.components().collect();
    segs.windows(2).any(|w| w[0] == a && w[1] == b)
}

/// Compute cross-worktree file overlaps (a changed file touched by more than
/// one worktree), keyed for display by each worktree's branch (path fallback).
fn overlaps_json(worktrees: &[WorktreeStatus]) -> Vec<Value> {
    let per_worker: Vec<(String, Vec<PathBuf>)> = worktrees
        .iter()
        .filter(|w| !w.changed_files.is_empty())
        .map(|w| {
            let label = w
                .branch
                .clone()
                .unwrap_or_else(|| w.path.to_string_lossy().to_string());
            (label, w.changed_files.clone())
        })
        .collect();
    worktree::detect_overlaps(&per_worker)
        .into_iter()
        .map(|(file, branches)| json!({ "file": file.to_string_lossy(), "branches": branches }))
        .collect()
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    RpcOutcome::new(value, vec![]).into_cli_compatible_json()
}

fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn required_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
#[path = "worktree_schemas_tests.rs"]
mod tests;
