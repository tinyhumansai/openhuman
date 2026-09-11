
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
    // `find` is read-only unless it executes commands, deletes, or writes
    // files. -fprintf / -fprint / -fprint0 / -fls write their output to a
    // named file rather than stdout — an arbitrary-path write that side-steps
    // the gated file-write tools (and their workspace confinement), so they
    // must be classified Write (approval-gated) alongside -delete.
    //
    // `args` is built with `split_whitespace()`, which keeps shell quotes that
    // the shell itself strips before `find` runs — `find . '-fprint' out`
    // arrives as the literal `'-fprint'`. Trim surrounding single/double quotes
    // before matching so a quoted predicate cannot slip the write past the gate.
    if base == "find" {
        if args.iter().any(|a| {
            matches!(
                a.trim_matches(|c| c == '\'' || c == '"'),
                "-exec"
                    | "-execdir"
                    | "-ok"
                    | "-okdir"
                    | "-delete"
                    | "-fprintf"
                    | "-fprint"
                    | "-fprint0"
                    | "-fls"
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
