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

    // `npm_exec` rides on the same resolved toolchain. On distros that
    // package `nodejs` and `npm` separately (Debian/Ubuntu default,
    // Alpine's `nodejs-current`, some NixOS setups) the `node` binary can
    // be present without `npm`. If we cached `NodeSource::System` here
    // every `npm_exec` call would break with an obscure error. Require a
    // usable `npm --version` probe before accepting the system toolchain;
    // on failure, return `None` so the managed download path takes over.
    let Some(npm_path) = which_npm() else {
        tracing::info!(
            node_path = %path.display(),
            "[node_runtime::resolver] compatible system node found but `npm` is missing on PATH — falling back to managed runtime"
        );
        return None;
    };

    if probe_subcommand_version(&npm_path, "npm").is_none() {
        tracing::warn!(
            npm_path = %npm_path.display(),
            "[node_runtime::resolver] `npm --version` failed; falling back to managed runtime"
        );
        return None;
    }

    let normalized = version.trim_start_matches('v').trim().to_string();
    tracing::info!(
        path = %path.display(),
        npm_path = %npm_path.display(),
        version = %normalized,
        "[node_runtime::resolver] reusing compatible system node (npm verified)"
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
///
/// Unix command lookup skips non-executable entries. A non-executable
/// placeholder earlier in `PATH` (e.g. an unprivileged `node` shim left by
/// a failed install) would otherwise mask a valid later install and force
/// the managed runtime download. We mirror the shell behaviour by checking
/// the execute bit before returning.
fn which_node() -> Option<PathBuf> {
    let exe_name = format!("node{}", std::env::consts::EXE_SUFFIX);
    which_exe(&exe_name)
}

/// Locate an `npm` binary on `PATH`. Applies the same execute-bit filter
/// as [`which_node`]. On Windows we look for `npm.cmd` first (the official
/// installer ships a batch shim; there is no `npm.exe`) and fall back to
/// `npm` for unusual setups that expose a bare binary.
fn which_npm() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(p) = which_exe("npm.cmd") {
            return Some(p);
        }
        which_exe("npm")
    }
    #[cfg(not(windows))]
    {
        which_exe("npm")
    }
}

/// `PATH` search helper shared by `which_node` / `which_npm`. Applies the
/// platform-specific executability check so a non-executable placeholder
/// earlier in `PATH` doesn't shadow a valid later entry.
fn which_exe(exe_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe_name);
        if is_executable_candidate(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable_candidate(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && (meta.permissions().mode() & 0o111 != 0))
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_candidate(path: &std::path::Path) -> bool {
    // On Windows, the `.exe` suffix already encodes executability for the
    // loader; any regular file matching `node.exe` is a valid candidate.
    path.is_file()
}

/// Invoke `<path> --version` with a real 5-second timeout and return the raw
/// version string on success. The timeout guards against a broken shim on
/// `PATH` hanging the bootstrap indefinitely.
fn probe_node_version(path: &std::path::Path) -> Option<String> {
    probe_subcommand_version(path, "node")
}

/// Same semantics as [`probe_node_version`], but usable for arbitrary
/// toolchain binaries. `label` is only used for log attribution.
fn probe_subcommand_version(path: &std::path::Path, label: &str) -> Option<String> {
    use std::io::Read;
    use wait_timeout::ChildExt;

    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let timeout = Duration::from_secs(5);
    let status = match child.wait_timeout(timeout).ok()? {
        Some(s) => s,
        None => {
            tracing::warn!(
                path = %path.display(),
                label,
                timeout_secs = 5,
                "[node_runtime::resolver] `<bin> --version` timed out; killing process"
            );
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    if !status.success() {
        let mut stderr_buf = String::new();
        if let Some(mut s) = child.stderr.take() {
            let _ = s.read_to_string(&mut stderr_buf);
        }
        tracing::debug!(
            status = ?status,
            label,
            stderr = %stderr_buf,
            "[node_runtime::resolver] `<bin> --version` exited non-zero"
        );
        return None;
    }

    let mut stdout_buf = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut stdout_buf);
    }
    let trimmed = stdout_buf.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
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
