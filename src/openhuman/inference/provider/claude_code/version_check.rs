//! Locate the `claude` CLI binary and verify it meets `MIN_CLI_VERSION`.
//!
//! We rely on `claude --version`, which prints a line of the form:
//!   `2.0.4 (Claude Code)`
//! The first whitespace-delimited token is the semver string we compare
//! against [`MIN_CLI_VERSION`].

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::types::{CliStatus, MIN_CLI_VERSION};

/// How long the login-shell probe may take before it is abandoned.
const LOGIN_SHELL_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Keep a stale or broken fallback binary from blocking resolution forever.
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Locate the `claude` CLI binary on `PATH`.
///
/// Resolution order:
/// 1. `OPENHUMAN_CLAUDE_CLI` env override (tests / power users / a fixed path).
/// 2. `PATH` search.
/// 3. Well-known absolute install locations ([`well_known_candidates`]).
///
/// Step 3 exists because a macOS app launched from Finder/Dock inherits only
/// the stripped launchd `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), which never
/// contains the native installer's `~/.local/bin` — so a PATH-only lookup
/// reports the CLI "not installed" even though it is present. (Terminal
/// launches inherit the shell `PATH` and hit step 2, so this only bites GUI
/// launches.)
pub fn resolve_binary() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("OPENHUMAN_CLAUDE_CLI") {
        let p = PathBuf::from(explicit);
        if p.exists() {
            return Some(p);
        }
    }
    which_on_path("claude").or_else(well_known_install)
}

/// Fallback locations for the `claude` CLI, probed when `PATH` does not carry
/// it.
///
/// A macOS app launched from Finder/Dock inherits `launchd`'s minimal `PATH`
/// (`/usr/bin:/bin:/usr/sbin:/sbin`), **not** the login shell's — so the same
/// install that resolves fine from a terminal-launched build reports
/// `NotInstalled` in the shipped app. The npm-global, Homebrew and native
/// installer locations below cover every documented install route; a
/// version-manager layout (nvm, asdf, mise) is not on that list and is picked
/// up by the time-boxed login-shell probe that follows it.
fn well_known_install() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut outdated = None;
    for candidate in well_known_candidates(home.as_deref()) {
        if candidate.is_file() {
            match version_probe_version(&candidate) {
                Some(version) if !version_lt(&version, MIN_CLI_VERSION) => {
                    log::debug!(
                        "[claude-code][version] resolved off-PATH candidate path={}",
                        candidate.display()
                    );
                    return Some(candidate);
                }
                Some(_) => {
                    outdated.get_or_insert(candidate);
                }
                None => {}
            }
        }
    }

    outdated.or_else(login_shell_lookup)
}

/// Check that a fallback is an executable Claude CLI, rather than merely a
/// stale path left behind by an installer or migration. `probe()` performs
/// the authoritative version check after resolution; this lightweight probe
/// only lets resolution continue to later candidates when this one cannot
/// answer `--version` at all.
fn version_probe_version(path: &Path) -> Option<String> {
    let path_env = super::driver::child_path_with_user_bins(path);
    match bounded_version_probe_with_path(path, VERSION_PROBE_TIMEOUT, Some(&path_env)) {
        Ok(Some(output)) => output
            .status
            .success()
            .then(|| parse_version(&String::from_utf8_lossy(&output.stdout)))?,
        Ok(None) => None,
        Err(err) => {
            log::debug!(
                "[claude-code][version] skipping unusable fallback path={} err={err}",
                path.display()
            );
            None
        }
    }
}

/// Run a fallback binary's `--version` probe with a hard bound.
///
/// This is deliberately synchronous because fallback resolution is synchronous.
/// A timed-out child is killed and reaped before returning, so repeated turns do
/// not leak processes or leave a probe holding a worker thread indefinitely.
fn bounded_version_probe(
    path: &Path,
    budget: Duration,
) -> std::io::Result<Option<std::process::Output>> {
    bounded_version_probe_with_path(path, budget, None)
}

