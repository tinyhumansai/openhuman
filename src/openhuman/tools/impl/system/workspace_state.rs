//! Tool: read_workspace_state — read-only workspace overview for Orchestrator/Planner.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

/// Returns a summary of the workspace: git status, file tree, recent commits.
pub struct WorkspaceStateTool {
    workspace_dir: PathBuf,
}

impl WorkspaceStateTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for WorkspaceStateTool {
    fn name(&self) -> &str {
        "read_workspace_state"
    }

    fn description(&self) -> &str {
        "Get a read-only overview of the workspace: git status (modified/untracked files), \
         recent commits, and top-level directory structure. Useful for understanding the \
         current project state before planning tasks."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "include_tree": {
                    "type": "boolean",
                    "description": "Include top-level directory tree (default: true).",
                    "default": true
                },
                "recent_commits": {
                    "type": "integer",
                    "description": "Number of recent commits to show (default: 5).",
                    "default": 5
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let include_tree = args
            .get("include_tree")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let recent_commits = args
            .get("recent_commits")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        tracing::debug!(
            "[workspace_state] dir={}, include_tree={include_tree}, recent_commits={recent_commits}",
            self.workspace_dir.display()
        );

        let mut output = String::new();
        let dir = &self.workspace_dir;

        // Git status
        output.push_str("## Git Status\n");
        match run_git(dir, &["status", "--porcelain"]).await {
            Ok(status) if status.trim().is_empty() => {
                output.push_str("Clean working tree.\n");
            }
            Ok(status) => {
                output.push_str(&status);
            }
            Err(e) => {
                output.push_str(&format!("(not a git repo or error: {e})\n"));
            }
        }

        // Recent commits
        output.push_str(&format!("\n## Recent Commits (last {recent_commits})\n"));
        let log_arg = format!("-{recent_commits}");
        match run_git(dir, &["log", &log_arg, "--oneline", "--no-decorate"]).await {
            Ok(log) => output.push_str(&log),
            Err(e) => output.push_str(&format!("(error: {e})\n")),
        }

        // Directory tree (top-level only)
        if include_tree {
            output.push_str("\n## Directory Tree (top-level)\n");
            match tokio::fs::read_dir(dir).await {
                Ok(mut entries) => {
                    let mut names = Vec::new();
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !name.starts_with('.') {
                            let suffix = if entry
                                .file_type()
                                .await
                                .map(|ft| ft.is_dir())
                                .unwrap_or(false)
                            {
                                "/"
                            } else {
                                ""
                            };
                            names.push(format!("{name}{suffix}"));
                        }
                    }
                    names.sort();
                    for name in &names {
                        output.push_str(&format!("  {name}\n"));
                    }
                }
                Err(e) => output.push_str(&format!("(error reading dir: {e})\n")),
            }
        }

        tracing::debug!("[workspace_state] output length={}", output.len());
        Ok(ToolResult::success(output))
    }
}

/// Repository config keys this tool will run `git` under.
///
/// **This is an allowlist, and the direction is the whole point.** Several git
/// config keys name a command that git then executes — `core.fsmonitor` is run
/// by `git status`, `core.sshCommand` by anything that reaches a remote,
/// `diff.external` by a diff, `core.pager` and `core.editor` by the commands
/// that use them, and the `filter.*.process` / `*.clean` / `*.smudge` and
/// `diff.*.textconv` families by content operations. Enumerating *those* and
/// clearing them is the obvious fix and it is the wrong shape: the list is only
/// correct until git adds a key, and a denylist that has gone stale reads as
/// protection while providing none.
///
/// An allowlist ages the other way. A key nobody here has heard of is refused,
/// so a new git release makes this tool fail closed and loud rather than
/// silently regain the hole.
///
/// The entries are `section.key`, lowercased, with any subsection elided —
/// `remote.origin.url` is checked as `remote.url`. That is what git's own
/// `--list` output normalises to for our purposes, and it means a repository
/// with fifty remotes needs no extra entries.
const ALLOWED_REPO_CONFIG: &[&str] = &[
    // What `git init` and `git clone` write, and nothing else.
    "core.repositoryformatversion",
    "core.filemode",
    "core.bare",
    "core.logallrefupdates",
    "core.ignorecase",
    "core.precomposeunicode",
    "core.symlinks",
    "core.worktree",
    "remote.url",
    "remote.fetch",
    "remote.pushurl",
    "remote.mirror",
    "branch.remote",
    "branch.merge",
    "branch.rebase",
    "submodule.active",
    "submodule.url",
    "user.name",
    "user.email",
    "pull.rebase",
    "push.default",
    "init.defaultbranch",
    // Inert settings ordinary repositories carry that the first draft refused,
    // making the tool useless on a large class of real workspaces. Each is a
    // value git *interprets*; none names a program git runs.
    "core.autocrlf",
    "core.eol",
    "core.untrackedcache",
    "core.longpaths",
    "core.fscache",
    "core.hidedotfiles",
    "core.sparsecheckout",
    "core.sparsecheckoutcone",
    "commit.gpgsign",
    "tag.gpgsign",
    "remote.tagopt",
    "remote.prune",
    "remote.partialclonefilter",
    "remote.promisor",
    "branch.vscodemerge",
    "gc.auto",
    "fetch.prune",
    // SHA-256 repositories and worktree-scoped config. Only these two
    // `extensions.*` keys — the namespace as a whole is where git puts
    // repository-format switches, and a blanket allow would admit whatever it
    // adds next.
    "extensions.objectformat",
    "extensions.worktreeconfig",
    // `filter.<driver>.required` is a boolean. The driver's actual programs —
    // `clean`, `smudge`, `process` — are NOT here and must not be; see the LFS
    // note on `NEUTRALISED_CONFIG`.
    "filter.required",
    "lfs.repositoryformatversion",
];

