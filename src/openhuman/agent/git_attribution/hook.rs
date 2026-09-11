use std::collections::HashMap;
#[cfg(unix)]
use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::OnceLock;

/// OpenHuman's commit trailer.
pub const TRAILER: &str = "Co-authored-by: OpenHuman <openhuman@tinyhumans.ai>";

#[cfg(unix)]
const HOOKS: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "post-rewrite",
    "pre-auto-gc",
    "push-to-checkout",
    "reference-transaction",
    "sendemail-validate",
    "post-index-change",
];

#[cfg(unix)]
const SHIM: &str = r#"#!/bin/sh
# OpenHuman hook shim. Delegate to the repository hook before attributing.
hook_name=$(basename "$0")
self_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# Suppress only our injected config while resolving the repository's configured
# hook path. A project may intentionally set core.hooksPath away from .git/hooks.
orig_dir=$(GIT_CONFIG_COUNT=0 GIT_CONFIG_PARAMETERS= git config --path --get core.hooksPath 2>/dev/null)
if [ -z "$orig_dir" ]; then
    orig_dir=$(GIT_CONFIG_COUNT=0 GIT_CONFIG_PARAMETERS= git rev-parse --git-path hooks 2>/dev/null)
fi
if [ -n "$orig_dir" ] && [ "$orig_dir" != "$self_dir" ] && [ -x "$orig_dir/$hook_name" ]; then
    "$orig_dir/$hook_name" "$@" || exit $?
fi

[ "$hook_name" = "prepare-commit-msg" ] || exit 0
case "$2" in merge|squash) exit 0 ;; esac
git interpret-trailers --in-place --if-exists addIfDifferent \
    --trailer "$OPENHUMAN_GIT_ATTRIBUTION" "$1" 2>/dev/null || true
"#;

#[cfg(unix)]
static HOOK_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Environment values that activate OpenHuman's commit-attribution hook.
///
/// The result is intended for an agent-owned child process. It does not change
/// repository configuration or the parent application's environment.
#[cfg(unix)]
pub fn hook_env() -> HashMap<OsString, OsString> {
    let Some(dir) = HOOK_DIR.get_or_init(|| build_hook_dir().ok()).as_ref() else {
        return HashMap::new();
    };
    build_hook_env(dir, std::env::var_os("GIT_CONFIG_PARAMETERS").as_deref())
}

#[cfg(unix)]
fn build_hook_env(
    dir: &std::path::Path,
    inherited_parameters: Option<&OsStr>,
) -> HashMap<OsString, OsString> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let mut parameters = inherited_parameters
        .map(|value| value.as_bytes().to_vec())
        .unwrap_or_default();
    if !parameters.is_empty() {
        parameters.push(b' ');
    }
    parameters.extend_from_slice(b"'core.hooksPath'='");
    for byte in dir.as_os_str().as_bytes() {
        if *byte == b'\'' {
            parameters.extend_from_slice(b"'\\''");
        } else {
            parameters.push(*byte);
        }
    }
    parameters.push(b'\'');
    push_neutralised_config(&mut parameters);

    HashMap::from([
        (OsString::from("OPENHUMAN_GIT_ATTRIBUTION"), TRAILER.into()),
        // Parameters outrank GIT_CONFIG_COUNT. Preserve settings inherited
        // from the parent harness, then append our hook so it wins if that
        // harness also selected a hook path.
        (
            OsString::from("GIT_CONFIG_PARAMETERS"),
            OsString::from_vec(parameters),
        ),
    ])
}

#[cfg(not(unix))]
pub fn hook_env() -> HashMap<std::ffi::OsString, std::ffi::OsString> {
    HashMap::new()
}

/// Append the shell-safe git config overrides to a `GIT_CONFIG_PARAMETERS`
/// buffer.
///
/// `GIT_CONFIG_PARAMETERS` is how git itself propagates `-c` to child
/// processes, so these land at `-c` precedence: above the repository's own
/// config, which in an agent-writable workspace is the thing an attacker gets
/// to author. Several git config keys name a command git then runs —
/// `core.fsmonitor` on `git status`, `core.pager` and `core.editor` on the
/// commands that use them — and until `git_operations` moved behind a tool
/// pack, `shell git` was the one path that got none of that hardening.
///
/// The list, and the reasoning about what is deliberately left out of it,
/// lives with the rest of the policy in `tools::impl::filesystem`.
///
/// Later entries win, so this is appended after the hooks path rather than
/// before: nothing here sets `core.hooksPath`, but the ordering is the
/// contract and worth keeping explicit.
#[cfg(unix)]
fn push_neutralised_config(parameters: &mut Vec<u8>) {
    for entry in crate::openhuman::tools::implementations::filesystem::SHELL_NEUTRALISED_CONFIG {
        let Some((key, value)) = entry.split_once('=') else {
            continue;
        };
        parameters.push(b' ');
        parameters.push(b'\'');
        parameters.extend_from_slice(key.as_bytes());
        parameters.extend_from_slice(b"'='");
        parameters.extend_from_slice(value.as_bytes());
        parameters.push(b'\'');
    }
}

#[cfg(unix)]
fn build_hook_dir() -> std::io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir()?.keep();
    for name in HOOKS {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path)?;
        file.write_all(SHIM.as_bytes())?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(dir)
}

#[cfg(all(unix, test))]
pub(super) fn test_hook_env(inherited_parameters: Option<&OsStr>) -> HashMap<OsString, OsString> {
    let dir = build_hook_dir().expect("create OpenHuman hook directory");
    build_hook_env(&dir, inherited_parameters)
}