fn bounded_version_probe_with_path(
    path: &Path,
    budget: Duration,
    path_env: Option<&std::ffi::OsStr>,
) -> std::io::Result<Option<std::process::Output>> {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(path_env) = path_env {
        command.env("PATH", path_env);
    }
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    bounded_child_output(command, budget, path)
}

/// Collect child output without allowing inherited pipes from descendants to
/// defeat the process deadline.
fn bounded_child_output(
    mut command: Command,
    budget: Duration,
    path: &Path,
) -> std::io::Result<Option<std::process::Output>> {
    use std::io::Read;
    use std::sync::mpsc;

    let mut child = command.spawn()?;
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let mut stderr = child.stderr.take().expect("stderr was piped");
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = stdout_tx.send(bytes);
    });
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        let _ = stderr_tx.send(bytes);
    });

    let deadline = std::time::Instant::now() + budget;
    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;
    loop {
        if status.is_none() {
            status = child.try_wait()?;
        }
        if stdout.is_none() {
            stdout = stdout_rx.try_recv().ok();
        }
        if stderr.is_none() {
            stderr = stderr_rx.try_recv().ok();
        }
        if let (Some(status), Some(stdout), Some(stderr)) = (status, stdout, stderr) {
            return Ok(Some(std::process::Output {
                status,
                stdout,
                stderr,
            }));
        }
        if std::time::Instant::now() >= deadline {
            #[cfg(unix)]
            let _ = unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) };
            let _ = child.kill();
            let _ = child.wait();
            log::debug!(
                "[claude-code][version] fallback version probe timed out path={} after {:?}",
                path.display(),
                budget
            );
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Ask the user's login shell where `claude` lives — last resort, time-boxed.
///
/// The fixed list above cannot cover a version-manager layout: nvm puts an
/// npm-global binary under `~/.nvm/versions/node/<version>/bin`, and asdf/mise
/// resolve through shims whose path is decided by the shell's own init. Those
/// are real installs, so dropping this fallback would regress users who could
/// resolve the CLI before.
///
/// It is bounded because `-lc` sources the user's rc files, and an rc file that
/// blocks — on a prompt, on a slow network call — would otherwise hang provider
/// construction forever with no diagnostic. Two seconds is far longer than
/// `command -v` needs and far shorter than a user will wait.
///
/// `command -v` rather than `which`: it is POSIX-builtin and resolves the way
/// the user's own terminal would. A shell *function* named `claude` (a common
/// wrapper) makes it print the function body instead of a path, so anything
/// that is not an existing file is discarded rather than handed to
/// `Command::new`.
fn login_shell_lookup() -> Option<PathBuf> {
    // Resolved at most ONCE per process, and this is load-bearing, not an
    // optimisation. `probe()` is uncached and runs on every turn build, on the
    // tokio worker driving that turn (`TurnModelSource::build` is sync all the
    // way down). Without this cache a machine whose shell profile is slow would
    // pay the full budget on every turn AND abandon one worker thread plus one
    // shell process each time, unbounded, for the life of the app. Cached, the
    // worst case is one stalled thread and one 2s wait, once.
    //
    // Only this fallback is cached — not `probe()` — so a user who installs the
    // CLI onto `PATH`, or into any of the well-known directories above, is
    // still picked up on the next turn without a restart. Those are re-probed
    // every time; it is the shell question alone that is asked once.
    //
    // The accepted cost is a sticky negative: a user whose CLI arrives *only*
    // through a version manager (nvm/asdf/mise) mid-session keeps the cached
    // `None` until the app restarts. That is the deliberate trade — the
    // alternative is re-asking a shell that already proved slow, on every turn,
    // abandoning a thread and a shell process each time with no bound. A
    // restart is a fair price for the rarer half of an already-rare path, and a
    // removed or downgraded binary is NOT affected: `probe()` still runs
    // `--version` against the cached path on every turn, so it degrades to
    // `Unusable`/`NotInstalled` with the right message rather than going stale.
    static RESOLVED: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

    RESOLVED
        .get_or_init(|| {
            if cfg!(windows) {
                return None;
            }
            let shell = std::env::var("SHELL")
                .ok()
                .filter(|s| !s.trim().is_empty())?;
            login_shell_lookup_with(&shell, LOGIN_SHELL_PROBE_TIMEOUT)
        })
        .clone()
}

