// tinybox's shell helpers are deprecated in favour of tinysecurity-policy; the
// test-only comparisons below keep using them until that move lands.
#![allow(deprecated)]

use std::path::{Path, PathBuf};

#[cfg(test)]
use super::types::CommandRiskLevel;
use super::types::{AutonomyLevel, GateDecision, SecurityPolicy};
#[cfg(test)]
use crate::util::floor_char_boundary;
use tinybox_bus::CommandClass;
#[cfg(test)]
use tinybox_core::shell::classify::{classify_segment, has_hidden_execution};
#[cfg(test)]
use tinybox_core::shell::env_guard::has_dangerous_env_prefix;
#[cfg(test)]
use tinybox_core::shell::executor::is_command_executor;
#[cfg(test)]
use tinybox_core::shell::scan::{
    command_basename, contains_unquoted_char, contains_unquoted_single_ampersand,
    has_leading_env_assignment, normalized_command_name, skip_env_assignments,
    split_unquoted_segments, strip_quoted_heredoc_bodies,
};
use tinytools::status::POLICY_BLOCKED_MARKER;

impl SecurityPolicy {
    /// Apply the supervised allowlist to parser facts returned by TinyBox.
    /// The module owns shell structure; this host owns the permission policy.
    #[cfg(feature = "modules")]
    pub fn is_command_allowed_analysis(
        &self,
        analysis: &crate::modules::tinybox::contract::CommandAnalysis,
    ) -> bool {
        if !self.enabled {
            return true;
        }
        if self.autonomy == AutonomyLevel::ReadOnly {
            return false;
        }
        // Full access deliberately bypasses the supervised command allowlist.
        if self.autonomy == AutonomyLevel::Full {
            return true;
        }
        if analysis.hidden_execution
            || analysis.expansion
            || analysis.background
            || analysis.redirection
            || analysis.tee
        {
            return false;
        }

        let mut saw_command = false;
        for segment in &analysis.segments {
            if segment.command.trim().is_empty() {
                continue;
            }
            saw_command = true;
            if segment.leading_env_assignment || segment.dangerous_env_prefix || segment.executor {
                return false;
            }
            let base = segment.normalized_name.as_str();
            if base.is_empty() || !self.allowed_commands.iter().any(|allowed| allowed == base) {
                return false;
            }
            if !self.is_analysis_args_safe(base, &segment.arguments) {
                return false;
            }
        }
        saw_command
    }

    /// Apply OpenHuman's shell policy to generic facts returned by TinyBox.
    /// The module supplies parsing facts, while this host owns path checks,
    /// autonomy tiers, and approval decisions.
    #[cfg(feature = "modules")]
    pub fn check_gated_command_analysis(
        &self,
        analysis: &crate::modules::tinybox::contract::CommandAnalysis,
    ) -> Result<CommandClass, String> {
        self.check_protected_path_words(&analysis.literal_words)?;

        let mut class = analysis
            .segments
            .iter()
            .map(|segment| segment.class)
            .max()
            .unwrap_or(CommandClass::Read);
        if analysis.redirection || analysis.tee {
            class = class.max(CommandClass::Write);
        }

        if !self.enabled {
            return Ok(class);
        }
        if self.gate_decision(class) == GateDecision::Block {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Security policy: read-only mode — only read commands are permitted."
            ));
        }
        if self.autonomy != AutonomyLevel::Full && analysis.hidden_execution {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Command blocked: hidden command execution is not allowed in this mode."
            ));
        }
        Ok(class)
    }

    /// Classify command risk. Any high-risk segment marks the whole command high.
    #[cfg(test)]
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
    #[cfg(test)]
    pub fn classify_command(&self, command: &str) -> CommandClass {
        // A quoted heredoc body is document text, not shell. Without this the
        // splitter chops the body into "commands" and a prose line containing
        // `rm` or `curl` lifts a plain file write to Destructive/Network.
        let command = &strip_quoted_heredoc_bodies(command);
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
            let segment_class = classify_segment(&base, &args, &joined);
            class = class.max(segment_class);
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
        if !self.enabled {
            return GateDecision::Allow;
        }
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
    #[cfg(test)]
    pub fn check_gated_command(&self, command: &str) -> Result<CommandClass, String> {
        let class = self.classify_command(command);
        self.check_protected_path_literals(command)?;
        if !self.enabled {
            return Ok(class);
        }
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

    /// Refuse literal references to the protected-path floor before launching
    /// a command, including when the autonomy policy is disabled. This is a
    /// guard for direct path arguments and source literals; arbitrary shell or
    /// interpreter code still needs a filesystem sandbox to enforce the floor.
    #[cfg(test)]
    pub fn check_protected_path_literals(&self, input: &str) -> Result<(), String> {
        let words = strip_quoted_heredoc_bodies(input)
            .split(|c: char| {
                c.is_whitespace()
                    || matches!(c, '\'' | '"' | '(' | ')' | '[' | ']' | ',' | ';' | '=')
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        self.check_protected_path_words(&words)
    }

    /// Refuse protected paths and unsafe path literals returned by the
    /// execution module's command analysis. The host still owns path policy;
    /// the module only supplies token boundaries.
    pub fn check_protected_path_words(&self, words: &[String]) -> Result<(), String> {
        for word in words {
            let word = word.trim_matches(|c: char| matches!(c, ':' | '<' | '>' | '&' | '|'));
            if word.is_empty() || word.contains("://") {
                continue;
            }
            let candidate_path = PathBuf::from(word);
            if word.contains('\0')
                || candidate_path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(format!(
                    "{POLICY_BLOCKED_MARKER} Command references an unsafe path"
                ));
            }
            let word = ["${HOME}", "$HOME", "%USERPROFILE%"]
                .iter()
                .find_map(|prefix| word.strip_prefix(prefix).map(|rest| format!("~{rest}")))
                .unwrap_or_else(|| word.to_string());
            let candidate = PathBuf::from(self.expand_tilde(&word));
            let candidate = if candidate.is_absolute() {
                candidate
            } else {
                self.action_dir.join(candidate)
            };
            if Self::is_always_forbidden(&candidate)
                || candidate
                    .canonicalize()
                    .ok()
                    .is_some_and(|path| Self::is_always_forbidden(Path::new(&path)))
            {
                return Err(format!(
                    "{POLICY_BLOCKED_MARKER} Command references a protected path"
                ));
            }
        }
        Ok(())
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
    #[cfg(test)]
    pub fn validate_command_execution(
        &self,
        command: &str,
        approved: bool,
    ) -> Result<CommandRiskLevel, String> {
        if !self.enabled {
            return Ok(self.command_risk_level(command));
        }
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
    #[cfg(test)]
    pub fn is_command_allowed(&self, command: &str) -> bool {
        if !self.enabled {
            return true;
        }
        if self.autonomy == AutonomyLevel::ReadOnly {
            return false;
        }
        // See `classify_command`: a quoted heredoc body is data, so it must not
        // be scanned for `$(`, `&` or a disallowed base command.
        let command = &strip_quoted_heredoc_bodies(command);

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
            if is_command_executor(base_cmd) || !self.is_args_safe(base_cmd, &args) {
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
        self.is_analysis_args_safe(base, args)
    }

    fn is_analysis_args_safe(&self, base: &str, args: &[String]) -> bool {
        let base = base.to_ascii_lowercase();

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
