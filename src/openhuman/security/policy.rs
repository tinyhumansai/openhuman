use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::openhuman::util::floor_char_boundary;

/// How much autonomy the agent has
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AutonomyLevel {
    /// Read-only: can observe but not act
    ReadOnly,
    /// Supervised: acts but requires approval for risky operations
    #[default]
    Supervised,
    /// Full: autonomous execution within policy bounds
    Full,
}

/// Risk score for shell command execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRiskLevel {
    Low,
    Medium,
    High,
}

/// Classifies whether a tool operation is read-only or side-effecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOperation {
    Read,
    Act,
}

/// Sliding-window action tracker for rate limiting.
#[derive(Debug)]
pub struct ActionTracker {
    /// Timestamps of recent actions (kept within the last hour).
    actions: Mutex<Vec<Instant>>,
}

impl Default for ActionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionTracker {
    pub fn new() -> Self {
        Self {
            actions: Mutex::new(Vec::new()),
        }
    }

    /// Record an action and return the current count within the window.
    pub fn record(&self) -> usize {
        let mut actions = self.actions.lock();
        let cutoff = Instant::now()
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        actions.retain(|t| *t > cutoff);
        actions.push(Instant::now());
        actions.len()
    }

    /// Count of actions in the current window without recording.
    pub fn count(&self) -> usize {
        let mut actions = self.actions.lock();
        let cutoff = Instant::now()
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        actions.retain(|t| *t > cutoff);
        actions.len()
    }
}

impl Clone for ActionTracker {
    fn clone(&self) -> Self {
        let actions = self.actions.lock();
        Self {
            actions: Mutex::new(actions.clone()),
        }
    }
}

/// Security policy enforced on all tool executions
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    pub autonomy: AutonomyLevel,
    pub workspace_dir: PathBuf,
    pub workspace_only: bool,
    pub allowed_commands: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub max_actions_per_hour: u32,
    pub max_cost_per_day_cents: u32,
    pub require_approval_for_medium_risk: bool,
    pub block_high_risk_commands: bool,
    pub tracker: ActionTracker,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: PathBuf::from("."),
            workspace_only: true,
            allowed_commands: vec![
                "git".into(),
                "npm".into(),
                "cargo".into(),
                "ls".into(),
                "cat".into(),
                "grep".into(),
                "find".into(),
                "echo".into(),
                "pwd".into(),
                "wc".into(),
                "head".into(),
                "tail".into(),
                "date".into(),
                // Windows read-only equivalents for the same basic
                // inspection workflows as ls/cat/grep/which.
                "dir".into(),
                "type".into(),
                "where".into(),
                "findstr".into(),
                "more".into(),
            ],
            forbidden_paths: vec![
                // System directories (blocked even when workspace_only=false)
                "/etc".into(),
                "/root".into(),
                "/home".into(),
                "/usr".into(),
                "/bin".into(),
                "/sbin".into(),
                "/lib".into(),
                "/opt".into(),
                "/boot".into(),
                "/dev".into(),
                "/proc".into(),
                "/sys".into(),
                "/var".into(),
                "/tmp".into(),
                // Sensitive dotfiles
                "~/.ssh".into(),
                "~/.gnupg".into(),
                "~/.aws".into(),
                "~/.config".into(),
            ],
            max_actions_per_hour: 20,
            max_cost_per_day_cents: 500,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            tracker: ActionTracker::new(),
        }
    }
}

/// Skip leading environment variable assignments (e.g. `FOO=bar cmd args`).
/// Returns the remainder starting at the first non-assignment word.
fn skip_env_assignments(s: &str) -> &str {
    let mut rest = s;
    loop {
        let Some(word) = rest.split_whitespace().next() else {
            return rest;
        };
        // Environment assignment: contains '=' and starts with a letter or underscore
        if word.contains('=')
            && word
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            // Advance past this word
            rest = rest[word.len()..].trim_start();
        } else {
            return rest;
        }
    }
}

fn command_basename(command: &str) -> &str {
    command
        .split(|ch| ch == '/' || ch == '\\')
        .next_back()
        .unwrap_or(command)
}

fn normalized_command_name(command: &str) -> String {
    let command = command_basename(command).to_ascii_lowercase();
    command
        .strip_suffix(".exe")
        .unwrap_or(command.as_str())
        .to_string()
}

