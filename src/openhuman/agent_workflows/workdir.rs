//! Working-directory context providers.
//!
//! Builds a compact, agent-readable block describing properties of the current
//! working directory (git branch / status / recent log). Extensible: future
//! providers (project type, language, etc.) plug into [`working_dir_context`].

use std::path::Path;

/// Build the working-dir context block for the requested `providers`. Returns
/// an empty string when no providers are requested.
pub fn working_dir_context(dir: &Path, providers: &[String]) -> String {
    if providers.is_empty() {
        return String::new();
    }
    let mut out = String::from("### Working directory context\n");
    out.push_str(&format!("- path: {}\n", dir.display()));
    for provider in providers {
        match provider.as_str() {
            "git" => out.push_str(&git_block(dir)),
            other => out.push_str(&format!("- unknown context provider: {other}\n")),
        }
    }
    out
}

/// Run a git command synchronously in `dir`. Returns the trimmed stdout on
/// success, or an error if the command fails or git is not available.
fn run_git_sync(dir: &Path, args: &[&str]) -> Result<String, String> {
    match std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
    {
        Ok(output) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        Ok(output) => Err(String::from_utf8_lossy(&output.stderr).to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Render the git context: branch, dirty flag, porcelain status, recent log.
fn git_block(dir: &Path) -> String {
    let mut b = String::new();

    // Detect repo membership independently of HEAD so a freshly-`init`ed repo
    // with an unborn branch (no commits yet) is still recognised and reported
    // as dirty when it has uncommitted/untracked files.
    if run_git_sync(dir, &["rev-parse", "--is-inside-work-tree"]).is_err() {
        b.push_str("- git: not a git repository\n");
        return b;
    }

    // `--abbrev-ref HEAD` fails on an unborn branch; fall back gracefully.
    let branch = match run_git_sync(dir, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(b) if b.trim() != "HEAD" && !b.trim().is_empty() => b.trim().to_string(),
        _ => "(unborn)".to_string(),
    };
    b.push_str(&format!("- git branch: {branch}\n"));

    let status = run_git_sync(dir, &["status", "--porcelain"]).unwrap_or_default();
    let dirty = !status.trim().is_empty();
    b.push_str(&format!("- git dirty: {dirty}\n"));
    if dirty {
        b.push_str("- git status:\n");
        for line in status.lines().take(10) {
            b.push_str(&format!("    {line}\n"));
        }
    }

    if let Ok(log) = run_git_sync(dir, &["log", "--oneline", "-5"]) {
        if !log.trim().is_empty() {
            b.push_str("- recent commits:\n");
            for line in log.lines().take(5) {
                b.push_str(&format!("    {line}\n"));
            }
        }
    }
    b
}

#[cfg(test)]
#[path = "workdir_tests.rs"]
mod tests;
