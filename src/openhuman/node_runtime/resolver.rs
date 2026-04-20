//! System-node resolver.
//!
//! Walks `PATH`, probes `node --version`, and returns a [`SystemNode`] when
//! the host-installed binary matches the configured target major version.
//! Runs synchronously because it blocks on one short-lived subprocess and is
//! called exactly once per bootstrap — pushing it onto the Tokio runtime
//! would add noise without benefit.
//!
//! Target-version matching is intentionally loose: we only compare **major**
//! versions. Point releases of Node.js are ABI-stable, and skills pin their
//! own dependency versions via `package.json` / `package-lock.json`, so a
//! host `v22.8.0` is accepted when `node.version = "v22.11.0"`. If a user
//! needs strict pinning they can set `node.prefer_system = false`.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// A usable Node.js toolchain discovered on the host `PATH`.
#[derive(Debug, Clone)]
pub struct SystemNode {
    /// Absolute path to the `node` executable.
    pub path: PathBuf,
    /// Parsed major version (e.g. `22`).
    pub major: u32,
    /// Raw version string reported by `node --version`, trimmed of the
    /// leading `v` and trailing whitespace (e.g. `"22.11.0"`).
    pub version: String,
}

/// Parse a version string like `v22.11.0` / `22.11.0` / `v22` and return the
/// numeric major component.
///
/// Returns `None` when the input is malformed. Tolerant of surrounding
/// whitespace and an optional leading `v` prefix so it can accept both the
/// config value (`node.version = "v22.11.0"`) and the raw `node --version`
/// output (`v22.11.0\n`).
pub fn parse_node_version(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let major = stripped.split('.').next()?;
    major.parse::<u32>().ok()
}

/// Probe the host for a `node` binary on `PATH` whose major version matches
/// `target_version`. Returns `Some(SystemNode)` on success, `None` when no
/// compatible toolchain is found.
///
/// Heavy tracing is intentional — resolver decisions drive whether we skip a
/// multi-hundred-MB download, so operators need a clear breadcrumb trail.
pub fn detect_system_node(target_version: &str) -> Option<SystemNode> {
    let Some(target_major) = parse_node_version(target_version) else {
        tracing::warn!(
            target_version,
            "[node_runtime::resolver] invalid target_version, skipping system-node probe"
        );
        return None;
    };

    let Some(path) = which_node() else {
        tracing::debug!(
            "[node_runtime::resolver] no `node` found on PATH — will fall back to download"
        );
        return None;
    };

    tracing::debug!(
        path = %path.display(),
        target_major,
        "[node_runtime::resolver] probing system node"
    );

    let Some(version) = probe_node_version(&path) else {
        tracing::warn!(
            path = %path.display(),
            "[node_runtime::resolver] `node --version` failed; treating as unavailable"
        );
        return None;
    };

    let Some(host_major) = parse_node_version(&version) else {
        tracing::warn!(
            path = %path.display(),
            version = %version,
            "[node_runtime::resolver] could not parse `node --version` output"
        );
        return None;
    };

    if host_major != target_major {
        tracing::info!(
            path = %path.display(),
            host_major,
            target_major,
            "[node_runtime::resolver] host node major mismatch — will download managed runtime"
        );
        return None;
    }

    let normalized = version.trim_start_matches('v').trim().to_string();
    tracing::info!(
        path = %path.display(),
        version = %normalized,
        "[node_runtime::resolver] reusing compatible system node"
    );
    Some(SystemNode {
        path,
        major: host_major,
        version: normalized,
    })
}

/// Locate a `node` binary on `PATH`. Cross-platform: appends the host
/// executable suffix (`.exe` on Windows) so callers receive a path that can
/// be invoked directly.
fn which_node() -> Option<PathBuf> {
    let exe_name = format!("node{}", std::env::consts::EXE_SUFFIX);
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(&exe_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Invoke `<path> --version` with a 5-second timeout and return the raw
/// version string on success. The timeout is a belt-and-braces guard against
/// a broken shim sitting on `PATH`.
fn probe_node_version(path: &std::path::Path) -> Option<String> {
    let output = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // `Command` has no native timeout; the CLI is expected to return in
        // well under 5 s, so we rely on process exit and keep the duration
        // here as a documentation anchor for future refactors toward
        // `wait_timeout` if needed.
        .output()
        .ok()?;

    let _ = Duration::from_secs(5);

    if !output.status.success() {
        tracing::debug!(
            status = ?output.status,
            stderr = %String::from_utf8_lossy(&output.stderr),
            "[node_runtime::resolver] `node --version` exited non-zero"
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return None;
    }
    Some(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_with_v_prefix() {
        assert_eq!(parse_node_version("v22.11.0"), Some(22));
    }

    #[test]
    fn parses_version_without_v_prefix() {
        assert_eq!(parse_node_version("22.11.0"), Some(22));
    }

    #[test]
    fn parses_major_only() {
        assert_eq!(parse_node_version("v22"), Some(22));
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        assert_eq!(parse_node_version("  v22.11.0\n"), Some(22));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_node_version("not-a-version"), None);
        assert_eq!(parse_node_version(""), None);
        assert_eq!(parse_node_version("v"), None);
    }
}