fn is_python_command(command: &str) -> bool {
    let command = normalized_command_name(command);
    command == "python"
        || command == "pythonw"
        || command
            .strip_prefix("pythonw")
            .and_then(|suffix| suffix.chars().next())
            .is_some_and(|ch| ch.is_ascii_digit())
        || command
            .strip_prefix("python")
            .and_then(|suffix| suffix.chars().next())
            .is_some_and(|ch| ch.is_ascii_digit())
}

fn is_command_executor(command: &str) -> bool {
    let command = normalized_command_name(command);
    is_python_command(command.as_str())
        || matches!(
            command.as_str(),
            "xargs"
                | "awk"
                | "gawk"
                | "mawk"
                | "nawk"
                | "perl"
                | "ruby"
                | "bash"
                | "sh"
                | "dash"
                | "zsh"
                | "ksh"
                | "fish"
                | "env"
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteState {
    None,
    Single,
    Double,
}

/// Split a shell command into sub-commands by unquoted separators.
///
/// Separators:
/// - `;` and newline
/// - `|`
/// - `&&`, `||`
///
/// Characters inside single or double quotes are treated as literals, so
/// `sqlite3 db "SELECT 1; SELECT 2;"` remains a single segment.
fn split_unquoted_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote = QuoteState::None;
    let mut escaped = false;
    let mut chars = command.chars().peekable();

    let push_segment = |segments: &mut Vec<String>, current: &mut String| {
        let trimmed = current.trim();
        if !trimmed.is_empty() {
            segments.push(trimmed.to_string());
        }
        current.clear();
    };

    while let Some(ch) = chars.next() {
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
                current.push(ch);
            }
            QuoteState::Double => {
                if escaped {
                    escaped = false;
                    current.push(ch);
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    current.push(ch);
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::None;
                }
                current.push(ch);
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    current.push(ch);
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    current.push(ch);
                    continue;
                }

                match ch {
                    '\'' => {
                        quote = QuoteState::Single;
                        current.push(ch);
                    }
                    '"' => {
                        quote = QuoteState::Double;
                        current.push(ch);
                    }
                    ';' | '\n' => push_segment(&mut segments, &mut current),
                    '|' => {
                        if chars.next_if_eq(&'|').is_some() {
                            // Consume full `||`; both characters are separators.
                        }
                        push_segment(&mut segments, &mut current);
                    }
                    '&' => {
                        if chars.next_if_eq(&'&').is_some() {
                            // `&&` is a separator; single `&` is handled separately.
                            push_segment(&mut segments, &mut current);
                        } else {
                            current.push(ch);
                        }
                    }
                    _ => current.push(ch),
                }
            }
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }

    segments
}

/// Detect a single unquoted `&` operator (background/chain). `&&` is allowed.
///
/// We treat any standalone `&` as unsafe in policy validation because it can
/// chain hidden sub-commands and escape foreground timeout expectations.
fn contains_unquoted_single_ampersand(command: &str) -> bool {
    let mut quote = QuoteState::None;
    let mut escaped = false;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
            }
            QuoteState::Double => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::None;
                }
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                match ch {
                    '\'' => quote = QuoteState::Single,
                    '"' => quote = QuoteState::Double,
                    '&' => {
                        if chars.next_if_eq(&'&').is_none() {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    false
}

/// Detect an unquoted character in a shell command.
fn contains_unquoted_char(command: &str, target: char) -> bool {
    let mut quote = QuoteState::None;
    let mut escaped = false;

    for ch in command.chars() {
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
            }
            QuoteState::Double => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::None;
                    continue;
                }
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                match ch {
                    '\'' => quote = QuoteState::Single,
                    '"' => quote = QuoteState::Double,
                    _ if ch == target => return true,
                    _ => {}
                }
            }
        }
    }

    false
}

