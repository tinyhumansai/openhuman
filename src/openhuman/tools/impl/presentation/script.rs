//! Embedded `generate_pptx.py` script + lazy tempfile materialiser.
//!
//! The script is shipped into the binary via [`include_str!`] so the
//! tool never depends on a packaged-resource lookup that would differ
//! between `cargo run`, the Tauri shell, and the standalone
//! `openhuman-core` binary. First call materialises the script to a
//! per-process temp directory; subsequent calls reuse the cached path.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::sync::OnceCell;

/// The bundled Python source. Compiled into the binary.
pub(super) const GENERATE_PPTX_PY: &str = include_str!("generate_pptx.py");

/// Per-process cache of the materialised script path. We do NOT keep
/// a `TempDir` handle because we want the script to outlive any
/// individual tool call within the process — the tempfile is cleaned
/// up by the OS at process exit.
static MATERIALISED_SCRIPT: OnceCell<PathBuf> = OnceCell::const_new();

/// Resolve a filesystem path that points at a copy of the bundled
/// `generate_pptx.py` script, materialising it on first call.
pub(super) async fn materialise_script() -> Result<PathBuf> {
    MATERIALISED_SCRIPT
        .get_or_try_init(write_script_once)
        .await
        .cloned()
}

async fn write_script_once() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("openhuman-presentation");
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("creating script tempdir {}", dir.display()))?;
    // Unpredictable per-process filename + O_EXCL open so a
    // pre-created path or dangling symlink in the shared temp dir
    // cannot redirect or clobber the materialised script.
    let path = dir.join(format!(
        "generate_pptx-{}-{}.py",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await
        .with_context(|| format!("creating bundled script file {}", path.display()))?;
    file.write_all(GENERATE_PPTX_PY.as_bytes())
        .await
        .with_context(|| format!("writing bundled script to {}", path.display()))?;
    file.flush()
        .await
        .with_context(|| format!("flushing bundled script to {}", path.display()))?;
    tracing::debug!(
        path = %path.display(),
        bytes = GENERATE_PPTX_PY.len(),
        "[presentation::script] materialised generate_pptx.py"
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_script_has_expected_shape() {
        assert!(GENERATE_PPTX_PY.contains("def main()"));
        assert!(GENERATE_PPTX_PY.contains("from pptx import Presentation"));
        assert!(GENERATE_PPTX_PY.contains("--output"));
    }

    #[tokio::test]
    async fn materialise_returns_existing_file() {
        let path = materialise_script().await.expect("script materialises");
        assert!(path.exists(), "script not at {path:?}");
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(contents.contains("def main()"));
        // Second call returns the same path (OnceCell caching).
        let again = materialise_script().await.unwrap();
        assert_eq!(path, again);
    }
}