/// The probe itself, with the shell and the budget passed in.
///
/// Split out so the timeout branch is testable: a test can point this at a
/// script that never returns and assert it gives up, which is the whole reason
/// the bound exists. Reading `SHELL` inside would have forced an env-mutating
/// test that races every other test in the binary.
fn login_shell_lookup_with(shell: &str, budget: Duration) -> Option<PathBuf> {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let mut command = Command::new(shell);
    command
        .args(["-lc", "command -v claude"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = match bounded_child_output(command, budget, Path::new(shell)) {
        Ok(Some(output)) => output,
        Ok(None) => {
            log::warn!(
                "[claude-code][version] login shell probe timed out after {:?}; a slow or blocking shell profile can cause this",
                budget
            );
            return None;
        }
        Err(err) => {
            log::debug!("[claude-code][version] login shell probe failed err={err}");
            return None;
        }
    };

    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    path.is_file().then(|| {
        log::debug!(
            "[claude-code][version] resolved via login shell path={}",
            path.display()
        );
        path
    })
}

/// The ordered fallback candidates, split out so the list is unit-testable
/// without mutating the process environment.
fn well_known_candidates(home: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = home {
        for suffix in [
            ".local/bin/claude",
            "bin/claude",
            ".claude/local/claude",
            ".bun/bin/claude",
            ".volta/bin/claude",
            "Library/pnpm/claude",
            ".npm-global/bin/claude",
        ] {
            candidates.push(home.join(suffix));
        }
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/claude"));
    candidates.push(PathBuf::from("/usr/local/bin/claude"));
    candidates
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path_var) {
        if cfg!(windows) {
            for ext in &exts {
                let candidate = dir.join(format!("{name}{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        } else {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Probe the `claude` CLI and return its status.
pub fn probe() -> CliStatus {
    let Some(path) = resolve_binary() else {
        log::debug!("[claude-code][version] no `claude` binary on PATH");
        return CliStatus::NotInstalled;
    };
    let path_str = path.display().to_string();

    let mut command = Command::new(&path);
    command
        .arg("--version")
        .env("PATH", super::driver::child_path_with_user_bins(&path))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = match bounded_child_output(command, VERSION_PROBE_TIMEOUT, &path) {
        Ok(Some(o)) => o,
        Ok(None) => {
            return CliStatus::Unusable {
                path: path_str,
                reason: format!("version probe timed out after {VERSION_PROBE_TIMEOUT:?}"),
            };
        }
        Err(e) => {
            log::warn!("[claude-code][version] spawn failed path={path_str} err={e}");
            return CliStatus::Unusable {
                path: path_str,
                reason: format!("spawn failed: {e}"),
            };
        }
    };

    if !output.status.success() {
        return CliStatus::Unusable {
            path: path_str,
            reason: format!(
                "non-zero exit {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        };
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = match parse_version(&stdout) {
        Some(v) => v,
        None => {
            return CliStatus::Unusable {
                path: path_str,
                reason: format!("could not parse version from: {stdout:?}"),
            }
        }
    };

    if version_lt(&version, MIN_CLI_VERSION) {
        CliStatus::Outdated {
            version,
            min_required: MIN_CLI_VERSION.to_string(),
            path: path_str,
        }
    } else {
        CliStatus::Ok {
            version,
            path: path_str,
        }
    }
}

fn parse_version(stdout: &str) -> Option<String> {
    stdout
        .split_whitespace()
        .next()
        .filter(|tok| tok.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|s| s.to_string())
}

/// Numeric semver compare. Returns true when `a < b`.
/// Pre-release suffixes (`-rc.1`) are stripped before comparison.
fn version_lt(a: &str, b: &str) -> bool {
    let pa = parts(a);
    let pb = parts(b);
    pa < pb
}

fn parts(v: &str) -> (u32, u32, u32) {
    let core = v.split('-').next().unwrap_or(v);
    let mut it = core.split('.').map(|s| s.parse::<u32>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

#[cfg(test)]
#[path = "version_check_tests.rs"]
mod tests;
