//! Worker environment + harness-script materialisation helpers.
//!
//! Split out of `mod.rs` so the module root stays export-focused. Owns the
//! allow-listed child environment (`base_env`) and the once-per-process harness
//! write (`ensure_worker_script`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::sync::OnceCell;

/// Env vars forwarded (allow-listed) into pooled workers. Mirrors the
/// `node_exec` / shell hygiene: secrets never leak into a worker's environment;
/// `PATH` is rebuilt separately with the managed interpreter's bin dir first.
const SAFE_ENV_VARS: &[&str] = &[
    "HOME",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "USER",
    "SHELL",
    "TMPDIR",
    // Windows process creation + child command lookup after env_clear().
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
];

/// Build the allow-listed environment for a worker, with `bin_dir` prepended to
/// `PATH` so the child resolves the managed interpreter (and its tools).
pub(crate) fn base_env(bin_dir: &Path) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = Vec::new();

    let host_path = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    let path = if host_path.is_empty() {
        bin_dir.to_string_lossy().into_owned()
    } else {
        format!("{}{}{}", bin_dir.display(), sep, host_path)
    };
    env.push(("PATH".to_string(), path));

    for var in SAFE_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            env.push(((*var).to_string(), val));
        }
    }
    env
}

/// Materialise a bundled harness script into a stable per-workspace cache path
/// and return it.
async fn write_worker_script(
    workspace_dir: &Path,
    filename: &str,
    contents: &str,
) -> Result<PathBuf> {
    let root = workspace_dir.join("runtime_pool");
    tracing::debug!(dir = %root.display(), filename, "[runtime_pool] writing worker harness");
    tokio::fs::create_dir_all(&root)
        .await
        .with_context(|| format!("creating runtime_pool cache {}", root.display()))?;
    let path = root.join(filename);
    tokio::fs::write(&path, contents)
        .await
        .with_context(|| format!("writing worker script {}", path.display()))?;
    tracing::debug!(path = %path.display(), bytes = contents.len(), "[runtime_pool] worker harness ready");
    Ok(path)
}

/// Return the harness script path, writing it **once per process** (a hot-path
/// `node_exec`/`python_exec` must not touch disk on every call — the point of
/// #5106 is to *reduce* per-run cost). The script is written on the first inline
/// exec and cached; a core upgrade is a fresh process, so it re-materialises
/// then, keeping the shipped harness current.
pub(crate) async fn ensure_worker_script(
    cell: &'static OnceCell<PathBuf>,
    workspace_dir: &Path,
    filename: &str,
    contents: &str,
) -> Result<PathBuf> {
    Ok(cell
        .get_or_try_init(|| write_worker_script(workspace_dir, filename, contents))
        .await?
        .clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_env_prepends_bin_dir_to_path() {
        let env = base_env(Path::new("/managed/bin"));
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .expect("PATH present");
        assert!(
            path.starts_with("/managed/bin"),
            "bin dir must be first on PATH; got {path}"
        );
    }

    #[tokio::test]
    async fn write_worker_script_roundtrips() {
        let tmp = std::env::temp_dir().join(format!("rt-pool-test-{}", std::process::id()));
        let path = write_worker_script(&tmp, "probe.js", "console.log('hi')")
            .await
            .expect("script written");
        let read = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(read, "console.log('hi')");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }
}
