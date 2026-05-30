use super::CommandClass;

/// Environment variable names that can trigger arbitrary command execution
/// when supplied as a leading inline assignment on an otherwise-allowed
/// command. Each name here is either a hook variable that a downstream tool
/// will spawn as a subprocess (`GIT_PAGER`, `GIT_SSH_COMMAND`, `EDITOR`,
/// `LESS`/`LESSOPEN`, `MANPAGER`, `BROWSER`, `BAT_PAGER`), a runtime
/// configuration knob that affects how Python or the shell evaluate user
/// input (`PYTHONSTARTUP`, `BASH_ENV`, `ENV`, `PROMPT_COMMAND`), or a loader
/// override that lets an attacker inject a library into the next process
/// (`LD_PRELOAD`, `LD_LIBRARY_PATH`, `LD_AUDIT`, `DYLD_INSERT_LIBRARIES`,
/// `DYLD_LIBRARY_PATH`, `DYLD_FORCE_FLAT_NAMESPACE`).
///
/// `PATH` and `SHELL` are listed so an inline override cannot redirect
/// resolution of any allowed binary to an attacker-controlled path. `IFS`
/// is listed because the shell uses it for word splitting and a malicious
/// value can hide command boundaries from later parsers.
const DANGEROUS_ENV_PREFIXES: &[&str] = &[
    "BASH_ENV",
    "BAT_PAGER",
    "BROWSER",
    "DYLD_FORCE_FLAT_NAMESPACE",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "EDITOR",
    "ENV",
    "GIT_EDITOR",
    "GIT_EXTERNAL_DIFF",
    "GIT_EXTERNAL_FILTER",
    "GIT_PAGER",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "IFS",
    "LD_AUDIT",
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "LESS",
    "LESSCLOSE",
    "LESSOPEN",
    "MANOPT",
    "MANPAGER",
    "PAGER",
    "PATH",
    "PROMPT_COMMAND",
    "PS1",
    "PS2",
    "PS3",
    "PS4",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "SHELL",
    "VISUAL",
];

/// Returns true if `s` starts with one or more inline env assignments and any
/// of the assigned names are in [`DANGEROUS_ENV_PREFIXES`].
///
/// The allowlist validation in [`SecurityPolicy::is_command_allowed`] uses
/// [`skip_env_assignments`] to look past the env prefix before matching the
/// command name. That leaves a class of attacks where the bare command (e.g.
/// `git log`) is allowlisted but the env prefix mutates how it executes (e.g.
/// `GIT_PAGER=<cmd> git log` — `git` spawns `<cmd>` as its pager). Because
/// the prefix is stripped before allowlisting and the shell evaluates the
/// prefix at execution time, the bypass lands without ever touching a
/// blocked command name.
///
/// Treating any dangerous prefix as a denial keeps the allowlist
/// semantically meaningful without having to enumerate every shape of every
/// downstream tool's hook surface.
pub(super) fn has_dangerous_env_prefix(s: &str) -> bool {
    let mut rest = s.trim_start();
    loop {
        let Some(word) = rest.split_whitespace().next() else {
            return false;
        };
        if !word.contains('=') {
            return false;
        }
        if !word
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            return false;
        }
        let (name, _) = word.split_once('=').unwrap_or((word, ""));
        let upper = name.to_ascii_uppercase();
        if DANGEROUS_ENV_PREFIXES.iter().any(|d| *d == upper.as_str()) {
            return true;
        }
        rest = rest[word.len()..].trim_start();
    }
}

