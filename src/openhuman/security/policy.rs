use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::OnceCell;

use crate::openhuman::util::floor_char_boundary;

/// Stable, machine-recognizable marker prefixing a **permanent** policy
/// rejection: the identical `(tool, args)` call can never succeed in the
/// current tier (read-only blocking a write, a forbidden/credential path, a
/// disallowed high-risk or hidden-execution command, an off-allowlist command).
/// The agent harness ([`crate::openhuman::agent::harness::tool_loop`]) detects
/// this and halts on the **first verbatim repeat** rather than reiterating a
/// provably-futile call. Kept short and bracketed so it survives the
/// `Error: …` wrapping the tool layer adds and is easy to grep in logs.
pub const POLICY_BLOCKED_MARKER: &str = "[policy-blocked]";

/// Stable marker prefixing a **this-turn denial** — the user answered "no" to
/// an approval prompt, or the prompt timed out / its channel dropped. Unlike a
/// block this isn't permanent across turns, but re-issuing the *same* call this
/// turn just re-prompts the user, so the harness records it in the circuit
/// breaker and stops the agent from re-asking the identical call.
pub const POLICY_DENIED_MARKER: &str = "[policy-denied]";

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

/// Access level granted to a trusted root outside the workspace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TrustedAccess {
    /// Read + list only.
    #[default]
    Read,
    /// Read and write/edit.
    ReadWrite,
}

/// A directory outside the workspace the agent is explicitly granted access to.
/// Takes precedence over `workspace_only` and `forbidden_paths` for its subtree,
/// except for credential stores (see `SecurityPolicy::is_always_forbidden`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TrustedRoot {
    /// Absolute path (a leading `~` is expanded to the user's home).
    pub path: String,
    /// Whether the agent may write within this root.
    #[serde(default)]
    pub access: TrustedAccess,
}

/// Risk score for shell command execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRiskLevel {
    Low,
    Medium,
    High,
}

/// Coarse permission bucket the harness approval gate keys on.
///
/// Classification is **fail-closed**: a command that is not provably read-only
/// (and not a recognized network/destructive command) is treated as at least
/// [`CommandClass::Write`]. Across multiple shell segments the **highest** class
/// wins (so `ls | curl …` is `Network`). Variants are ordered low→high so
/// [`Ord`] / [`Iterator::max`] compose them directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandClass {
    /// Provably read-only / observational (curated safe-read allowlist).
    Read,
    /// State-changing but not inherently catastrophic — the fail-closed default
    /// for anything not recognized as read/network/destructive.
    Write,
    /// Reaches the network (curl/wget/ssh/scp/…). Always prompts, every tier.
    Network,
    /// Installs an OS / language package (system package manager, or a *global*
    /// npm/pnpm/yarn/cargo/pip install). Always-ask in every acting tier,
    /// including Full — mirrors the dedicated `install_tool` gate so shell
    /// installs can't slip past it. Project-local installs are ordinary `Write`.
    Install,
    /// Catastrophic / irreversible / privilege-escalating / system-control.
    /// Always prompts, even in Full.
    Destructive,
}

