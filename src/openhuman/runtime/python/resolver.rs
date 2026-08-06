//! System Python resolver.
//!
//! Walks the configured command candidates / `PATH`, probes `--version`, and
//! returns a [`SystemPython`] when the interpreter satisfies the configured
//! minimum version floor.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Parsed Python semantic version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PythonVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl PythonVersion {
    pub fn display(self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A usable Python interpreter discovered on the host.
#[derive(Debug, Clone)]
pub struct SystemPython {
    /// Absolute path to the executable.
    pub path: PathBuf,
    /// Parsed semantic version.
    pub version_info: PythonVersion,
    /// Normalized `major.minor.patch` string.
    pub version: String,
}

/// Parse a version line like `Python 3.12.4` or `3.12.4`.
pub fn parse_python_version(raw: &str) -> Option<PythonVersion> {
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix("Python ").unwrap_or(trimmed);
    let mut parts = stripped.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch = parts
        .next()
        .and_then(|segment| {
            let digits = segment
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>();
            if digits.is_empty() {
                None
            } else {
                digits.parse::<u32>().ok()
            }
        })
        .unwrap_or(0);
    Some(PythonVersion {
        major,
        minor,
        patch,
    })
}

/// Probe the host for a Python interpreter satisfying `minimum_version`.
///
/// Candidate order:
/// 1. `preferred_command` when supplied
/// 2. `python3.12`
/// 3. `python3`
/// 4. `python`
pub fn detect_system_python(
    minimum_version: &str,
    preferred_command: Option<&str>,
) -> Option<SystemPython> {
    detect_system_python_in_path(
        minimum_version,
        preferred_command,
        std::env::var_os("PATH").as_ref(),
    )
}

fn detect_system_python_in_path(
    minimum_version: &str,
    preferred_command: Option<&str>,
    path_var: Option<&OsString>,
) -> Option<SystemPython> {
    let Some(minimum) = parse_python_version(minimum_version) else {
        tracing::warn!(
            minimum_version,
            "[runtime_python::resolver] invalid minimum_version, skipping system-python probe"
        );
        return None;
    };

    for candidate in candidate_commands(preferred_command, minimum) {
        let Some(path) = resolve_candidate(&candidate, path_var) else {
            tracing::debug!(candidate, "[runtime_python::resolver] candidate not found");
            continue;
        };

        tracing::debug!(
            candidate,
            path = %path.display(),
            minimum_version = %minimum.display(),
            "[runtime_python::resolver] probing python candidate"
        );

        let Some(raw_version) = probe_python_version(&path) else {
            tracing::warn!(
                candidate,
                path = %path.display(),
                "[runtime_python::resolver] `python --version` failed; skipping candidate"
            );
            continue;
        };

        let Some(version_info) = parse_python_version(&raw_version) else {
            tracing::warn!(
                candidate,
                path = %path.display(),
                raw_version = %raw_version,
                "[runtime_python::resolver] could not parse python version output"
            );
            continue;
        };

        if version_info < minimum {
            tracing::info!(
                candidate,
                path = %path.display(),
                found = %version_info.display(),
                minimum = %minimum.display(),
                "[runtime_python::resolver] python candidate below minimum version"
            );
            continue;
        }

        let normalized = version_info.display();
        tracing::info!(
            candidate,
            path = %path.display(),
            version = %normalized,
            "[runtime_python::resolver] reusing compatible system python"
        );
        return Some(SystemPython {
            path,
            version_info,
            version: normalized,
        });
    }

    None
}

fn candidate_commands(preferred_command: Option<&str>, minimum: PythonVersion) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(preferred) = preferred_command.map(str::trim).filter(|s| !s.is_empty()) {
        candidates.push(preferred.to_string());
    }
    let minimum_specific = format!("python{}.{}", minimum.major, minimum.minor);
    for fallback in [minimum_specific.as_str(), "python3.12", "python3", "python"] {
        if !candidates.iter().any(|existing| existing == fallback) {
            candidates.push(fallback.to_string());
        }
    }
    candidates
}

fn resolve_candidate(candidate: &str, path_var: Option<&OsString>) -> Option<PathBuf> {
    let path = Path::new(candidate);
    if path.components().count() > 1 || path.is_absolute() {
        return is_executable_candidate(path).then(|| path.to_path_buf());
    }

    let path_var = path_var?;
    for dir in std::env::split_paths(path_var) {
        let base = dir.join(candidate);
        if is_executable_candidate(&base) {
            return Some(base);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{candidate}.exe"));
            if is_executable_candidate(&exe) {
                return Some(exe);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable_candidate(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && (meta.permissions().mode() & 0o111 != 0))
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_candidate(path: &Path) -> bool {
    path.is_file()
}

fn probe_python_version(path: &Path) -> Option<String> {
    use std::io::Read;
    use wait_timeout::ChildExt;

    let mut cmd = Command::new(path);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let timeout = Duration::from_secs(5);
    let status = match child.wait_timeout(timeout).ok()? {
        Some(status) => status,
        None => {
            tracing::warn!(
                path = %path.display(),
                timeout_secs = 5,
                "[runtime_python::resolver] `<bin> --version` timed out; killing process"
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
            path = %path.display(),
            status = ?status,
            stderr = %stderr_buf,
            "[runtime_python::resolver] `<bin> --version` exited non-zero"
        );
        return None;
    }

    let mut stdout_buf = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut stdout_buf);
    }
    let mut stderr_buf = String::new();
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut stderr_buf);
    }

    let combined = if stdout_buf.trim().is_empty() {
        stderr_buf.trim().to_string()
    } else {
        stdout_buf.trim().to_string()
    };
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

pub(crate) fn probe_python_version_public(path: &Path) -> Option<String> {
    probe_python_version(path)
}

#[cfg(test)]
#[path = "resolver_tests.rs"]
mod tests;
