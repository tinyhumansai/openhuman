//! Shell commands under local-only privacy mode while security is disabled.
//!
//! With security enforced, TinySecurity classifies every command, including
//! whether it reaches the network. With security disabled
//! (`crate::security::switch`) nothing is classified and commands are allowed,
//! except under `[privacy] mode = "local_only"`. That mode is the user's own
//! network setting, not a module check, so it stays enforced: the host refuses
//! a command that may reach the network.
//!
//! The classification here is deliberately conservative and narrow. It reads
//! the program of every command in a pipeline or list and refuses:
//!
//! - a known network client (`curl`, `wget`, `nc`, `ssh`, `scp`, `ftp`, ...);
//! - a network subcommand of a local tool (`git clone/fetch/pull/push`,
//!   `npm install`, `pip install`, `cargo install`, `docker pull`, ...), and
//!   `rsync` with a remote endpoint;
//! - anything it cannot read with confidence: command or process
//!   substitution, parameter expansion, unbalanced quotes, inline code passed
//!   to a shell or an interpreter (`sh -c`, `python -c`, `eval`), and runners
//!   whose program is built from input (`xargs`).
//!
//! Scripts and build tools (`make`, `cargo build`, `python app.py`) are not
//! opened: their own network use is outside what a command line shows.

use super::native_commands::{CommandQuery, CommandVerdict, Refusal};
use super::types::{CommandClass, GateDecision, SecurityPolicy};
use crate::config::PrivacyMode;

impl SecurityPolicy {
    /// The verdict for a command TinySecurity is not asked about (security
    /// disabled): allowed, unless local-only privacy mode is active and the
    /// command may reach the network.
    pub(super) fn unchecked_command_verdict(
        &self,
        query: &CommandQuery<'_>,
    ) -> Result<CommandVerdict, Refusal> {
        let network = may_reach_network(query.command);
        if network && self.local_only() {
            tracing::debug!(
                "[privacy][local-only] refused a shell command that may reach the network"
            );
            return Err(Refusal::LocalOnly);
        }
        Ok(CommandVerdict {
            class: unchecked_class(query, network),
            gate: GateDecision::Allow,
        })
    }

    /// Whether local-only privacy mode is active: on this policy, or on the live
    /// session policy the egress chokepoints read.
    fn local_only(&self) -> bool {
        self.privacy_mode() == PrivacyMode::LocalOnly
            || crate::security::live_policy::current_privacy_mode() == PrivacyMode::LocalOnly
    }
}

/// The class of a command nobody classified: what the model declared, else
/// `Network` for a command that may reach the network and `Write` otherwise.
/// Informational only, since a disabled scope allows every class.
pub(super) fn unchecked_class(query: &CommandQuery<'_>, network: bool) -> CommandClass {
    query.declared.unwrap_or(if network {
        CommandClass::Network
    } else {
        CommandClass::Write
    })
}

/// Programs whose purpose is to talk to the network.
const NETWORK_CLIENTS: &[&str] = &[
    "curl",
    "wget",
    "nc",
    "ncat",
    "netcat",
    "socat",
    "ssh",
    "scp",
    "sftp",
    "ftp",
    "lftp",
    "telnet",
    "http",
    "https",
    "httpie",
    "xh",
    "aria2c",
    "ping",
    "ping6",
    "dig",
    "nslookup",
    "host",
    "whois",
    "traceroute",
    "tracepath",
    "mosh",
    "rclone",
    "gsutil",
    "aws",
    "gcloud",
    "az",
    "kubectl",
    "helm",
];

/// Shells and interpreters that run inline code given with one of these flags.
const INLINE_CODE: &[(&str, &[&str])] = &[
    ("sh", &["-c"]),
    ("bash", &["-c"]),
    ("zsh", &["-c"]),
    ("dash", &["-c"]),
    ("ksh", &["-c"]),
    ("fish", &["-c"]),
    ("python", &["-c"]),
    ("python3", &["-c"]),
    ("python2", &["-c"]),
    ("node", &["-e", "--eval", "-p", "--print"]),
    ("deno", &["eval"]),
    ("bun", &["-e", "--eval"]),
    ("perl", &["-e", "-E"]),
    ("ruby", &["-e"]),
    ("php", &["-r"]),
    ("osascript", &["-e"]),
    ("powershell", &["-c", "-command"]),
    ("pwsh", &["-c", "-command"]),
];

/// Programs that run text they are given, or a program built from input.
const OPAQUE: &[&str] = &["eval", "source", ".", "xargs", "parallel", "watch"];

/// Programs that run the rest of the line as a command, after their options.
const WRAPPERS: &[&str] = &[
    "sudo", "doas", "env", "nohup", "time", "nice", "ionice", "timeout", "command", "exec",
    "stdbuf", "builtin", "chrt", "setsid", "unbuffer",
];

/// Subcommands of local tools that fetch from, or send to, the network.
const NETWORK_SUBCOMMANDS: &[(&str, &[&str])] = &[
    (
        "git",
        &[
            "clone",
            "fetch",
            "pull",
            "push",
            "ls-remote",
            "remote",
            "submodule",
            "archive",
        ],
    ),
    (
        "npm",
        &[
            "install", "i", "ci", "add", "update", "publish", "view", "exec",
        ],
    ),
    ("npx", &[""]),
    ("pnpm", &["install", "i", "add", "update", "dlx", "publish"]),
    ("yarn", &["install", "add", "upgrade", "dlx", "publish"]),
    ("bunx", &[""]),
    ("pip", &["install", "download"]),
    ("pip3", &["install", "download"]),
    ("pipx", &["install", "run"]),
    ("uv", &["pip", "add", "sync", "tool"]),
    ("uvx", &[""]),
    ("poetry", &["install", "add", "update", "publish"]),
    (
        "cargo",
        &["install", "fetch", "update", "publish", "add", "search"],
    ),
    ("go", &["get", "install", "mod"]),
    ("gem", &["install", "update", "fetch"]),
    ("bundle", &["install", "update"]),
    ("composer", &["install", "require", "update"]),
    ("apt", &["install", "update", "upgrade", "download"]),
    ("apt-get", &["install", "update", "upgrade", "download"]),
    ("dnf", &["install", "update", "upgrade", "download"]),
    ("yum", &["install", "update", "upgrade"]),
    ("apk", &["add", "update", "upgrade", "fetch"]),
    ("pacman", &["-S", "-Sy", "-Syu"]),
    ("brew", &["install", "update", "upgrade", "fetch", "tap"]),
    ("docker", &["pull", "push", "login", "search"]),
    ("podman", &["pull", "push", "login", "search"]),
];

