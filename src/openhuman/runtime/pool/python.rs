//! Python pool backend: materialise the Python harness and submit inline jobs
//! to the shared [`LangPool`](super::pool::LangPool).
//!
//! Unlike the node backend, a job runs **in the worker's own interpreter** (no
//! per-job thread isolation — CPython can't safely kill a running thread), so a
//! Python job's soft deadline is enforced best-effort via `SIGALRM` on Unix and
//! otherwise falls back to the Rust-side hard deadline (which kills + respawns
//! the worker). `recycle_after_jobs` bounds cross-job module-state leakage.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::OnceCell;

use super::pool;
use super::types::{PoolExecOutcome, PoolLang, PoolSettings};
use super::worker::WorkerLaunch;
use crate::openhuman::config::{RuntimePoolConfig, RuntimePoolLangConfig};

/// The bundled Python worker harness.
const WORKER_PY: &str = include_str!("pool_worker.py");

/// Written once per process (see [`super::ensure_worker_script`]).
static PYTHON_SCRIPT: OnceCell<PathBuf> = OnceCell::const_new();

/// Whether inline `python` jobs should route through the pool. `runtime_python.
/// enabled` is already implied by the tool only being constructed when the
/// python runtime is enabled, so only the pool switches are checked here.
pub fn enabled(pool: &RuntimePoolConfig) -> bool {
    // Python defaults OFF: jobs share one interpreter (no worker_thread
    // equivalent), so reuse can leak process-global state (`sys.modules`,
    // `os.environ`, logging handlers, threads) across otherwise-unrelated runs.
    // Opt in explicitly (`[runtime_pool.python] enabled = true`) to accept that
    // in exchange for the warm-worker memory win.
    pool.enabled && pool.python.is_enabled(false)
}