impl SecurityPolicy {
    /// Classify command risk. Any high-risk segment marks the whole command high.
    pub fn command_risk_level(&self, command: &str) -> CommandRiskLevel {
        let mut saw_medium = false;

        for segment in split_unquoted_segments(command) {
            let cmd_part = skip_env_assignments(&segment);
            let mut words = cmd_part.split_whitespace();
            let Some(base_raw) = words.next() else {
                continue;
            };

            let base = normalized_command_name(base_raw);

            let args: Vec<String> = words.map(|w| w.to_ascii_lowercase()).collect();
            let joined_segment = cmd_part.to_ascii_lowercase();

            // High-risk commands
            if is_command_executor(base.as_str())
                || matches!(
                    base.as_str(),
                    "rm" | "mkfs"
                        | "dd"
                        | "shutdown"
                        | "reboot"
                        | "halt"
                        | "poweroff"
                        | "sudo"
                        | "su"
                        | "chown"
                        | "chmod"
                        | "useradd"
                        | "userdel"
                        | "usermod"
                        | "passwd"
                        | "mount"
                        | "umount"
                        | "iptables"
                        | "ufw"
                        | "firewall-cmd"
                        | "curl"
                        | "wget"
                        | "nc"
                        | "ncat"
                        | "netcat"
                        | "scp"
                        | "ssh"
                        | "ftp"
                        | "telnet"
                )
            {
                return CommandRiskLevel::High;
            }

            if joined_segment.contains("rm -rf /")
                || joined_segment.contains("rm -fr /")
                || joined_segment.contains(":(){:|:&};:")
            {
                return CommandRiskLevel::High;
            }

            // Medium-risk commands (state-changing, but not inherently destructive)
            let medium = match base.as_str() {
                "git" => args.first().is_some_and(|verb| {
                    matches!(
                        verb.as_str(),
                        "commit"
                            | "push"
                            | "reset"
                            | "clean"
                            | "rebase"
                            | "merge"
                            | "cherry-pick"
                            | "revert"
                            | "branch"
                            | "checkout"
                            | "switch"
                            | "tag"
                    )
                }),
                "npm" | "pnpm" | "yarn" => args.first().is_some_and(|verb| {
                    matches!(
                        verb.as_str(),
                        "install" | "add" | "remove" | "uninstall" | "update" | "publish"
                    )
                }),
                "cargo" => args.first().is_some_and(|verb| {
                    matches!(
                        verb.as_str(),
                        "add" | "remove" | "install" | "clean" | "publish"
                    )
                }),
                "touch" | "mkdir" | "mv" | "cp" | "ln" => true,
                _ => false,
            };

            saw_medium |= medium;
        }

        if saw_medium {
            CommandRiskLevel::Medium
        } else {
            CommandRiskLevel::Low
        }
    }

    /// Validate full command execution policy (allowlist + risk gate).
    pub fn validate_command_execution(
        &self,
        command: &str,
        approved: bool,
    ) -> Result<CommandRiskLevel, String> {
        if !self.is_command_allowed(command) {
            // Truncate the command in BOTH the log and the Err return: the Err
            // string is bubbled back to the frontend, and a full untruncated
            // command can leak secrets in args (e.g. `curl -H "Authorization:
            // Bearer …"`, `psql "postgres://user:pass@…"`). The 80-char cap
            // matches the log truncation so a long base command with safe args
            // still shows enough context to diagnose the block.
            let truncated = &command[..floor_char_boundary(command, 80)];
            log::warn!(
                "[openhuman:policy] Command blocked by allowlist: {}",
                truncated
            );
            return Err(format!(
                "Command not allowed by security policy: {truncated}"
            ));
        }

        let risk = self.command_risk_level(command);

        if risk == CommandRiskLevel::High {
            if self.block_high_risk_commands {
                log::warn!(
                    "[openhuman:policy] High-risk command blocked: {}",
                    &command[..floor_char_boundary(command, 80)]
                );
                return Err("Command blocked: high-risk command is disallowed by policy".into());
            }
            if self.autonomy == AutonomyLevel::Supervised && !approved {
                log::warn!(
                    "[openhuman:policy] High-risk command needs approval: {}",
                    &command[..floor_char_boundary(command, 80)]
                );
                return Err(
                    "Command requires explicit approval (approved=true): high-risk operation"
                        .into(),
                );
            }
        }

        if risk == CommandRiskLevel::Medium
            && self.autonomy == AutonomyLevel::Supervised
            && self.require_approval_for_medium_risk
            && !approved
        {
            log::info!(
                "[openhuman:policy] Medium-risk command needs approval: {}",
                &command[..floor_char_boundary(command, 80)]
            );
            return Err(
                "Command requires explicit approval (approved=true): medium-risk operation".into(),
            );
        }

        log::debug!(
            "[openhuman:policy] Command validated: risk={:?}, approved={}, cmd={}",
            risk,
            approved,
            &command[..floor_char_boundary(command, 80)]
        );
        Ok(risk)
    }

