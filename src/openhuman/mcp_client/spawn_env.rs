//! Spawn-environment resolution for stdio MCP servers.
//!
//! Desktop builds launched from a GUI (Finder/`launchd` on macOS, the shell-less
//! session on Linux) inherit a **stripped** PATH — typically just
//! `/usr/bin:/bin:/usr/sbin:/sbin` — that lacks Homebrew, `/usr/local/bin`, and
//! every Node/Python version-manager shim (nvm, volta, fnm, uv). The bulk of the
//! community MCP ecosystem ships as `npx <pkg>` / `uvx <pkg>` invocations, so
//! those servers fail to spawn with a bare `ENOENT` even though the user has Node
//! installed and working in their terminal (issue #4279).
//!
//! This module reconstructs the PATH a stdio child *should* see, the same way a
//! terminal would:
//!
//! 1. Probe the user's interactive login shell (`$SHELL -ilc`) for its `$PATH`
//!    so version managers that hook into shell rc files are honoured.
//! 2. Keep well-known version-manager bin directories as a fallback for when the
//!    login-shell probe is unavailable (non-interactive sandboxes, rc files that
//!    don't export PATH, etc.).
//! 3. Merge, de-duplicated. When the probe succeeds it is authoritative — the
//!    shell PATH leads and the fallback dirs are appended only to fill gaps, so
//!    they never override the shell's chosen Node version. When the probe fails
//!    the fallback dirs lead instead.
//!
//! The result is resolved once per process and cached. [`locate_command`] and
//! [`missing_command_error`] let callers fail *before* spawn with actionable
//! guidance instead of an opaque OS error.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::OnceCell;

const PATH_SEP: char = if cfg!(windows) { ';' } else { ':' };
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(3);
const PATH_MARK_START: &str = "__OPENHUMAN_MCP_PATH_START__";
const PATH_MARK_END: &str = "__OPENHUMAN_MCP_PATH_END__";

static SPAWN_PATH: OnceCell<String> = OnceCell::const_new();

/// The PATH a stdio MCP child should inherit. Resolved once per process and
/// cached; subsequent calls clone the cached value.
pub async fn spawn_path() -> String {
    SPAWN_PATH.get_or_init(build_spawn_path).await.clone()
}

async fn build_spawn_path() -> String {
    let process_path = std::env::var("PATH").unwrap_or_default();
    let fallback = join_dirs(&version_manager_dirs());
    let login_path = login_shell_path().await;
    let resolved = merge_path_strings(order_sources(
        login_path.as_deref(),
        &process_path,
        &fallback,
    ));
    tracing::debug!(
        target: "[mcp_client::spawn_env]",
        resolved_login = login_path.is_some(),
        entries = resolved.split(PATH_SEP).count(),
        "resolved stdio MCP spawn PATH"
    );
    resolved
}

/// Probe the user's interactive login shell for its `$PATH`.
///
/// Windows GUI processes already inherit the full user PATH, so there is no
/// login-shell dance to perform there.
#[cfg(windows)]
async fn login_shell_path() -> Option<String> {
    None
}

#[cfg(not(windows))]
async fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    // Markers fence the PATH from any rc-file noise (MOTD, banners, prompts) so
    // we extract exactly the value of `$PATH` and nothing else.
    let script = format!("printf '{PATH_MARK_START}%s{PATH_MARK_END}' \"$PATH\"");
    // `kill_on_drop` so the 3s `timeout` below actually tears the shell down:
    // `timeout` only drops the `output()` future, and Tokio leaves the child
    // running otherwise — a hung rc file would outlive the timeout.
    let probe = Command::new(&shell)
        .arg("-ilc")
        .arg(&script)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();

    let stdout = match tokio::time::timeout(LOGIN_SHELL_TIMEOUT, probe).await {
        Ok(Ok(out)) if out.status.success() => out.stdout,
        Ok(Ok(out)) => {
            tracing::debug!(
                target: "[mcp_client::spawn_env]",
                shell = %shell,
                status = ?out.status.code(),
                "login-shell PATH probe exited non-zero"
            );
            return None;
        }
        Ok(Err(err)) => {
            tracing::debug!(
                target: "[mcp_client::spawn_env]",
                shell = %shell,
                error = %err,
                "login-shell PATH probe failed to run"
            );
            return None;
        }
        Err(_) => {
            tracing::debug!(
                target: "[mcp_client::spawn_env]",
                shell = %shell,
                "login-shell PATH probe timed out"
            );
            return None;
        }
    };

    parse_marked_path(&String::from_utf8_lossy(&stdout))
}