/// Run inline Python on a pooled, warm `python` worker.
///
/// `python_bin` / `bin_dir` come from the caller's already-resolved
/// [`ResolvedPython`](crate::openhuman::runtime::python::ResolvedPython).
/// `workspace_dir` and `lang_cfg` are injected at tool construction so this hot
/// path never re-reads config or re-writes the harness. `timeout` is the soft
/// per-job deadline (best-effort on Unix, hard-enforced by the Rust side).
pub async fn run_inline(
    workspace_dir: &Path,
    lang_cfg: &RuntimePoolLangConfig,
    python_bin: &Path,
    bin_dir: &Path,
    code: String,
    cwd: Option<PathBuf>,
    timeout: Option<Duration>,
) -> Result<PoolExecOutcome, super::pool::PoolRunError> {
    let script =
        super::ensure_worker_script(&PYTHON_SCRIPT, workspace_dir, "pool_worker.py", WORKER_PY)
            .await
            .map_err(super::pool::PoolRunError::PreDispatch)?
            .to_string_lossy()
            .into_owned();

    let mut env = super::base_env(bin_dir);
    // Line-buffered stdio so protocol frames flush promptly.
    env.push(("PYTHONUNBUFFERED".to_string(), "1".to_string()));

    let launch = WorkerLaunch {
        lang: PoolLang::Python,
        bin: python_bin.to_path_buf(),
        // `-u` unbuffered mirrors the runtime_python_server launch contract.
        args: vec!["-u".to_string(), script],
        env,
        isolated_protocol: true,
    };
    let settings = PoolSettings::from_lang_config(lang_cfg);
    let pool = pool::ensure_pool(launch, settings).await;

    let cwd = cwd.map(|p| p.to_string_lossy().into_owned());
    pool.run_inline(code, cwd, timeout).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use crate::openhuman::runtime::pool::{all_stats, PoolLang};

    /// Resolve the host `python3` binary + its bin dir, or `None` to skip.
    fn system_python() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        for cmd in ["python3", "python"] {
            if let Ok(out) = std::process::Command::new(cmd)
                .args(["-c", "import sys; sys.stdout.write(sys.executable)"])
                .output()
            {
                if out.status.success() {
                    let bin = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
                    if let Some(dir) = bin.parent() {
                        return Some((bin.clone(), dir.to_path_buf()));
                    }
                }
            }
        }
        None
    }

    async fn python_spawns() -> u64 {
        all_stats()
            .await
            .into_iter()
            .find(|(lang, _)| *lang == PoolLang::Python)
            .map(|(_, stats)| stats.worker_spawns)
            .unwrap_or(0)
    }

    /// End-to-end: inline Python runs on the pool, reuses a warm worker, and
    /// surfaces stdout / exit codes / cwd. Skips when no system python.
    #[tokio::test]
    async fn pooled_python_runs_inline_and_reuses_worker() {
        let Some((python_bin, bin_dir)) = system_python() else {
            eprintln!("[runtime_pool] test skipped: no system python on PATH");
            return;
        };
        let tmp = std::env::temp_dir().join(format!("rt-pool-py-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let mut config = Config::default();
        config.workspace_dir = tmp.clone();
        config.runtime_pool.python.max_workers = 1;
        config.runtime_pool.python.recycle_after_jobs = 0;
        let lang = config.runtime_pool.python.clone();

        let spawns_before = python_spawns().await;

        let out1 = run_inline(
            &config.workspace_dir,
            &lang,
            &python_bin,
            &bin_dir,
            "print(6 * 7)".to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("py job 1 runs");
        assert!(out1.success(), "job 1 should succeed: {out1:?}");
        assert_eq!(out1.stdout.trim(), "42");

        // Raising surfaces a non-zero exit + traceback on stderr.
        let out2 = run_inline(
            &config.workspace_dir,
            &lang,
            &python_bin,
            &bin_dir,
            "raise ValueError('boom')".to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("py job 2 runs");
        assert!(!out2.success(), "raising job should fail");
        assert!(out2.stderr.contains("boom"), "stderr was {:?}", out2.stderr);

        // cwd-relative read resolves against the job cwd.
        std::fs::write(tmp.join("probe.txt"), "PY_REL_OK").unwrap();
        let out3 = run_inline(
            &config.workspace_dir,
            &lang,
            &python_bin,
            &bin_dir,
            "print(open('./probe.txt').read(), end='')".to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("py job 3 runs");
        assert!(out3.success(), "cwd read should succeed: {out3:?}");
        assert_eq!(out3.stdout, "PY_REL_OK");

        // User stdin is an isolated EOF stream, not the long-lived worker's
        // NDJSON request pipe.
        let stdin_out = run_inline(
            &config.workspace_dir,
            &lang,
            &python_bin,
            &bin_dir,
            "import os, sys\nprint(repr(sys.stdin.read()))\nprint(os.read(0, 1))".to_string(),
            Some(tmp.clone()),
            Some(Duration::from_secs(2)),
        )
        .await
        .expect("stdin EOF job runs");
        assert!(
            stdin_out.success(),
            "python stdin should be EOF: {stdin_out:?}"
        );
        assert_eq!(stdin_out.stdout, "''\nb''\n");

        // A missing cwd must fail before executing user code. Continuing in the
        // worker's inherited cwd would escape the requested action root.
        let missing_cwd = tmp.join("deleted-action-root");
        let err = run_inline(
            &config.workspace_dir,
            &lang,
            &python_bin,
            &bin_dir,
            "open('must-not-exist.txt', 'w').write('bad')".to_string(),
            Some(missing_cwd),
            None,
        )
        .await
        .expect_err("missing cwd must fail closed");
        assert!(
            err.to_string().contains("failed to set worker cwd"),
            "unexpected missing-cwd error: {err}"
        );
        assert!(!tmp.join("must-not-exist.txt").exists());

        // The harness-level error must not poison framing or worker reuse.
        let out4 = run_inline(
            &config.workspace_dir,
            &lang,
            &python_bin,
            &bin_dir,
            "print('AFTER_CWD_ERROR')".to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("job after cwd error runs");
        assert_eq!(out4.stdout, "AFTER_CWD_ERROR\n");

        let spawns_after = python_spawns().await;
        assert!(
            spawns_after - spawns_before <= 1,
            "expected warm-worker reuse: {} new spawns",
            spawns_after - spawns_before
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