/// What the harness should do with an acting tool call of a given
/// [`CommandClass`] under the session's [`AutonomyLevel`]. Computed by
/// [`SecurityPolicy::gate_decision`]; the harness translates `Prompt` into an
/// `ApprovalGate` round-trip *before* the tool's `execute()` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    /// Run without prompting.
    Allow,
    /// Require explicit human approval before running.
    Prompt,
    /// Refuse outright — no in-tier prompt can authorize it (e.g. any act in
    /// read-only mode).
    Block,
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
    /// Directories outside the workspace the agent may access (read or read-write).
    pub trusted_roots: Vec<TrustedRoot>,
    /// Whether the agent may install OS packages via the `install_tool` tool.
    pub allow_tool_install: bool,
    /// Tool names the user has pre-approved ("Always allow"). The `ApprovalGate`
    /// skips the interactive prompt for any tool in this set. Sourced from
    /// `autonomy.auto_approve`; populated/cleared via `config.update_autonomy_settings`
    /// (or an "Always allow" decision) and observed live via `live_policy`.
    pub auto_approve: Vec<String>,
    pub tracker: ActionTracker,
    /// Lazily-cached canonical form of [`workspace_dir`].
    ///
    /// `validate_path` / `validate_parent_path` use the canonical workspace
    /// root to check resolved paths against `forbidden_paths`. Without a cache
    /// each call invokes `tokio::fs::canonicalize(&workspace_dir)` — one
    /// `stat(2)` + symlink walk on the same path on every file op. A single
    /// agent turn doing tens of read/edit/shell-path validations hits this
    /// repeatedly with identical input.
    ///
    /// `workspace_dir` is effectively immutable for a given `SecurityPolicy`
    /// (a config update builds a *new* policy via `from_config` and swaps the
    /// `Arc` in [`live_policy`]), so caching the resolved value is safe and
    /// stays correct across config updates.
    ///
    /// `Arc<OnceCell<_>>` so the struct stays `Clone` (clone the `Arc`) and
    /// init happens lazily on the first async call site without blocking
    /// constructors. Fallback (raw `workspace_dir` if canonicalize fails)
    /// matches the previous inline behavior exactly.
    ///
    /// Visibility is `pub` to match every other field on the struct: external
    /// crates (Cargo examples, downstream consumers) construct
    /// `SecurityPolicy` with the `..SecurityPolicy::default()` functional-update
    /// spread, and Rust requires every field of the target struct to be
    /// visible to the caller in that syntax — even fields supplied by the
    /// default. `pub(crate)` was an over-tight first cut that broke
    /// `examples/mouse_smoke.rs` with E0451.
    pub canonical_workspace: Arc<OnceCell<PathBuf>>,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: PathBuf::from("."),
            workspace_only: true,
            // When adding a new entry to this allowlist, re-audit
            // `DANGEROUS_ENV_PREFIXES` (see below). Every newly-allowed binary
            // may introduce its own env-driven subprocess hooks (pager, editor,
            // loader override, SSH/diff helper, preprocessor) — those names
            // must be added to the prefix denylist so that the
            // `KEY=cmd <allowed-binary>` shape cannot bypass allowlisting via
            // `skip_env_assignments` in `is_command_allowed`. Cross-ref #2636.
            allowed_commands: vec![
                // Version control
                "git".into(),
                // Package managers / build systems
                "npm".into(),
                "pnpm".into(),
                "yarn".into(),
                "cargo".into(),
                "make".into(),
                "cmake".into(),
                // Directory / file inspection (read-only, low-risk)
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
                "sort".into(),
                "uniq".into(),
                "diff".into(),
                "which".into(),
                "uname".into(),
                "basename".into(),
                "dirname".into(),
                "tr".into(),
                "cut".into(),
                "realpath".into(),
                "readlink".into(),
                "stat".into(),
                "file".into(),
                // Filesystem mutations (medium-risk — require approval in Supervised mode)
                "mkdir".into(),
                "touch".into(),
                "cp".into(),
                "mv".into(),
                "ln".into(),
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
            // Effectively unlimited — matches AutonomyConfig::default_max_actions_per_hour().
            // The rate-limiter check is `count <= max`, so u32::MAX is functionally
            // infinite without requiring an Option sentinel on the field type.
            max_actions_per_hour: u32::MAX,
            max_cost_per_day_cents: 500,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            trusted_roots: Vec::new(),
            allow_tool_install: false,
            auto_approve: Vec::new(),
            tracker: ActionTracker::new(),
            canonical_workspace: Arc::new(OnceCell::new()),
        }
    }
}

