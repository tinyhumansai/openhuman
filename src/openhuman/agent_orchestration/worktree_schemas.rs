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
use crate::openhuman::agent_orchestration::worktree::{self, WorktreeError, WorktreeStatus};
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
mod tests {
    use super::*;
    use std::process::Command;

    /// `true` when `git` is invokable on this host. Tests that need real
    /// `git worktree` plumbing skip (pass trivially) when it's absent, so a
    /// git-less CI image doesn't hard-fail — same convention as
    /// `worktree_tests.rs`.
    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git invocation");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Temp git repo with one commit. Returns the guard (kept alive by the
    /// caller) and the repo root.
    fn init_repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "user.email", "test@example.com"]);
        git(&root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("README.md"), "hello\n").unwrap();
        git(&root, &["add", "README.md"]);
        git(&root, &["commit", "-m", "initial"]);
        (tmp, root)
    }

    #[test]
    fn list_view_degrades_to_empty_for_non_git_root() {
        if !git_available() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        // A plain (non-git) directory must yield an empty list, not an error —
        // the panel shows "no worktrees" rather than surfacing a failure.
        let v = list_view(tmp.path(), "cid").expect("non-git root degrades cleanly");
        assert_eq!(v["worktrees"], json!([]));
        assert_eq!(v["overlaps"], json!([]));
    }

    #[test]
    fn list_view_surfaces_managed_worktree() {
        if !git_available() {
            return;
        }
        let (_tmp, root) = init_repo();
        let st = worktree::create(&root, "run-1", worktree::BaseRef::Head).expect("create");
        assert!(
            is_managed_worktree(&st.path),
            "created under .claude/worktrees"
        );

        let v = list_view(&root, "cid").expect("list ok");
        let worktrees = v["worktrees"].as_array().expect("array");
        assert_eq!(worktrees.len(), 1, "the one managed worktree is listed");
        // A fresh worktree is clean → no overlaps.
        assert_eq!(v["overlaps"], json!([]));
    }

    /// Drives the full `handle_list` async handler (the public RPC entry point)
    /// through `repo_root` → config load, anchored on a non-git
    /// `OPENHUMAN_ACTION_DIR`. Confirms the panel-facing path degrades to an
    /// empty list rather than erroring. Holds `TEST_ENV_LOCK` because the env
    /// override is process-global.
    #[tokio::test]
    async fn handle_list_degrades_for_non_git_action_dir() {
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let action_dir = tmp.path().join("actions");
        std::fs::create_dir_all(&action_dir).unwrap();

        // SAFETY: env writes are serialized by TEST_ENV_LOCK above.
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
            std::env::set_var("OPENHUMAN_ACTION_DIR", &action_dir);
        }

        let out = handle_list(Map::new())
            .await
            .expect("list handler degrades cleanly for a non-git action_dir");
        // `into_cli_compatible_json` returns the bare value when there are no
        // logs; the list payload is therefore at the top level.
        let payload = out.get("result").unwrap_or(&out);
        assert_eq!(payload["worktrees"], json!([]));
        assert_eq!(payload["overlaps"], json!([]));

        unsafe {
            std::env::remove_var("OPENHUMAN_ACTION_DIR");
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[test]
    fn status_and_diff_views_round_trip_a_worktree() {
        if !git_available() {
            return;
        }
        let (_tmp, root) = init_repo();
        let st = worktree::create(&root, "run-2", worktree::BaseRef::Head).expect("create");

        // `WorktreeStatus` serializes `rename_all = "camelCase"` → `isDirty`.
        let status = status_view(&root, &st.path, "cid").expect("status ok");
        assert_eq!(status["isDirty"], json!(false), "fresh worktree is clean");

        // A clean worktree diffs to an empty summary.
        let diff = diff_view(&root, &st.path, "cid").expect("diff ok");
        assert_eq!(diff["summary"], json!(""));
    }

    #[test]
    fn remove_view_clears_a_clean_worktree() {
        if !git_available() {
            return;
        }
        let (_tmp, root) = init_repo();
        let st = worktree::create(&root, "run-3", worktree::BaseRef::Head).expect("create");
        assert!(st.path.exists());

        let removed = remove_view(&root, &st.path, false, "cid").expect("remove ok");
        assert_eq!(removed["removed"], json!(true));
        assert!(!st.path.exists(), "worktree dir gone after remove");
    }

    #[test]
    fn status_view_errors_on_unknown_path() {
        if !git_available() {
            return;
        }
        let (_tmp, root) = init_repo();
        let bogus = root.join(".claude/worktrees/never-created");
        assert!(status_view(&root, &bogus, "cid").is_err());
    }

    #[test]
    fn correlation_id_is_eight_hex_chars() {
        let id = new_correlation_id();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn registered_controllers_match_schemas() {
        let schemas = all_controller_schemas();
        let registered = all_registered_controllers();
        assert_eq!(schemas.len(), registered.len());
        assert_eq!(schemas.len(), 4);
        assert!(schemas.iter().all(|s| s.namespace == "worktree"));
        assert_eq!(schema_for("worktree_list").function, "list");
        assert_eq!(schema_for("worktree_status").function, "status");
        assert_eq!(schema_for("worktree_diff").function, "diff");
        assert_eq!(schema_for("worktree_remove").function, "remove");
    }

    #[test]
    fn managed_worktree_filter() {
        assert!(is_managed_worktree(Path::new(
            "/home/u/proj/.claude/worktrees/worker-abc"
        )));
        assert!(!is_managed_worktree(Path::new("/home/u/proj")));
        assert!(!is_managed_worktree(Path::new("/home/u/proj/.claude")));
    }

    #[test]
    fn require_managed_worktree_path_enforces_absolute_and_managed() {
        let mut p = Map::new();
        // Missing / blank path is rejected.
        assert!(require_managed_worktree_path(&p).is_err());
        p.insert("path".into(), Value::String("  ".into()));
        assert!(require_managed_worktree_path(&p).is_err());

        // A relative path is rejected even when it looks managed.
        p.insert(
            "path".into(),
            Value::String(".claude/worktrees/worker-x".into()),
        );
        assert!(require_managed_worktree_path(&p).is_err());

        // An absolute but unmanaged path (e.g. the main checkout) is rejected —
        // worktree_remove must never target it.
        p.insert("path".into(), Value::String("/home/u/proj".into()));
        assert!(require_managed_worktree_path(&p).is_err());

        // An absolute, managed worker checkout is accepted.
        let ok = "/home/u/proj/.claude/worktrees/worker-x";
        p.insert("path".into(), Value::String(ok.into()));
        assert_eq!(
            require_managed_worktree_path(&p).unwrap(),
            PathBuf::from(ok)
        );
    }

    #[test]
    fn overlaps_detected_across_branches() {
        let worktrees = vec![
            WorktreeStatus {
                path: PathBuf::from("/r/.claude/worktrees/a"),
                branch: Some("worker/a".into()),
                is_dirty: true,
                changed_files: vec![PathBuf::from("src/lib.rs"), PathBuf::from("a.rs")],
            },
            WorktreeStatus {
                path: PathBuf::from("/r/.claude/worktrees/b"),
                branch: Some("worker/b".into()),
                is_dirty: true,
                changed_files: vec![PathBuf::from("src/lib.rs")],
            },
        ];
        let overlaps = overlaps_json(&worktrees);
        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0]["file"], json!("src/lib.rs"));
        assert_eq!(overlaps[0]["branches"], json!(["worker/a", "worker/b"]));
    }
}