/// Skip leading environment variable assignments (e.g. `FOO=bar cmd args`).
/// Returns the remainder starting at the first non-assignment word.
pub(super) fn skip_env_assignments(s: &str) -> &str {
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

pub(super) fn command_basename(command: &str) -> &str {
    command
        .split(|ch| ch == '/' || ch == '\\')
        .next_back()
        .unwrap_or(command)
}

pub(super) fn normalized_command_name(command: &str) -> String {
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

pub(super) fn is_command_executor(command: &str) -> bool {
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
                // JS/TS runtimes (the `node_exec`/`npm_exec` shell equivalents)
                | "node"
                | "nodejs"
                | "deno"
                | "bun"
                // Windows / PowerShell arbitrary-code launchers + LOLBins
                | "iex"
                | "invoke-expression"
                | "cmd"
                | "pwsh"
                | "powershell"
                | "wscript"
                | "cscript"
                | "mshta"
                | "rundll32"
                | "start-process"
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
pub(super) fn split_unquoted_segments(command: &str) -> Vec<String> {
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
pub(super) fn contains_unquoted_single_ampersand(command: &str) -> bool {
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

/// Like [`contains_unquoted_single_ampersand`] but ignores file-descriptor
/// duplication redirects, where the `&` is part of a redirect operator rather
/// than a background/separator: `2>&1`, `>&2` (prev char `>`), and `&>file`
/// (next char `>`). Used by [`has_hidden_execution`] so a benign `… 2>&1` —
/// which `classify_command` already accounts for as a `Write` redirect — is not
/// mistaken for a backgrounded command and hard-blocked after the human
/// approved it. A standalone `&` (e.g. `cmd &`, `a & b`) still returns true,
/// since it can run a second command `classify_command` wouldn't see.
fn contains_unquoted_background_ampersand(command: &str) -> bool {
    let mut quote = QuoteState::None;
    let mut escaped = false;
    let mut prev = '\0';
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
                    prev = ch;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    prev = ch;
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::None;
                }
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    prev = ch;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    prev = ch;
                    continue;
                }
                match ch {
                    '\'' => quote = QuoteState::Single,
                    '"' => quote = QuoteState::Double,
                    '&' => {
                        if chars.next_if_eq(&'&').is_some() {
                            // `&&` logical AND — consume both, not background.
                        } else {
                            let next = chars.peek().copied().unwrap_or('\0');
                            // Skip fd-dup redirects: `2>&1`/`>&2` (prev `>`) and
                            // `&>file` (next `>`).
                            if prev != '>' && next != '>' {
                                return true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        prev = ch;
    }

    false
}

/// Detect an unquoted character in a shell command.
pub(super) fn contains_unquoted_char(command: &str, target: char) -> bool {
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

/// Provably read-only command bases (cross-platform union). A base **not** in
/// this set — and not a recognized network/destructive/executor command, nor a
/// read-only verb of git/npm/cargo — falls through to [`CommandClass::Write`]
/// (the classifier is fail-closed). Conservative on purpose: anything that can
/// write a file under a common flag is intentionally omitted (`sort -o`, `tee`).
const READ_ONLY_BASES: &[&str] = &[
    // POSIX inspection / read-only coreutils
    "ls",
    "cat",
    "pwd",
    "echo",
    "wc",
    "head",
    "tail",
    "date",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "which",
    "whoami",
    "id",
    "hostname",
    "uname",
    "printenv",
    "stat",
    "file",
    "du",
    "df",
    "tree",
    "realpath",
    "readlink",
    "dirname",
    "basename",
    "cmp",
    "true",
    "false",
    "sleep",
    "seq",
    "tty",
    "groups",
    "locale",
    "ps",
    "top",
    "free",
    "uptime",
    "lsblk",
    "lscpu",
    "cut",
    // Windows cmd / PowerShell read verbs + common aliases
    "dir",
    "type",
    "where",
    "whereis",
    "get-childitem",
    "gci",
    "get-content",
    "gc",
    "get-location",
    "gl",
    "select-string",
    "sls",
    "measure-object",
    "get-item",
    "gi",
    "test-path",
    "resolve-path",
    "get-command",
    "gcm",
    "get-process",
];

/// Commands that reach the network. Always-ask in every acting tier.
const NETWORK_BASES: &[&str] = &[
    "curl",
    "wget",
    "ssh",
    "scp",
    "sftp",
    "rsync",
    "nc",
    "ncat",
    "netcat",
    "telnet",
    "ftp",
    "tftp",
    "socat",
    // Windows / PowerShell
    "invoke-webrequest",
    "iwr",
    "invoke-restmethod",
    "irm",
    "start-bitstransfer",
    "bitsadmin",
];

/// Catastrophic / irreversible / privilege / system-control bases. Always-ask
/// in every acting tier (Full included). Coarse on the broad Windows verbs
/// (`reg`/`net`/`sc`) — over-prompting there is the safe default.
const DESTRUCTIVE_BASES: &[&str] = &[
    // POSIX privilege / disk / system-control
    "sudo",
    "su",
    "doas",
    "dd",
    "mkfs",
    "fdisk",
    "sfdisk",
    "parted",
    "wipefs",
    "shred",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "init",
    "telinit",
    "mount",
    "umount",
    "swapoff",
    "iptables",
    "ip6tables",
    "nft",
    "ufw",
    "firewall-cmd",
    "useradd",
    "userdel",
    "usermod",
    "groupadd",
    "groupdel",
    "passwd",
    "chpasswd",
    "visudo",
    "modprobe",
    "insmod",
    "rmmod",
    // Windows / PowerShell
    "format",
    "diskpart",
    "bcdedit",
    "takeown",
    "cipher",
    "vssadmin",
    "reg",
    "regedit",
    "runas",
    "sc",
    "net",
    "set-executionpolicy",
    "stop-computer",
    "restart-computer",
    "clear-disk",
    "format-volume",
    "remove-partition",
    "disable-computerrestore",
];

/// Git subcommands that only read repository state. Anything else — including
/// `commit`/`push`/`branch`/`config`/unknown/bare `git` — is fail-closed to
/// `Write`.
const GIT_READ_VERBS: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "remote",
    "describe",
    "blame",
    "ls-files",
    "ls-tree",
    "rev-parse",
    "cat-file",
    "shortlog",
    "reflog",
    "rev-list",
    "name-rev",
    "var",
    "check-ignore",
    "check-attr",
    "verify-commit",
    "count-objects",
    "fsck",
    "whatchanged",
    "grep",
    "version",
    "help",
];

/// npm/pnpm/yarn read-only subcommands. `install`/`run`/`test`/`exec` (which
/// run arbitrary scripts) and unknown verbs are fail-closed to `Write`.
const NODE_PKG_READ_VERBS: &[&str] = &[
    "ls", "list", "view", "info", "outdated", "ping", "whoami", "help", "why", "audit", "doctor",
];

/// cargo read-only subcommands. `build`/`run`/`test`/`check` compile and may
/// run build scripts, so they are fail-closed to `Write`.
const CARGO_READ_VERBS: &[&str] = &["tree", "metadata", "search", "info", "version", "help"];

/// Detect a pacman *install/upgrade* from its bundled operation flag.
///
/// pacman packs its operation and modifiers into a single flag (`-Syu`, `-Ss`),
/// and `args` reach us already lowercased — so the `-S` (sync) operation is
/// indistinguishable from a literal `-s` by case alone. We therefore key off
/// the *modifier* letters instead of a blanket `starts_with("-s")`, which would
/// over-match every read-only `-S` query: a `-S`-family flag mutates the host
/// only when it carries none of pacman's read-only query modifiers — search
/// (`s`), info (`i`), list (`l`), groups (`g`) or print (`p`). So `-S pkg`,
/// `-Sy`, `-Syu` are installs while `-Ss`/`-Si`/`-Sl`/`-Sg`/`-Sp` are reads.
fn is_pacman_install(args: &[String]) -> bool {
    args.iter().any(|a| {
        a.strip_prefix("-s")
            .is_some_and(|modifiers| !modifiers.contains(['s', 'i', 'l', 'g', 'p']))
    })
}

/// Detect a package-manager *install* invocation. These mutate the host /
/// global environment, so they are the always-ask `Install` bucket (even in
/// Full) — the same gate the dedicated `install_tool` enforces, applied to the
/// shell escape hatch. Project-local installs (`npm install` without `-g`,
/// `cargo add`) are ordinary `Write`s and are deliberately NOT matched here.
/// `args` are already lowercased by the caller.
fn is_install_command(base: &str, args: &[String]) -> bool {
    let has = |needle: &str| args.iter().any(|a| a == needle);
    let first_is = |verb: &str| args.first().map(String::as_str) == Some(verb);
    match base {
        // System package managers.
        "apt" | "apt-get" | "dnf" | "yum" | "zypper" => has("install"),
        "pacman" => is_pacman_install(args),
        "apk" => has("add"),
        "brew" | "snap" | "flatpak" | "winget" | "choco" | "scoop" => has("install"),
        // Language package managers — host/global-modifying installs only.
        "pip" | "pip3" | "pipx" | "gem" | "go" | "cargo" => first_is("install"),
        "npm" | "pnpm" => {
            (has("install") || has("i") || has("add")) && (has("-g") || has("--global"))
        }
        "yarn" => has("global"),
        _ => false,
    }
}

/// Classify a single already-split shell segment. `base` is the normalized
/// (lowercased, `.exe`-stripped, basename-only) program name; `args` are the
/// lowercased remaining words; `joined` is the lowercased segment used for
/// pattern matching. Fail-closed: an unrecognized base resolves to `Write`.
pub(super) fn classify_segment(base: &str, args: &[String], joined: &str) -> CommandClass {
    // Catastrophic patterns first — they win regardless of the base command.
    if joined.contains("rm -rf /") || joined.contains("rm -fr /") || joined.contains(":(){:|:&};:")
    {
        return CommandClass::Destructive;
    }
    if DESTRUCTIVE_BASES.contains(&base) {
        return CommandClass::Destructive;
    }
    if NETWORK_BASES.contains(&base) {
        return CommandClass::Network;
    }
    // Package installs mutate the host → always-ask Install bucket (closes the
    // shell escape hatch around `install_tool`).
    if is_install_command(base, args) {
        return CommandClass::Install;
    }
    // Interpreters / code executors run arbitrary code. Fail-closed to Write
    // (not Destructive) so Full can still run code while Supervised prompts.
    if is_command_executor(base) {
        return CommandClass::Write;
    }
    // `find` is read-only unless it executes commands or deletes files.
    if base == "find" {
        if args.iter().any(|a| {
            matches!(
                a.as_str(),
                "-exec" | "-execdir" | "-ok" | "-okdir" | "-delete"
            )
        }) {
            return CommandClass::Write;
        }
        return CommandClass::Read;
    }
    // Verb-sensitive VCS / package tools.
    if base == "git" {
        return verb_class(args, GIT_READ_VERBS);
    }
    if matches!(base, "npm" | "pnpm" | "yarn") {
        return verb_class(args, NODE_PKG_READ_VERBS);
    }
    if base == "cargo" {
        return verb_class(args, CARGO_READ_VERBS);
    }
    if READ_ONLY_BASES.contains(&base) {
        return CommandClass::Read;
    }
    // Fail closed: unknown or known-mutating base → Write.
    CommandClass::Write
}

/// `Read` when the first subcommand word is in `read_verbs`, else fail-closed
/// `Write`. Mirrors the `args.first()` verb check used by `command_risk_level`.
fn verb_class(args: &[String], read_verbs: &[&str]) -> CommandClass {
    match args.first().map(String::as_str) {
        Some(verb) if read_verbs.contains(&verb) => CommandClass::Read,
        _ => CommandClass::Write,
    }
}

/// Structural-safety guard for the harness-gated command flow (Option 2). Even
/// after a human approves a command, a hidden subshell / command substitution /
/// output redirect / `tee` / background `&` could smuggle a *different* command
/// past the approval summary, so these are refused outside Full (which is
/// trusted to use redirects and pipes). Mirrors the structural checks in
/// [`SecurityPolicy::is_command_allowed`].
/// Detect shell structure that can **hide execution** from `classify_command`,
/// which only inspects the base command of each `;`/`&&`/`|` segment. Command
/// and process substitution and backticks run an *inner* command classification
/// can't see (`echo $(rm -rf ~)` classifies as `echo` = Read and would run
/// unprompted), and a trailing `&` detaches a process past the gate — so these
/// stay hard-blocked outside Full.
///
/// Deliberately NOT flagged here: plain redirects (`>`, `2>&1`, `2>/dev/null`),
/// `tee`, and `${VAR}` expansion. `classify_command` already lifts a redirect /
/// `tee` to `Write`, so the gate prompts and — once the human approves — the
/// command MUST actually run. Re-blocking an approved `… 2>&1` here was the bug
/// that made Supervised mode unusable: every command the agent wrote carried a
/// `2>&1`, got approved, then silently failed this in-tool guard and never ran.
pub(super) fn has_hidden_execution(command: &str) -> bool {
    // The backtick check is deliberately NOT quote-aware: any backtick in the
    // command string is blocked, even inside a double-quoted literal. Over-
    // blocking is the safe direction here. (By contrast the `&` case below is
    // quote-aware via `contains_unquoted_background_ampersand`, because that one
    // must still allow benign fd-dup redirects like `2>&1`.)
    command.contains('`')
        || command.contains("$(")
        || command.contains("<(")
        || command.contains(">(")
        || contains_unquoted_background_ampersand(command)
}