    /// Check if a shell command is allowed.
    ///
    /// Validates the **entire** command string, not just the first word:
    /// - Blocks subshell operators (`` ` ``, `$(`) that hide arbitrary execution
    /// - Splits on command separators (`|`, `&&`, `||`, `;`, newlines) and
    ///   validates each sub-command against the allowlist
    /// - Blocks single `&` background chaining (`&&` remains supported)
    /// - Blocks output redirections (`>`, `>>`) that could write outside workspace
    /// - Blocks dangerous arguments (e.g. `find -exec`, `git config`)
    pub fn is_command_allowed(&self, command: &str) -> bool {
        if self.autonomy == AutonomyLevel::ReadOnly {
            return false;
        }

        // Block subshell/expansion operators — these allow hiding arbitrary
        // commands inside an allowed command (e.g. `echo $(rm -rf /)`)
        if command.contains('`')
            || command.contains("$(")
            || command.contains("${")
            || command.contains("<(")
            || command.contains(">(")
        {
            return false;
        }

        // Block output redirections (`>`, `>>`) — they can write to arbitrary paths.
        // Ignore quoted literals, e.g. `echo "a>b"`.
        if contains_unquoted_char(command, '>') {
            return false;
        }

        // Block `tee` — it can write to arbitrary files, bypassing the
        // redirect check above (e.g. `echo secret | tee /etc/crontab`)
        if command
            .split_whitespace()
            .any(|w| w == "tee" || w.ends_with("/tee"))
        {
            return false;
        }

        // Block background command chaining (`&`), which can hide extra
        // sub-commands and outlive timeout expectations. Keep `&&` allowed.
        if contains_unquoted_single_ampersand(command) {
            return false;
        }

        // Split on unquoted command separators and validate each sub-command.
        let segments = split_unquoted_segments(command);
        for segment in &segments {
            // Strip leading env var assignments (e.g. FOO=bar cmd)
            let cmd_part = skip_env_assignments(segment);

            let mut words = cmd_part.split_whitespace();
            let base_raw = words.next().unwrap_or("");
            let base_cmd = command_basename(base_raw);

            if base_cmd.is_empty() {
                continue;
            }

            if !self
                .allowed_commands
                .iter()
                .any(|allowed| allowed == base_cmd)
            {
                return false;
            }

            // Validate arguments for the command
            let args: Vec<String> = words.map(|w| w.to_ascii_lowercase()).collect();
            if !self.is_args_safe(base_cmd, &args) {
                return false;
            }
        }

        // At least one command must be present
        let has_cmd = segments.iter().any(|s| {
            let s = skip_env_assignments(s.trim());
            s.split_whitespace().next().is_some_and(|w| !w.is_empty())
        });

        has_cmd
    }

    /// Check for dangerous arguments that allow sub-command execution.
    fn is_args_safe(&self, base: &str, args: &[String]) -> bool {
        let base = base.to_ascii_lowercase();
        if is_command_executor(base.as_str()) {
            return false;
        }

        match base.as_str() {
            "find" => {
                // find -exec and find -ok allow arbitrary command execution
                !args.iter().any(|arg| arg == "-exec" || arg == "-ok")
            }
            "git" => {
                // git config, alias, and -c can be used to set dangerous options
                // (e.g. git config core.editor "rm -rf /")
                !args.iter().any(|arg| {
                    arg == "config"
                        || arg.starts_with("config.")
                        || arg == "alias"
                        || arg.starts_with("alias.")
                        || arg == "-c"
                })
            }
            _ => true,
        }
    }