/// Extract the PATH fenced between the probe markers, ignoring surrounding
/// shell output. Returns `None` when the markers are absent or fence an empty
/// value.
fn parse_marked_path(text: &str) -> Option<String> {
    let start = text.find(PATH_MARK_START)? + PATH_MARK_START.len();
    let rest = &text[start..];
    let end = rest.find(PATH_MARK_END)?;
    let path = rest[..end].trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// Well-known version-manager bin directories that exist on disk. These are a
/// fallback for when the login-shell probe is unavailable; the probe is the
/// authoritative source when it succeeds.
fn version_manager_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        // volta and bun expose stable shim directories; nvm needs a version
        // glob. fnm is intentionally omitted — it has no stable shim dir and is
        // covered by the login-shell probe (it requires shell eval anyway).
        push_if_dir(&mut dirs, home.join(".local").join("bin"));
        push_if_dir(&mut dirs, home.join(".volta").join("bin"));
        push_if_dir(&mut dirs, home.join(".bun").join("bin"));
        push_if_dir(&mut dirs, home.join(".cargo").join("bin"));
        dirs.extend(nvm_latest_bin_dir(&home));
    }
    for fixed in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/local/sbin"] {
        push_if_dir(&mut dirs, PathBuf::from(fixed));
    }
    dirs
}