/// Command-valued keys cleared on the command line as a second layer.
///
/// Command-line `-c` outranks every config file, so this genuinely neutralises
/// these keys even when a repository sets them. It is a denylist and therefore
/// cannot be the guarantee — [`ALLOWED_REPO_CONFIG`] is — but it costs nothing
/// and it means a key that slips past the allowlist check still does not run.
/// `core.hooksPath` is deliberately absent: there is no portable value that
/// means "nowhere", and `git status` / `git log` run no hooks anyway. An
/// unrecognised `core.hookspath` is refused by [`ALLOWED_REPO_CONFIG`], which
/// is the layer that is supposed to catch it.
///
/// ## Two keys that look inert and are not
///
/// **`credential.helper` is command-valued.** A value beginning `!` is run as a
/// shell command, so it belongs nowhere near [`ALLOWED_REPO_CONFIG`] despite
/// reading like a mere preference. It is refused, and a repository that sets it
/// gets the refusal.
///
/// **A `git lfs install` clone is refused, deliberately.** `filter.lfs.clean`,
/// `.smudge` and `.process` each name a program, so an LFS working copy cannot
/// be read by this tool. That is the fail-closed answer and it is the intended
/// one — the alternative is running whatever a `.git/config` in an
/// agent-writable directory nominates as a filter driver. Only
/// `filter.<driver>.required`, a boolean, is allowed.
const NEUTRALISED_CONFIG: &[&str] = &[
    "core.fsmonitor=",
    "core.sshCommand=",
    "core.pager=cat",
    "core.editor=false",
    "diff.external=",
    "sequence.editor=false",
    "uploadpack.packObjectsHook=",
];

/// A path git will read as an empty config file.
///
/// `GIT_CONFIG_GLOBAL` must name something readable-and-empty rather than be
/// unset — unsetting it lets git fall back to `~/.gitconfig`, which is the
/// thing being suppressed. `/dev/null` is not a path on Windows; `NUL` is.
const NULL_CONFIG_PATH: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// Build a `git` invocation with the ambient environment taken away.
///
/// `GIT_CONFIG_NOSYSTEM` and `GIT_CONFIG_GLOBAL` close the system and global
/// config files. Note what they do *not* close: the repository's own
/// `.git/config`, which is the file an agent-writable workspace actually lets
/// an attacker author. That one is handled by [`repo_config_is_inert`]; these
/// two are here because they are free and they close two real vectors.
fn hardened_git(dir: &std::path::Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("git");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", NULL_CONFIG_PATH)
        // `git` consults these before it reads any config file.
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GIT_PAGER")
        .env_remove("GIT_EDITOR")
        .env_remove("GIT_SSH")
        .env_remove("GIT_SSH_COMMAND")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(dir);
    for kv in NEUTRALISED_CONFIG {
        cmd.arg("-c").arg(kv);
    }
    cmd
}

/// Normalise a `git config --list` key to the `section.key` form
/// [`ALLOWED_REPO_CONFIG`] uses, dropping any subsection.
///
/// `remote.origin.url` → `remote.url`; `core.filemode` → `core.filemode`. A
/// subsection may itself contain dots (`includeIf.gitdir:~/x.y/.path`), so the
/// first and last components are the reliable ones.
fn normalise_config_key(key: &str) -> String {
    let key = key.to_ascii_lowercase();
    match (key.find('.'), key.rfind('.')) {
        (Some(first), Some(last)) if first != last => {
            format!("{}.{}", &key[..first], &key[last + 1..])
        }
        _ => key,
    }
}

/// Is the repository at `dir` configured only in ways that cannot make `git`
/// run something?
///
/// Returns the offending key when it is not. Reading the config is itself done
/// through [`hardened_git`], and `git config --list` neither consults
/// `core.fsmonitor` nor spawns a pager when its output is captured, so this
/// inspection step does not have the property it is checking for.
async fn repo_config_is_inert(dir: &std::path::Path) -> anyhow::Result<Option<String>> {
    let output = hardened_git(dir)
        .args(["config", "--list", "--local", "--null"])
        .output()
        .await?;

    // A directory that is not a repository has no local config to distrust.
    // `git status` will report that in its own words a moment later.
    if !output.status.success() {
        return Ok(None);
    }

    let listing = String::from_utf8_lossy(&output.stdout);
    for entry in listing.split('\0').filter(|e| !e.is_empty()) {
        // `--null` separates entries with NUL and key from value with LF.
        let key = entry.split('\n').next().unwrap_or(entry);
        if !ALLOWED_REPO_CONFIG.contains(&normalise_config_key(key).as_str()) {
            return Ok(Some(key.to_string()));
        }
    }
    Ok(None)
}

async fn run_git(dir: &std::path::Path, args: &[&str]) -> anyhow::Result<String> {
    if let Some(key) = repo_config_is_inert(dir).await? {
        // The refusal is otherwise only visible folded into the tool's own
        // output, which is not greppable when an operator is asking why a
        // workspace stopped reporting. Correlation fields: the directory and
        // the key that caused it.
        tracing::debug!(
            "[workspace_state] refusing to run git: dir={}, disallowed_config_key={key}",
            dir.display()
        );
        anyhow::bail!(
            "refusing to run git in {}: its repository config sets `{key}`, which is \
             not on the allowlist of configuration this tool will run under. \
             Several git config keys name a command git then executes, and this \
             directory is agent-writable, so unrecognised configuration is treated \
             as untrusted rather than honoured.",
            dir.display()
        )
    }

    let output = hardened_git(dir).args(args).output().await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )
    }
}

#[cfg(test)]
#[path = "workspace_state_tests.rs"]
mod tests;