    fn expand_tilde(&self, path: &str) -> String {
        if let Some(rest) = path.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return format!("{}/{rest}", home.display());
            }
        }
        path.to_string()
    }

    /// String-only path check. Does NOT resolve symlinks.
    /// Use `validate_path()` for any path that will be used for file I/O.
    pub fn is_path_string_allowed(&self, path: &str) -> bool {
        // Block null bytes (can truncate paths in C-backed syscalls)
        if path.contains('\0') {
            return false;
        }

        // Block path traversal: check for ".." as a path component
        if Path::new(path)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return false;
        }

        // Block URL-encoded traversal attempts (e.g. ..%2f)
        let lower = path.to_lowercase();
        if lower.contains("..%2f") || lower.contains("%2f..") {
            return false;
        }

        // Expand tilde for comparison
        let expanded = self.expand_tilde(path);

        // Block absolute paths when workspace_only is set
        if self.workspace_only && Path::new(&expanded).is_absolute() {
            return false;
        }

        // Block forbidden paths using path-component-aware matching
        let expanded_path = Path::new(&expanded);
        for forbidden in &self.forbidden_paths {
            let forbidden_expanded = self.expand_tilde(forbidden);
            let forbidden_path = Path::new(&forbidden_expanded);
            if expanded_path.starts_with(forbidden_path) {
                return false;
            }
        }

        // Symlink-safe check (#1927). The string-level checks above can be
        // bypassed by creating a symlink inside the workspace that points to
        // a forbidden tree (e.g. `evil -> /etc/shadow`). Canonicalize the
        // path and re-validate `workspace_only` containment + forbidden_paths
        // against the resolved location.
        if let Some(canonical) = self.try_canonicalize_under_workspace(path) {
            let workspace_root = self
                .workspace_dir
                .canonicalize()
                .unwrap_or_else(|_| self.workspace_dir.clone());
            if self.workspace_only && !canonical.starts_with(&workspace_root) {
                log::trace!(
                    "[security:policy] path blocked: symlink escapes workspace (requested={}, resolved={}, workspace={})",
                    path,
                    canonical.display(),
                    workspace_root.display()
                );
                return false;
            }
            // If the resolved path stays inside the workspace, trust the
            // workspace boundary over forbidden_paths — otherwise a workspace
            // that lives under e.g. `/tmp` (common in tests and sandboxes)
            // would block every legitimate access. forbidden_paths is meant
            // to catch escapes *outside* the workspace, which the workspace
            // containment check above already validates.
            let inside_workspace = canonical.starts_with(&workspace_root);
            if !inside_workspace {
                for forbidden in &self.forbidden_paths {
                    let forbidden_expanded = if let Some(stripped) = forbidden.strip_prefix("~/") {
                        std::env::var("HOME")
                            .ok()
                            .map(|h| PathBuf::from(h).join(stripped))
                            .unwrap_or_else(|| PathBuf::from(forbidden))
                    } else {
                        PathBuf::from(forbidden)
                    };
                    let forbidden_canonical = forbidden_expanded
                        .canonicalize()
                        .unwrap_or(forbidden_expanded);
                    if canonical.starts_with(&forbidden_canonical) {
                        log::trace!(
                        "[security:policy] path blocked: symlink resolves to forbidden tree (requested={}, resolved={}, forbidden={})",
                        path,
                        canonical.display(),
                        forbidden_canonical.display()
                    );
                        return false;
                    }
                }
            }
        }

        true
    }

    /// Resolve a user-supplied path under the workspace, canonicalizing it
    /// (or its parent) when present on disk. Used by [`Self::is_path_string_allowed`]
    /// to defend against symlink-based escapes that pass the string-level
    /// checks. Returns `None` only when neither the path nor its parent can
    /// be resolved on disk — in that case the caller falls back to the
    /// string-level checks alone (which is the safe default for fresh paths
    /// whose entire chain does not yet exist).
    fn try_canonicalize_under_workspace(&self, path: &str) -> Option<PathBuf> {
        let expanded = if let Some(stripped) = path.strip_prefix("~/") {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(stripped))?
        } else {
            PathBuf::from(path)
        };
        let absolute = if expanded.is_absolute() {
            expanded
        } else {
            self.workspace_dir.join(&expanded)
        };
        if let Ok(canonical) = absolute.canonicalize() {
            return Some(canonical);
        }
        // Path itself does not exist (e.g. a write-to-new-file call). Try
        // canonicalizing the parent + appending the basename so we still
        // catch parent chains that resolve via symlink to a forbidden tree.
        let parent = absolute.parent()?;
        let name = absolute.file_name()?;
        parent.canonicalize().ok().map(|p| p.join(name))
    }

    /// Validate a path for file I/O: string checks, canonicalize, workspace containment,
    /// and forbidden-path check on the resolved path.
    /// Returns the canonical `PathBuf` on success.
    pub async fn validate_path(&self, path: &str) -> Result<PathBuf, String> {
        if !self.is_path_string_allowed(path) {
            return Err(format!("Path not allowed by security policy: {path}"));
        }
        let expanded = self.expand_tilde(path);
        let full_path = if Path::new(&expanded).is_absolute() {
            PathBuf::from(&expanded)
        } else {
            self.workspace_dir.join(&expanded)
        };
        let resolved = tokio::fs::canonicalize(&full_path)
            .await
            .map_err(|e| format!("Failed to resolve path '{path}': {e}"))?;
        if !self.is_resolved_path_allowed(&resolved) {
            return Err(format!(
                "Resolved path escapes workspace: {}",
                resolved.display()
            ));
        }
        let workspace_root = tokio::fs::canonicalize(&self.workspace_dir)
            .await
            .unwrap_or_else(|_| self.workspace_dir.clone());
        self.check_resolved_against_forbidden(&resolved, &workspace_root)?;
        log::debug!(
            "[security] validate_path: '{}' resolved to '{}'",
            path,
            resolved.display()
        );
        Ok(resolved)
    }

    /// Like `validate_path` but canonicalizes the parent directory.
    /// Use for write operations where the target file may not yet exist.
    /// Does NOT require the parent directory to exist — walks up to the deepest
    /// existing ancestor and checks that for symlink escapes.
    /// Returns the canonical full path (parent resolved + filename appended).
    pub async fn validate_parent_path(&self, path: &str) -> Result<PathBuf, String> {
        if !self.is_path_string_allowed(path) {
            return Err(format!("Path not allowed by security policy: {path}"));
        }
        let expanded = self.expand_tilde(path);
        let full_path = if Path::new(&expanded).is_absolute() {
            PathBuf::from(&expanded)
        } else {
            self.workspace_dir.join(&expanded)
        };
        let parent = full_path
            .parent()
            .ok_or_else(|| format!("Invalid path (no parent): {path}"))?;
        let file_name = full_path
            .file_name()
            .ok_or_else(|| format!("Invalid path (no filename): {path}"))?;

        // Walk up to the deepest existing ancestor so we can canonicalize without
        // requiring the full parent path to exist yet. This catches symlink escapes
        // in existing path components even when deeper dirs are not created yet.
        let mut existing_ancestor = parent.to_path_buf();
        loop {
            if existing_ancestor.exists() {
                break;
            }
            match existing_ancestor.parent() {
                Some(p) => existing_ancestor = p.to_path_buf(),
                None => break,
            }
        }
        let canonical_ancestor = tokio::fs::canonicalize(&existing_ancestor)
            .await
            .map_err(|e| format!("Failed to resolve parent of '{path}': {e}"))?;
        if !self.is_resolved_path_allowed(&canonical_ancestor) {
            return Err(format!(
                "Resolved parent path escapes workspace: {}",
                canonical_ancestor.display()
            ));
        }

        // Build resolved result: canonical_ancestor + suffix from existing_ancestor to parent + filename.
        // Since is_path_string_allowed blocked "..", all components between the ancestor
        // and the intended parent are newly created dirs — no symlinks possible there.
        let relative_suffix = parent
            .strip_prefix(&existing_ancestor)
            .unwrap_or(std::path::Path::new(""));
        let resolved_parent = canonical_ancestor.join(relative_suffix);
        let result = resolved_parent.join(file_name);

        let workspace_root = tokio::fs::canonicalize(&self.workspace_dir)
            .await
            .unwrap_or_else(|_| self.workspace_dir.clone());
        self.check_resolved_against_forbidden(&canonical_ancestor, &workspace_root)?;
        self.check_resolved_against_forbidden(&result, &workspace_root)?;

        log::debug!(
            "[security] validate_parent_path: '{}' resolved parent to '{}'",
            path,
            resolved_parent.display()
        );
        Ok(result)
    }

    /// Validate that a resolved path is still inside the workspace.
    /// Call this AFTER joining `workspace_dir` + relative path and canonicalizing.
    pub fn is_resolved_path_allowed(&self, resolved: &Path) -> bool {
        // Must be under workspace_dir (prevents symlink escapes).
        // Prefer canonical workspace root so `/a/../b` style config paths don't
        // cause false positives or negatives.
        let workspace_root = self
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_dir.clone());
        resolved.starts_with(workspace_root)
    }

    /// Check `resolved` against every entry in `forbidden_paths`, resolving relative
    /// entries against `workspace_root`. Absolute entries whose prefix IS the workspace
    /// root are skipped — the workspace containment check already covers them.
    fn check_resolved_against_forbidden(
        &self,
        resolved: &Path,
        workspace_root: &Path,
    ) -> Result<(), String> {
        for forbidden in &self.forbidden_paths {
            let forbidden_path = PathBuf::from(self.expand_tilde(forbidden));
            let forbidden_resolved = if forbidden_path.is_absolute() {
                if workspace_root.starts_with(&forbidden_path) {
                    continue;
                }
                forbidden_path
            } else {
                workspace_root.join(forbidden_path)
            };
            if resolved.starts_with(&forbidden_resolved) {
                return Err(format!(
                    "Resolved path is inside a forbidden directory: {}",
                    forbidden_resolved.display()
                ));
            }
        }
        Ok(())
    }

    /// Check if autonomy level permits any action at all
    pub fn can_act(&self) -> bool {
        self.autonomy != AutonomyLevel::ReadOnly
    }

    /// Enforce policy for a tool operation.
    ///
    /// Read operations are always allowed by autonomy/rate gates.
    /// Act operations require non-readonly autonomy and available action budget.
    pub fn enforce_tool_operation(
        &self,
        operation: ToolOperation,
        operation_name: &str,
    ) -> Result<(), String> {
        match operation {
            ToolOperation::Read => Ok(()),
            ToolOperation::Act => {
                if !self.can_act() {
                    log::warn!(
                        "[openhuman:policy] Operation '{}' blocked: read-only mode",
                        operation_name
                    );
                    return Err(format!(
                        "Security policy: read-only mode, cannot perform '{operation_name}'"
                    ));
                }

                if !self.record_action() {
                    log::warn!(
                        "[openhuman:policy] Operation '{}' blocked: rate limit exceeded",
                        operation_name
                    );
                    return Err("Rate limit exceeded: action budget exhausted".to_string());
                }

                log::debug!(
                    "[openhuman:policy] Operation '{}' allowed (actions: {}/{})",
                    operation_name,
                    self.tracker.count(),
                    self.max_actions_per_hour
                );
                Ok(())
            }
        }
    }

    /// Record an action and check if the rate limit has been exceeded.
    /// Returns `true` if the action is allowed, `false` if rate-limited.
    pub fn record_action(&self) -> bool {
        let count = self.tracker.record();
        count <= self.max_actions_per_hour as usize
    }

    /// Check if the rate limit would be exceeded without recording.
    pub fn is_rate_limited(&self) -> bool {
        self.tracker.count() >= self.max_actions_per_hour as usize
    }

    /// Build from config sections
    pub fn from_config(
        autonomy_config: &crate::openhuman::config::AutonomyConfig,
        workspace_dir: &Path,
    ) -> Self {
        log::info!(
            "[openhuman:policy] SecurityPolicy created: autonomy={:?}, workspace_only={}, allowed_cmds={}, max_actions/hr={}",
            autonomy_config.level,
            autonomy_config.workspace_only,
            autonomy_config.allowed_commands.len(),
            autonomy_config.max_actions_per_hour
        );
        Self {
            autonomy: autonomy_config.level,
            workspace_dir: workspace_dir.to_path_buf(),
            workspace_only: autonomy_config.workspace_only,
            allowed_commands: autonomy_config.allowed_commands.clone(),
            forbidden_paths: autonomy_config.forbidden_paths.clone(),
            max_actions_per_hour: autonomy_config.max_actions_per_hour,
            max_cost_per_day_cents: autonomy_config.max_cost_per_day_cents,
            require_approval_for_medium_risk: autonomy_config.require_approval_for_medium_risk,
            block_high_risk_commands: autonomy_config.block_high_risk_commands,
            tracker: ActionTracker::new(),
        }
    }
}

/// Validate that a file path resolves within a given root directory.
/// Canonicalizes both paths and checks that the resolved candidate
/// starts with the root. Callers should check `.is_file()` first
/// to avoid errors on non-existent paths (normal missing-file case).
///
/// Used to prevent path traversal in agent definition TOML files and
/// other user-controllable file references.
pub fn validate_path_within_root(
    candidate: &std::path::Path,
    root: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let resolved_root = root
        .canonicalize()
        .map_err(|e| format!("workspace root: {e}"))?;
    let resolved = candidate
        .canonicalize()
        .map_err(|e| format!("{}: {e}", candidate.display()))?;
    if !resolved.starts_with(&resolved_root) {
        return Err(format!(
            "path escapes root: {} is not under {}",
            resolved.display(),
            resolved_root.display()
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