/// The `bin` directory of the highest installed nvm Node version, if any.
fn nvm_latest_bin_dir(home: &Path) -> Option<PathBuf> {
    let nvm_dir = std::env::var_os("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".nvm"));
    let versions = nvm_dir.join("versions").join("node");
    let entries = std::fs::read_dir(&versions).ok()?;

    let mut latest: Option<(Vec<u32>, PathBuf)> = None;
    for entry in entries.flatten() {
        let Some(version) = parse_semver(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let bin = entry.path().join("bin");
        if !bin.is_dir() {
            continue;
        }
        match &latest {
            Some((current, _)) if *current >= version => {}
            _ => latest = Some((version, bin)),
        }
    }
    latest.map(|(_, bin)| bin)
}

fn push_if_dir(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if dir.is_dir() {
        dirs.push(dir);
    }
}

/// Parse a dotted version (`v22.11.0`, `18.3`) into numeric components for
/// ordering. Returns `None` when any component is non-numeric.
fn parse_semver(raw: &str) -> Option<Vec<u32>> {
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let parts: Option<Vec<u32>> = stripped.split('.').map(|p| p.parse::<u32>().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

/// Order the PATH sources so the authoritative one leads.
///
/// When the login-shell probe succeeds it *is* the user's terminal environment,
/// so it wins: shell PATH, then the inherited process PATH, and the
/// version-manager fallback dirs appended only to fill in locations the shell
/// didn't already expose (never to override the shell's chosen Node version).
/// When the probe fails the fallback dirs are the primary source of
/// version-manager paths and therefore lead.
fn order_sources<'a>(login: Option<&'a str>, process: &'a str, fallback: &'a str) -> Vec<&'a str> {
    match login {
        Some(login) => vec![login, process, fallback],
        None => vec![fallback, process],
    }
}

/// Join directories into a single PATH-style string.
fn join_dirs(dirs: &[PathBuf]) -> String {
    dirs.iter()
        .map(|dir| dir.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(&PATH_SEP.to_string())
}

/// Merge PATH-style sources left-to-right, de-duplicating while preserving
/// first-seen order.
fn merge_path_strings<'a>(sources: impl IntoIterator<Item = &'a str>) -> String {
    let mut seen = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for source in sources {
        for entry in source.split(PATH_SEP) {
            push_entry(&mut out, &mut seen, entry);
        }
    }
    out.join(&PATH_SEP.to_string())
}

fn push_entry(out: &mut Vec<String>, seen: &mut HashSet<String>, entry: &str) {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return;
    }
    if seen.insert(trimmed.to_string()) {
        out.push(trimmed.to_string());
    }
}

/// Resolve `command` against `path`, mirroring how the OS resolves an
/// executable name at spawn time. Returns the resolved path, or `None` when the
/// command cannot be found.
///
/// A `command` containing a path separator is treated as a direct path
/// (matching `execvp`/`CreateProcess` semantics). A *relative* direct path is
/// resolved against `cwd` when set, mirroring the child's working directory so a
/// `command = "./server"` + `cwd` config isn't rejected.
pub fn locate_command(command: &str, path: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    if command.is_empty() {
        return None;
    }
    if has_path_separator(command) {
        let candidate = PathBuf::from(command);
        let resolved = match (candidate.is_relative(), cwd) {
            (true, Some(dir)) => dir.join(&candidate),
            _ => candidate,
        };
        return is_executable_file(&resolved).then_some(resolved);
    }
    for dir in path.split(PATH_SEP) {
        if dir.is_empty() {
            continue;
        }
        for candidate in executable_candidates(Path::new(dir).join(command)) {
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn has_path_separator(command: &str) -> bool {
    command.contains('/') || (cfg!(windows) && command.contains('\\'))
}

/// Whether `path` is a file the OS would accept as an executable, so preflight
/// matches what `spawn` will actually run. On Unix that means the execute bit is
/// set; on Windows executability is determined by extension (already enumerated
/// via `executable_candidates`/`PATHEXT`), so a regular file is sufficient.
fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return path
            .metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(windows)]
fn executable_candidates(base: PathBuf) -> Vec<PathBuf> {
    let mut candidates = vec![base.clone()];
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    for ext in exts.split(';') {
        let ext = ext.trim();
        if ext.is_empty() {
            continue;
        }
        let mut name = base.as_os_str().to_os_string();
        name.push(ext);
        candidates.push(PathBuf::from(name));
    }
    candidates
}

#[cfg(not(windows))]
fn executable_candidates(base: PathBuf) -> Vec<PathBuf> {
    vec![base]
}

/// Build an actionable error for a stdio MCP command that could not be resolved
/// on the spawn PATH. Node (`npx`/`npm`/`node`) and Python uv (`uvx`/`uv`)
/// runtimes get install guidance; anything else gets a generic PATH hint.
pub fn missing_command_error(command: &str) -> String {
    let lower = command.to_ascii_lowercase();
    let base = lower.rsplit(['/', '\\']).next().unwrap_or(lower.as_str());
    match base {
        "npx" | "npm" | "node" => format!(
            "`{command}` was not found. This MCP server needs Node.js, which doesn't \
             appear to be installed (or isn't on OpenHuman's PATH). Install Node.js from \
             https://nodejs.org and restart OpenHuman."
        ),
        "uvx" | "uv" => format!(
            "`{command}` was not found. This MCP server needs uv (Python), which doesn't \
             appear to be installed. Install it from https://docs.astral.sh/uv/ and restart \
             OpenHuman."
        ),
        _ => format!(
            "`{command}` was not found on OpenHuman's PATH. Install it (or its runtime) and \
             make sure it's available in your shell, then restart OpenHuman."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(path: &Path, executable: bool) {
        std::fs::File::create(path)
            .unwrap()
            .write_all(b"x")
            .unwrap();
        #[cfg(unix)]
        if executable {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).unwrap();
        }
        let _ = executable;
    }

    #[test]
    fn parse_marked_path_extracts_between_markers() {
        let raw = format!("banner noise\n{PATH_MARK_START}/usr/bin:/bin{PATH_MARK_END}\ntrailing");
        assert_eq!(parse_marked_path(&raw).as_deref(), Some("/usr/bin:/bin"));
    }

    #[test]
    fn parse_marked_path_none_without_markers() {
        assert_eq!(parse_marked_path("no markers here"), None);
    }

    #[test]
    fn parse_marked_path_none_when_empty() {
        let raw = format!("{PATH_MARK_START}   {PATH_MARK_END}");
        assert_eq!(parse_marked_path(&raw), None);
    }

    #[test]
    fn merge_dedups_preserving_first_seen_order() {
        let merged = merge_path_strings([
            "/opt/homebrew/bin",
            "/opt/homebrew/bin:/usr/local/bin",
            "/usr/bin:/bin",
        ]);
        let parts: Vec<&str> = merged.split(PATH_SEP).collect();
        assert_eq!(
            parts,
            vec!["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]
        );
    }

    #[test]
    fn merge_skips_empty_entries() {
        let merged = merge_path_strings(["/usr/bin::/bin:"]);
        assert_eq!(
            merged.split(PATH_SEP).collect::<Vec<_>>(),
            vec!["/usr/bin", "/bin"]
        );
    }

    #[test]
    fn probe_success_lets_shell_path_win_over_fallback() {
        // nvm default is v20 on the shell PATH but v22 is the highest install
        // (a fallback dir). The shell's choice must win; fallback only appends.
        let merged = merge_path_strings(order_sources(
            Some("/nvm/v20/bin:/usr/bin"),
            "/usr/bin:/bin",
            "/nvm/v22/bin",
        ));
        let parts: Vec<&str> = merged.split(PATH_SEP).collect();
        assert_eq!(
            parts,
            vec!["/nvm/v20/bin", "/usr/bin", "/bin", "/nvm/v22/bin"]
        );
        let shell = parts.iter().position(|p| *p == "/nvm/v20/bin").unwrap();
        let fallback = parts.iter().position(|p| *p == "/nvm/v22/bin").unwrap();
        assert!(shell < fallback, "shell PATH must precede fallback dirs");
    }

    #[test]
    fn probe_failure_leads_with_fallback_dirs() {
        let merged = merge_path_strings(order_sources(None, "/usr/bin:/bin", "/nvm/v22/bin"));
        assert_eq!(
            merged.split(PATH_SEP).collect::<Vec<_>>(),
            vec!["/nvm/v22/bin", "/usr/bin", "/bin"]
        );
    }

    #[test]
    fn join_dirs_uses_path_separator() {
        let joined = join_dirs(&[PathBuf::from("/a/bin"), PathBuf::from("/b/bin")]);
        assert_eq!(
            joined.split(PATH_SEP).collect::<Vec<_>>(),
            vec!["/a/bin", "/b/bin"]
        );
    }

    #[test]
    fn parse_semver_handles_prefix_and_arity() {
        assert_eq!(parse_semver("v22.11.0"), Some(vec![22, 11, 0]));
        assert_eq!(parse_semver(" 18.3 "), Some(vec![18, 3]));
        assert_eq!(parse_semver("garbage"), None);
        assert_eq!(parse_semver("22.x"), None);
    }

    #[test]
    fn locate_command_resolves_via_path_and_direct() {
        let dir = std::env::temp_dir().join(format!("oh-spawnenv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("oh-fake-cmd");
        write_file(&exe, true);
        let path = dir.to_string_lossy().to_string();

        assert!(locate_command("oh-fake-cmd", &path, None).is_some());
        assert!(locate_command("definitely-missing-xyz", &path, None).is_none());
        assert!(locate_command(&exe.to_string_lossy(), "", None).is_some());
        assert!(locate_command("", &path, None).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locate_command_resolves_relative_path_against_cwd() {
        let dir = std::env::temp_dir().join(format!("oh-spawnenv-cwd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_file(&dir.join("rel-server"), true);

        // A relative `command` resolves against the configured cwd, not the
        // process directory — mirroring where the child actually spawns.
        assert!(locate_command("./rel-server", "", Some(&dir)).is_some());
        // Without a cwd it resolves against the process dir and isn't found.
        assert!(locate_command("./rel-server", "", None).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn locate_command_rejects_non_executable_file() {
        let dir = std::env::temp_dir().join(format!("oh-spawnenv-noexec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_file(&dir.join("plain-file"), false);
        let path = dir.to_string_lossy().to_string();

        // A regular, non-executable file must not satisfy preflight — spawn
        // would reject it too.
        assert!(locate_command("plain-file", &path, None).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_command_error_guides_node_runtimes() {
        assert!(missing_command_error("npx").contains("Node.js"));
        assert!(missing_command_error("/usr/local/bin/node").contains("Node.js"));
        assert!(missing_command_error("npm").contains("Node.js"));
    }

    #[test]
    fn missing_command_error_guides_uv_runtime() {
        assert!(missing_command_error("uvx").contains("uv"));
        assert!(missing_command_error("uv").contains("uv"));
    }

    #[test]
    fn missing_command_error_generic_for_others() {
        let msg = missing_command_error("docker");
        assert!(msg.contains("docker"));
        assert!(msg.contains("PATH"));
    }

    #[tokio::test]
    async fn spawn_path_is_non_empty_and_cached() {
        let first = spawn_path().await;
        let second = spawn_path().await;
        assert!(!first.is_empty());
        assert_eq!(first, second);
    }
}