/// Whether `command` may reach the network, or cannot be read with confidence.
pub(crate) fn may_reach_network(command: &str) -> bool {
    if ["`", "$(", "<(", ">(", "${"]
        .iter()
        .any(|construct| command.contains(construct))
    {
        return true;
    }
    let Some(segments) = segments(command) else {
        return true;
    };
    segments
        .iter()
        .any(|words| segment_may_reach_network(words))
}

/// The words of every simple command in `command`, split at `;`, `&`, `|`,
/// newlines and parentheses. `None` when quoting is unbalanced.
fn segments(command: &str) -> Option<Vec<Vec<String>>> {
    let mut segments = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        other => word.push(other),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => word.push(chars.next()?),
                        other => word.push(other),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(next) = chars.next() {
                    if next != '\n' {
                        word.push(next);
                    }
                }
            }
            ';' | '&' | '|' | '\n' | '(' | ')' => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
                if !words.is_empty() {
                    segments.push(std::mem::take(&mut words));
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    if !words.is_empty() {
        segments.push(words);
    }
    Some(segments)
}

/// Whether one simple command may reach the network.
fn segment_may_reach_network(words: &[String]) -> bool {
    let words = strip_prefix(words);
    let Some((program, args)) = words.split_first() else {
        return false;
    };
    let program = program_name(program);
    if NETWORK_CLIENTS.contains(&program) || OPAQUE.contains(&program) {
        return true;
    }
    if let Some((_, flags)) = INLINE_CODE
        .iter()
        .find(|(name, _)| program_matches(name, program))
    {
        if args.iter().any(|arg| {
            let arg = arg.to_ascii_lowercase();
            flags.iter().any(|flag| arg == *flag)
        }) {
            return true;
        }
    }
    if program == "rsync" {
        return args.iter().any(|arg| remote_endpoint(arg));
    }
    if program == "find" {
        return args
            .iter()
            .position(|arg| matches!(arg.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"))
            .is_some_and(|at| segment_may_reach_network(&args[at + 1..]));
    }
    if let Some((_, subcommands)) = NETWORK_SUBCOMMANDS
        .iter()
        .find(|(name, _)| *name == program)
    {
        if subcommands == &[""] {
            return true;
        }
        let Some(subcommand) = first_subcommand(program, args) else {
            return false;
        };
        return subcommands.contains(&subcommand);
    }
    false
}

/// Drop leading variable assignments and wrapper programs with their options,
/// leaving the program that actually runs.
fn strip_prefix(mut words: &[String]) -> &[String] {
    loop {
        let Some((first, rest)) = words.split_first() else {
            return words;
        };
        if is_assignment(first) {
            words = rest;
            continue;
        }
        let name = program_name(first);
        if !WRAPPERS.contains(&name) {
            return words;
        }
        words = rest;
        // Options of the wrapper, and the duration `timeout` takes.
        while let Some((next, rest)) = words.split_first() {
            if next.starts_with('-') || is_assignment(next) {
                let takes_value = matches!(next.as_str(), "-n" | "-u" | "-g" | "-s" | "-k" | "-C");
                words = rest;
                if takes_value {
                    words = words.split_first().map_or(words, |(_, rest)| rest);
                }
            } else if name == "timeout" && next.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                words = rest;
            } else {
                break;
            }
        }
    }
}

/// The first subcommand of `program`, skipping its global options (and the
/// values `git -C <dir>` and `git -c <key=value>` take).
fn first_subcommand<'a>(program: &str, args: &'a [String]) -> Option<&'a str> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if program == "pacman" && arg.starts_with("-S") {
            return Some(if arg.as_str() == "-S" { "-S" } else { "-Sy" });
        }
        if arg.starts_with('-') {
            if program == "git" && matches!(arg.as_str(), "-C" | "-c" | "--git-dir" | "--work-tree")
            {
                args.next();
            }
            continue;
        }
        return Some(arg.as_str());
    }
    None
}

/// The program's file name, so `/usr/bin/curl` reads as `curl`.
fn program_name(word: &str) -> &str {
    word.rsplit(['/', '\\']).next().unwrap_or(word)
}

/// Whether `program` is `name` or a versioned form of it (`python3.12`).
fn program_matches(name: &str, program: &str) -> bool {
    program == name
        || program
            .strip_prefix(name)
            .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

/// `NAME=value` before a command.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// An rsync endpoint on another host: `host:path`, `user@host:path` or an
/// `rsync://` URL. A local path with a colon after a slash is not remote.
fn remote_endpoint(arg: &str) -> bool {
    if arg.starts_with('-') {
        return false;
    }
    if arg.contains("://") {
        return true;
    }
    match arg.find(':') {
        Some(colon) => !arg[..colon].contains('/'),
        None => false,
    }
}

#[cfg(test)]
#[path = "local_only_commands_tests.rs"]
mod tests;