#[path = "policy_command.rs"]
mod policy_command;
use policy_command::{
    classify_segment, command_basename, contains_unquoted_char, contains_unquoted_single_ampersand,
    has_dangerous_env_prefix, has_hidden_execution, is_command_executor, normalized_command_name,
    skip_env_assignments, split_unquoted_segments,
};

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

            // High-risk = catastrophic / irreversible / privilege-escalating /
            // system-control commands ONLY. Interpreters (python/bash/…),
            // network tools (curl/wget/ssh/…), and ordinary rm/chmod/chown are
            // deliberately NOT high-risk: they are routine for a coding agent and
            // are treated as medium-risk below (prompted in Supervised, run in
            // Full). This keeps "Full access" actually able to run code while
            // still guarding the few irreversible / system-destroying commands.
            if matches!(
                base.as_str(),
                "mkfs"
                    | "dd"
                    | "shutdown"
                    | "reboot"
                    | "halt"
                    | "poweroff"
                    | "sudo"
                    | "su"
                    | "mount"
                    | "umount"
                    | "iptables"
                    | "ufw"
                    | "firewall-cmd"
                    | "useradd"
                    | "userdel"
                    | "usermod"
                    | "passwd"
            ) {
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
                "touch" | "mkdir" | "mv" | "cp" | "ln" | "rm" | "chmod" | "chown" | "curl"
                | "wget" | "nc" | "ncat" | "netcat" | "scp" | "ssh" | "ftp" | "telnet" => true,
                _ => false,
            };

            // Interpreters / code executors run arbitrary code — medium-risk
            // (that is the job of a coding agent): prompted in Supervised,
            // allowed in Full. They are no longer classified high-risk.
            let medium = medium || is_command_executor(base.as_str());

            saw_medium |= medium;
        }

        if saw_medium {
            CommandRiskLevel::Medium
        } else {
            CommandRiskLevel::Low
        }
    }

    /// Classify a shell command into a fail-closed [`CommandClass`]. The highest
    /// class across all `;`/`|`/`&&`/`||`/newline-separated segments wins, and a
    /// file redirect (`>`/`>>`) or `tee` lifts the class to at least `Write` no
    /// matter how benign the base looks (`cat x > y` writes `y`).
    ///
    /// This is the deterministic floor the harness gate keys on; an LLM-declared
    /// category may only *raise* it (`gate = max(rust_floor, llm_declared)`),
    /// never lower it.
    pub fn classify_command(&self, command: &str) -> CommandClass {
        let mut class = CommandClass::Read;
        for segment in split_unquoted_segments(command) {
            let cmd_part = skip_env_assignments(&segment);
            let mut words = cmd_part.split_whitespace();
            let Some(base_raw) = words.next() else {
                continue;
            };
            let base = normalized_command_name(base_raw);
            let args: Vec<String> = words.map(|w| w.to_ascii_lowercase()).collect();
            let joined = cmd_part.to_ascii_lowercase();
            class = class.max(classify_segment(&base, &args, &joined));
        }
        // A redirect or `tee` writes a file regardless of the base command.
        if contains_unquoted_char(command, '>')
            || command
                .split_whitespace()
                .any(|w| w == "tee" || w.ends_with("/tee"))
        {
            class = class.max(CommandClass::Write);
        }
        class
    }

    /// The gate decision for an acting tool call of `class` under this policy's
    /// autonomy tier. The harness turns `Prompt` into an `ApprovalGate`
    /// round-trip *before* the tool runs; `Block` is refused outright.
    ///
    /// Matrix: read-only allows only `Read`; ask-before-edit (`Supervised`)
    /// prompts on every acting class; full runs `Read`/`Write` silently but
    /// always prompts on `Network`/`Destructive`.
    pub fn gate_decision(&self, class: CommandClass) -> GateDecision {
        match self.autonomy {
            AutonomyLevel::ReadOnly => match class {
                CommandClass::Read => GateDecision::Allow,
                _ => GateDecision::Block,
            },
            AutonomyLevel::Supervised => match class {
                CommandClass::Read => GateDecision::Allow,
                _ => GateDecision::Prompt,
            },
            AutonomyLevel::Full => match class {
                CommandClass::Read | CommandClass::Write => GateDecision::Allow,
                CommandClass::Network | CommandClass::Install | CommandClass::Destructive => {
                    GateDecision::Prompt
                }
            },
        }
    }

    /// Defense-in-depth check for the harness-gated command flow (Option 2).
    ///
    /// The run / prompt / block decision is made by [`Self::gate_decision`] +
    /// the process-global `ApprovalGate` (which prompts the human *before*
    /// `execute()`), so by the time a tool calls this the command is either a
    /// read or an already-approved act. This enforces what must still hold:
    ///
    /// - **Read-only**: only `Read`-class commands run (`Block` otherwise).
    /// - **Supervised**: no *hidden execution* (command/process substitution,
    ///   backticks, background `&`) that could smuggle an unseen command past
    ///   the approval the human read. Plain redirects (`2>&1`, `> file`) and
    ///   pipes are fine here — `classify_command` already lifts redirects to
    ///   `Write` so the gate prompted on them, and the human approved the
    ///   literal command. Full is trusted and skips the structural guard.
    ///
    /// Returns the classified [`CommandClass`] on success.
    pub fn check_gated_command(&self, command: &str) -> Result<CommandClass, String> {
        let class = self.classify_command(command);
        if self.gate_decision(class) == GateDecision::Block {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Security policy: read-only mode — only read commands are \
                 permitted. Do not retry this command; use a read-only approach or report that it \
                 cannot be done in this mode."
            ));
        }
        if self.autonomy != AutonomyLevel::Full && has_hidden_execution(command) {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Command blocked: command/process substitution ($(…), \
                 <(…)), backticks, and background (&) are not allowed in this mode — they can run \
                 a hidden command the approval prompt wouldn't show. Plain redirects like `2>&1` \
                 are fine. Do not retry as-is; rewrite the command without these constructs."
            ));
        }
        Ok(class)
    }

    /// Parse an LLM-declared command category. This is an **escalate-only**
    /// hint: callers combine it with the deterministic floor via
    /// `classify_command(cmd).max(declared)`, so the model can *raise* the gate
    /// (e.g. flag a `Write` as `Destructive` to request confirmation) but can
    /// never lower what the runtime determined. Unknown / empty → `None`.
    pub fn parse_declared_class(declared: &str) -> Option<CommandClass> {
        match declared.trim().to_ascii_lowercase().as_str() {
            "read" => Some(CommandClass::Read),
            "write" => Some(CommandClass::Write),
            "network" => Some(CommandClass::Network),
            "install" => Some(CommandClass::Install),
            "destructive" => Some(CommandClass::Destructive),
            _ => None,
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
                "{POLICY_BLOCKED_MARKER} Command not allowed by security policy: {truncated}. \
                 Do not retry this command; it is off the allowlist for this mode."
            ));
        }

        let risk = self.command_risk_level(command);

        if risk == CommandRiskLevel::High {
            if self.block_high_risk_commands {
                log::warn!(
                    "[openhuman:policy] High-risk command blocked: {}",
                    &command[..floor_char_boundary(command, 80)]
                );
                return Err(format!(
                    "{POLICY_BLOCKED_MARKER} Command blocked: high-risk command is disallowed by \
                     policy. Do not retry this command; choose a safer approach or report that it \
                     cannot be done."
                ));
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

        // Full access bypasses the command allowlist AND the structural guards
        // (redirects, pipes, subshells, background) — a Full-access agent is
        // trusted to run any command, including the `mkdir`/`node`/`python`/
        // redirect-using commands a coding workflow needs. The remaining safety
        // net is `validate_command_execution`'s high-risk handling (still gated
        // by `block_high_risk_commands`), plus path-level `forbidden_paths` and
        // any configured sandbox. The allowlist + structural guards below stay
        // in force for Supervised, which runs only curated commands.
        if self.autonomy == AutonomyLevel::Full {
            return true;
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
            // Reject segments that prefix the command with a dangerous env
            // assignment (e.g. `GIT_PAGER=<cmd> git log`). The bare command
            // after the assignment is allowlisted, but the prefix mutates
            // the downstream binary's execution to spawn `<cmd>` as a
            // subprocess. See [`has_dangerous_env_prefix`].
            if has_dangerous_env_prefix(segment) {
                return false;
            }

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
                // -exec / -ok run a command per match. -execdir / -okdir do
                // the same with the working directory set to the match's
                // parent — same code-execution semantics, just with a
                // different cwd, so they must be blocked alongside.
                !args.iter().any(|arg| {
                    arg == "-exec" || arg == "-ok" || arg == "-execdir" || arg == "-okdir"
                })
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
            "date" => args.is_empty(),
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
        let expanded_path = Path::new(&expanded);

        // Credential stores are never reachable, even via a trusted-root grant.
        if Self::is_always_forbidden(expanded_path) {
            return false;
        }

        // A trusted root grants access to its subtree, taking precedence over
        // workspace_only and forbidden_paths. Read-vs-write is enforced by the
        // operation-specific validators (validate_path / validate_parent_path).
        let in_trusted_root = self.is_within_trusted_root(expanded_path, false);

        // Block absolute paths when workspace_only is set (unless trusted-rooted).
        if self.workspace_only && expanded_path.is_absolute() && !in_trusted_root {
            return false;
        }

        // Block forbidden paths using path-component-aware matching (unless trusted-rooted).
        if !in_trusted_root {
            for forbidden in &self.forbidden_paths {
                let forbidden_expanded = self.expand_tilde(forbidden);
                let forbidden_path = Path::new(&forbidden_expanded);
                if expanded_path.starts_with(forbidden_path) {
                    return false;
                }
            }
        }

        // Symlink-safe check (#1927). The string-level checks above can be
        // bypassed by creating a symlink inside the workspace that points to
        // a forbidden tree (e.g. `evil -> /etc/shadow`). Canonicalize the
        // path and re-validate `workspace_only` containment + forbidden_paths
        // against the resolved location.
        if let Some(canonical) = self.try_canonicalize_under_workspace(path) {
            if Self::is_always_forbidden(&canonical) {
                return false;
            }
            let workspace_root = self
                .workspace_dir
                .canonicalize()
                .unwrap_or_else(|_| self.workspace_dir.clone());
            let canonical_in_trusted = self.is_within_trusted_root(&canonical, false);
            if self.workspace_only
                && !canonical.starts_with(&workspace_root)
                && !canonical_in_trusted
            {
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
            if !inside_workspace && !canonical_in_trusted {
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

    /// Return the canonical form of `workspace_dir`, hydrating the
    /// `canonical_workspace` cache on the first call.
    ///
    /// `validate_path` / `validate_parent_path` both need the canonical
    /// workspace root for forbidden-path containment checks. The underlying
    /// `tokio::fs::canonicalize` is a `stat(2)` + symlink walk and was
    /// previously invoked on every call with the same input.
    ///
    /// Falls back to the raw `workspace_dir` if `canonicalize` fails (e.g.
    /// during early startup or in tests where the workspace doesn't exist on
    /// disk), matching the inline behavior the callers used before the cache.
    async fn workspace_root(&self) -> PathBuf {
        self.canonical_workspace
            .get_or_init(|| async {
                tokio::fs::canonicalize(&self.workspace_dir)
                    .await
                    .unwrap_or_else(|_| self.workspace_dir.clone())
            })
            .await
            .clone()
    }

    /// Validate a path for file I/O: string checks, canonicalize, workspace containment,
    /// and forbidden-path check on the resolved path.
    /// Returns the canonical `PathBuf` on success.
    pub async fn validate_path(&self, path: &str) -> Result<PathBuf, String> {
        if !self.is_path_string_allowed(path) {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Path not allowed by security policy: {path}. Do not \
                 retry this path; use an allowed location (the workspace or a granted folder)."
            ));
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
        if !self.is_resolved_path_allowed_for(&resolved, false) {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Resolved path escapes workspace: {}",
                resolved.display()
            ));
        }
        let workspace_root = self.workspace_root().await;
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
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Path not allowed by security policy: {path}. Do not \
                 retry this path; use an allowed location (the workspace or a granted folder)."
            ));
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
        if !self.is_resolved_path_allowed_for(&canonical_ancestor, true) {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Resolved parent path escapes workspace: {}",
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

        let workspace_root = self.workspace_root().await;
        self.check_resolved_against_forbidden(&canonical_ancestor, &workspace_root)?;
        self.check_resolved_against_forbidden(&result, &workspace_root)?;

        log::debug!(
            "[security] validate_parent_path: '{}' resolved parent to '{}'",
            path,
            resolved_parent.display()
        );
        Ok(result)
    }

    /// Paths that remain blocked even when a `trusted_root` grant would
    /// otherwise reach them — credential stores and core OS directories. A
    /// grant on a parent must never expose SSH/GPG/AWS/keychain secrets, nor
    /// open `/etc`, `C:\Windows`, `/System`, etc. Matching is **case-insensitive**
    /// (Windows/macOS filesystems are), so `.SSH` / `C:\WINDOWS` cannot slip
    /// through. Gray-area dirs (`/usr`, `/opt`, `/var`, `~/Library`) stay in the
    /// user-overridable `forbidden_paths` instead, so a grant can still reach
    /// e.g. `/usr/local/...`.
    fn is_always_forbidden(path: &Path) -> bool {
        // Normalize separators + case BEFORE splitting: a Windows backslash
        // path is a single component on POSIX (and vice-versa), so we segment
        // the normalized string rather than rely on `Path::components()`.
        let lc_path = path
            .to_string_lossy()
            .to_ascii_lowercase()
            .replace('\\', "/");
        let segments: Vec<&str> = lc_path.split('/').filter(|s| !s.is_empty()).collect();

        // (a) Credential stores — matched by path segment, location-independent
        // (catches e.g. `C:\Users\x\.ssh` and `~/Library/Keychains`).
        const SENSITIVE_COMPONENTS: &[&str] =
            &[".ssh", ".gnupg", ".aws", ".azure", ".kube", "keychains"];
        if segments.iter().any(|s| SENSITIVE_COMPONENTS.contains(s)) {
            return true;
        }
        // Windows DPAPI / credential stores live under `…\Microsoft\{Protect,
        // Credentials,Crypto,Vault}` — match the pair so the generic second
        // name can't false-positive an unrelated project directory.
        if segments.windows(2).any(|w| {
            w[0] == "microsoft" && matches!(w[1], "protect" | "credentials" | "crypto" | "vault")
        }) {
            return true;
        }

        // (b) Core OS directories — matched by absolute prefix. Unconditional,
        // unlike the user-overridable `forbidden_paths`.
        const SYSTEM_PREFIXES: &[&str] = &[
            // POSIX
            "/etc",
            "/root",
            "/boot",
            "/proc",
            "/sys",
            // macOS (note: /private is intentionally NOT blocked — macOS temp
            // dirs and /etc canonicalize under /private/var and /private/etc).
            "/system",
            // Windows
            "c:/windows",
            "c:/program files",
            "c:/program files (x86)",
            "c:/programdata",
        ];
        SYSTEM_PREFIXES
            .iter()
            .any(|p| lc_path == *p || lc_path.starts_with(&format!("{p}/")))
    }

    /// True if `path` is within a configured trusted root. When `require_write`
    /// is set, only `ReadWrite` roots match. Never matches credential stores.
    pub fn is_within_trusted_root(&self, path: &Path, require_write: bool) -> bool {
        if Self::is_always_forbidden(path) {
            return false;
        }
        self.trusted_roots.iter().any(|root| {
            if require_write && root.access != TrustedAccess::ReadWrite {
                return false;
            }
            let root_path = PathBuf::from(self.expand_tilde(&root.path));
            let canonical_root = root_path
                .canonicalize()
                .unwrap_or_else(|_| root_path.clone());
            path.starts_with(&root_path) || path.starts_with(&canonical_root)
        })
    }

    /// Validate that a resolved path is still inside the workspace.
    /// Call this AFTER joining `workspace_dir` + relative path and canonicalizing.
    pub fn is_resolved_path_allowed(&self, resolved: &Path) -> bool {
        self.is_resolved_path_allowed_for(resolved, false)
    }

    /// Operation-aware resolved-path check: allowed when under the workspace, or
    /// within a trusted root (write roots only when `require_write`). Prefers the
    /// canonical workspace root so `/a/../b` style config paths don't misfire.
    pub fn is_resolved_path_allowed_for(&self, resolved: &Path, require_write: bool) -> bool {
        if Self::is_always_forbidden(resolved) {
            return false;
        }
        let workspace_root = self
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_dir.clone());
        resolved.starts_with(&workspace_root)
            || self.is_within_trusted_root(resolved, require_write)
    }

    /// Check `resolved` against every entry in `forbidden_paths`, resolving relative
    /// entries against `workspace_root`. Absolute entries whose prefix IS the workspace
    /// root are skipped — the workspace containment check already covers them.
    fn check_resolved_against_forbidden(
        &self,
        resolved: &Path,
        workspace_root: &Path,
    ) -> Result<(), String> {
        // Credential stores are never reachable, even via a trusted-root grant.
        if Self::is_always_forbidden(resolved) {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Resolved path is a protected credential store: {}",
                resolved.display()
            ));
        }
        // A trusted-root grant takes precedence over forbidden_paths for its subtree.
        if self.is_within_trusted_root(resolved, false) {
            return Ok(());
        }
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
                    "{POLICY_BLOCKED_MARKER} Resolved path is inside a forbidden directory: {}",
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
                        "{POLICY_BLOCKED_MARKER} Security policy: read-only mode, cannot perform \
                         '{operation_name}'. Do not retry; this tier blocks all write actions."
                    ));
                }

                if !self.record_action() {
                    log::warn!(
                        "[openhuman:policy] Operation '{}' blocked: rate limit exceeded",
                        operation_name
                    );
                    return Err(format!(
                        "Rate limit exceeded: action budget exhausted ({} actions/hour). Increase the limit in Settings -> Advanced -> Agent autonomy or wait for the rolling one-hour window to refill.",
                        self.max_actions_per_hour
                    ));
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

        // `auto_approve` is the user's "Always allow" allowlist: the
        // `ApprovalGate` reads it via `live_policy::current()` and skips the
        // interactive prompt for any tool named in it. Tier + `CommandClass`
        // (and the unconditional read-only / forbidden-path / high-risk denials)
        // still run *before* the gate, so the allowlist can only suppress the
        // human prompt — it can never override a hard policy denial.

        // The default projects home (`~/OpenHuman/projects`) is always a
        // read-write trusted root so the coding agent can create/edit projects
        // there regardless of tier or `workspace_only`. Injected here — the one
        // autonomy→policy chokepoint every session goes through — because the
        // channels-startup injection is skipped on cores with no listening
        // integrations (web-chat-only), and a freshly reloaded config wouldn't
        // carry an in-memory edit anyway. A user-granted entry is left as-is.
        let mut trusted_roots = autonomy_config.trusted_roots.clone();
        let projects_path = crate::openhuman::config::default_projects_dir()
            .to_string_lossy()
            .to_string();
        if !trusted_roots.iter().any(|r| r.path == projects_path) {
            trusted_roots.push(TrustedRoot {
                path: projects_path,
                access: TrustedAccess::ReadWrite,
            });
        }

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
            trusted_roots,
            allow_tool_install: autonomy_config.allow_tool_install,
            auto_approve: autonomy_config.auto_approve.clone(),
            tracker: ActionTracker::new(),
            canonical_workspace: Arc::new(OnceCell::new()),
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
