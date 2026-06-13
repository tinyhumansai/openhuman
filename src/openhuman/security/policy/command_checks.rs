use crate::openhuman::util::floor_char_boundary;

use super::policy_command::{
    classify_segment, command_basename, contains_unquoted_char, contains_unquoted_single_ampersand,
    has_dangerous_env_prefix, has_hidden_execution, has_leading_env_assignment,
    is_command_executor, normalized_command_name, skip_env_assignments, split_unquoted_segments,
};
use super::types::{
    AutonomyLevel, CommandClass, CommandRiskLevel, GateDecision, SecurityPolicy,
    POLICY_BLOCKED_MARKER,
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
            // Reject ANY segment that prefixes the command with an env-var
            // assignment, not just the known-dangerous names. Helper-style
            // exec primitives (`GIT_SSH=./wrapper git ls-remote`,
            // `SSH_ASKPASS=./prompt ssh user@host`, `LD_PRELOAD=./libx.so
            // ls`, etc.) change which binary the allowed command actually
            // resolves to — or change its behaviour via a hook — without
            // any blocked command name ever appearing in the segment. The
            // allowlist already names every command we want to permit, and
            // none of those commands need an operator-set env var at
            // invoke time, so the broader gate has no false-positive
            // surface on the approved path. `has_dangerous_env_prefix`
            // remains in the source for legacy tests and as the
            // narrower-grained signal.
            if has_leading_env_assignment(segment) || has_dangerous_env_prefix(segment) {
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
}
