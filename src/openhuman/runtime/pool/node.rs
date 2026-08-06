//! Node.js pool backend: resolve the interpreter, materialise the JS harness,
//! and submit inline jobs to the shared [`LangPool`](super::pool::LangPool).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::OnceCell;

use super::pool;
use super::types::{PoolExecOutcome, PoolLang, PoolSettings};
use super::worker::WorkerLaunch;
use crate::openhuman::config::{RuntimePoolConfig, RuntimePoolLangConfig};

/// The bundled Node worker harness (runs each inline job in an isolated
/// `worker_thread`, capturing its stdout/stderr and honouring a soft deadline).
const WORKER_JS: &str = include_str!("pool_worker.js");

/// Written once per process (see [`super::ensure_worker_script`]).
static NODE_SCRIPT: OnceCell<PathBuf> = OnceCell::const_new();

/// Whether inline `node` jobs should route through the pool. `node.enabled` is
/// already implied by the tool only being constructed when the node runtime is
/// enabled, so only the pool switches are checked here.
pub fn enabled(pool: &RuntimePoolConfig) -> bool {
    // Node defaults ON: each job runs in an isolated worker_thread, so reuse is
    // safe (fresh module graph + globals per job).
    pool.enabled && pool.node.is_enabled(true)
}

/// Run inline JavaScript on a pooled, warm `node` worker.
///
/// `node_bin` / `bin_dir` come from the caller's already-resolved
/// [`ResolvedNode`](crate::openhuman::runtime::node::ResolvedNode). `workspace_dir`
/// and `lang_cfg` are injected at tool construction so this hot path never
/// re-reads config or re-writes the harness. `cwd` is the job's working
/// directory; `timeout` is the soft per-job deadline (`None` ⇒ run to completion).
pub async fn run_inline(
    workspace_dir: &Path,
    lang_cfg: &RuntimePoolLangConfig,
    node_bin: &Path,
    bin_dir: &Path,
    code: String,
    cwd: Option<PathBuf>,
    timeout: Option<Duration>,
) -> Result<PoolExecOutcome, super::pool::PoolRunError> {
    let script =
        super::ensure_worker_script(&NODE_SCRIPT, workspace_dir, "pool_worker.js", WORKER_JS)
            .await
            .map_err(super::pool::PoolRunError::PreDispatch)?
            .to_string_lossy()
            .into_owned();

    let env = super::base_env(bin_dir);

    let launch = WorkerLaunch {
        lang: PoolLang::Node,
        bin: node_bin.to_path_buf(),
        // `--experimental-vm-modules` lets the harness root dynamic `import()` at
        // the job cwd (parity with `node -e`); the flag propagates to the per-job
        // worker_thread via inherited `execArgv`.
        args: vec![
            "--experimental-vm-modules".to_string(),
            "--experimental-import-meta-resolve".to_string(),
            script,
        ],
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

    /// Resolve the host `node` binary + its bin dir, or `None` to skip.
    fn system_node() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        let out = std::process::Command::new("node")
            .args(["-e", "process.stdout.write(process.execPath)"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let node_bin = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        let bin_dir = node_bin.parent()?.to_path_buf();
        Some((node_bin, bin_dir))
    }

    async fn node_spawns() -> u64 {
        all_stats()
            .await
            .into_iter()
            .find(|(lang, _)| *lang == PoolLang::Node)
            .map(|(_, stats)| stats.worker_spawns)
            .unwrap_or(0)
    }

    /// End-to-end: two inline jobs run on the pool, share ONE warm worker, and
    /// surface stdout / exit codes exactly like the legacy path. Skips when no
    /// system `node` is available (keeps CI hermetic on node-less runners).
    #[tokio::test]
    async fn pooled_node_runs_inline_and_reuses_worker() {
        let Some((node_bin, bin_dir)) = system_node() else {
            eprintln!("[runtime_pool] test skipped: no system node on PATH");
            return;
        };
        let tmp = std::env::temp_dir().join(format!("rt-pool-node-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let mut config = Config::default();
        config.workspace_dir = tmp.clone();
        config.runtime_pool.node.max_workers = 1;
        config.runtime_pool.node.recycle_after_jobs = 0; // no recycle mid-test
        let lang = config.runtime_pool.node.clone();

        let spawns_before = node_spawns().await;

        let out1 = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            "console.log(JSON.stringify({ v: 6 * 7 }))".to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("job 1 runs");
        assert!(out1.success(), "job 1 should succeed: {out1:?}");
        assert!(
            out1.stdout.contains("\"v\":42"),
            "stdout was {:?}",
            out1.stdout
        );

        let out2 = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            "throw new Error('nope')".to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("job 2 runs");
        assert!(!out2.success(), "throwing job should fail");
        assert!(out2.stderr.contains("nope"), "stderr was {:?}", out2.stderr);

        // cwd correctness: relative fs must resolve against the job's cwd, not
        // the worker process's launch dir. Guards the host-chdir fix (a worker
        // thread cannot chdir itself). Regression here = broken action sandbox.
        std::fs::write(tmp.join("probe.txt"), "REL_OK").unwrap();
        let out3 = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            "const fs=require('fs'); process.stdout.write(fs.readFileSync('./probe.txt','utf8'))"
                .to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("job 3 runs");
        assert!(out3.success(), "cwd-relative read should succeed: {out3:?}");
        assert_eq!(out3.stdout, "REL_OK", "relative read resolved wrong cwd");

        // fd-level writes must never share the protocol transport. This forged
        // frame uses the real request id; on stdout-based framing Rust would
        // accept it instead of the harness response and desynchronise the next
        // job.
        let out4 = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            r#"
const fs = require('fs');
const { workerData } = require('worker_threads');
fs.writeSync(1, JSON.stringify({
  id: workerData.id,
  ok: true,
  stdout: 'FORGED',
  stderr: '',
  exit_code: 0,
  elapsed_ms: 0
}) + '\n');
console.log('REAL_RESPONSE');
"#
            .to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("fd-level output job runs");
        assert!(
            out4.success(),
            "fd-level output job should succeed: {out4:?}"
        );
        assert_eq!(out4.stdout, "REAL_RESPONSE\n");

        // Bare dynamic imports retain node -e semantics rather than being
        // rewritten to a cwd-relative file URL.
        let out5 = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            "const fs = await import('fs'); console.log(typeof fs.readFileSync)".to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("bare import job runs");
        assert!(out5.success(), "bare import should succeed: {out5:?}");
        assert_eq!(out5.stdout, "function\n");

        // ESM-only packages expose an `import` condition but no CommonJS
        // `require` condition. Bare imports must therefore use Node's ESM
        // resolver rooted at the job cwd.
        let esm_package = tmp.join("node_modules/esm-only");
        std::fs::create_dir_all(&esm_package).unwrap();
        std::fs::write(
            esm_package.join("package.json"),
            r#"{"name":"esm-only","type":"module","exports":{"import":"./index.mjs"}}"#,
        )
        .unwrap();
        std::fs::write(
            esm_package.join("index.mjs"),
            "export const marker = 'ESM_ONLY_OK';",
        )
        .unwrap();
        let esm_out = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            "const { marker } = await import('esm-only'); console.log(marker)".to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("ESM-only package import runs");
        assert!(
            esm_out.success(),
            "ESM-only package import should succeed: {esm_out:?}"
        );
        assert_eq!(esm_out.stdout, "ESM_ONLY_OK\n");

        // vm dynamic-import hooks receive attributes separately; forward them
        // so JSON modules preserve legacy node -e behavior.
        std::fs::write(tmp.join("data.json"), r#"{"answer":42}"#).unwrap();
        let json_out = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            "const data = await import('./data.json', { with: { type: 'json' } }); console.log(data.default.answer)"
                .to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("JSON import with attributes runs");
        assert!(
            json_out.success(),
            "JSON import with attributes should succeed: {json_out:?}"
        );
        assert_eq!(json_out.stdout, "42\n");

        // User warnings are tool output, not harness noise. Never suppress them
        // globally on the pooled worker.
        let warning_out = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            "process.emitWarning('POOL_WARNING')".to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("warning job runs");
        assert!(warning_out.success(), "warning job failed: {warning_out:?}");
        assert!(
            warning_out.stderr.contains("POOL_WARNING"),
            "user warning was hidden: {:?}",
            warning_out.stderr
        );

        // The protocol never shares fd 0 with user code. Legacy Command::output
        // supplies EOF on stdin, so pooled code must do the same rather than
        // blocking on or consuming the next NDJSON request.
        let stdin_out = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            "const fs=require('fs'); console.log(JSON.stringify(fs.readFileSync(0,'utf8')))"
                .to_string(),
            Some(tmp.clone()),
            Some(Duration::from_secs(2)),
        )
        .await
        .expect("stdin EOF job runs");
        assert!(stdin_out.success(), "stdin should be EOF: {stdin_out:?}");
        assert_eq!(stdin_out.stdout, "\"\"\n");

        // A missing cwd is a harness error. Running in the worker's inherited
        // cwd would escape the requested action root and diverge from legacy
        // Command::current_dir behavior.
        let missing_cwd = tmp.join("deleted-action-root");
        let err = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            "require('fs').writeFileSync('must-not-exist.txt', 'bad')".to_string(),
            Some(missing_cwd.clone()),
            None,
        )
        .await
        .expect_err("missing cwd must fail closed");
        assert!(
            err.to_string().contains("failed to set worker cwd"),
            "unexpected missing-cwd error: {err}"
        );
        assert!(!tmp.join("must-not-exist.txt").exists());

        // The harness-level cwd error must not poison framing for the next job.
        let out6 = run_inline(
            &config.workspace_dir,
            &lang,
            &node_bin,
            &bin_dir,
            "console.log('AFTER_CWD_ERROR')".to_string(),
            Some(tmp.clone()),
            None,
        )
        .await
        .expect("job after cwd error runs");
        assert_eq!(out6.stdout, "AFTER_CWD_ERROR\n");

        // At most one NEW worker spawned for all jobs ⇒ the warm worker was
        // reused. Measured as a delta so prior global pool state can't skew it.
        let spawns_after = node_spawns().await;
        assert!(
            spawns_after - spawns_before <= 1,
            "expected warm-worker reuse: {} new spawns",
            spawns_after - spawns_before
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
