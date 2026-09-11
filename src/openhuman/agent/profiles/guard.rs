//! Active-profile identity plumbing + the cross-profile write guard.
//!
//! Two concerns, both keyed off *which* profile a turn runs under:
//!
//! 1. **Identity plumbing (1a).** When a dedicated-workspace profile is active,
//!    its id is carried to the tool layer inside the
//!    [`WorkspaceDescriptor`](tinyagents_harness::workspace::WorkspaceDescriptor)'s
//!    `policy_id` field as `openhuman.profile:<id>`. [`workspace_policy_id`] and
//!    [`profile_id_from_policy_id`] are the single encode/decode pair so the
//!    session builder and the tool gates can never drift on the wire format.
//!
//! 2. **Cross-profile write guard (1b).** A hermes `file_safety` equivalent:
//!    while a turn runs under a dedicated-workspace profile `P`, any tool
//!    write/command whose resolved target lands in a *sibling* profile's
//!    workspace `<action_dir>/profiles/<Q>/` (Q != P) is blocked. See
//!    [`classify_cross_profile_target`] (file tools) and
//!    [`scan_command_for_cross_profile`] (shell / `node_exec` / `npm_exec`). The
//!    guard only ever **tightens**: with no active profile the classifier is
//!    never consulted and behaviour is byte-identical to today.

use std::path::{Component, Path, PathBuf};

/// Wire prefix for the per-profile `WorkspaceDescriptor::policy_id`. The suffix
/// is the profile id. Kept private-behind-helpers so the encode/decode pair is
/// the only way this string is produced or parsed.
const PROFILE_POLICY_ID_PREFIX: &str = "openhuman.profile:";
/// Marker returned when the protected target is the shared `profiles/` root
/// rather than one named sibling profile.
pub const PROFILES_ROOT_SENTINEL: &str = "<profiles-root>";

/// Encode a profile id as the `WorkspaceDescriptor::policy_id` the session
/// builder stamps onto a dedicated-workspace descriptor (`openhuman.profile:<id>`).
///
/// Paired with [`profile_id_from_policy_id`]; the two are the sole owners of the
/// wire format so the encode and decode sites can never disagree.
pub fn workspace_policy_id(profile_id: &str) -> String {
    format!("{PROFILE_POLICY_ID_PREFIX}{profile_id}")
}

/// Decode the active profile id from a `WorkspaceDescriptor::policy_id`.
///
/// Returns `Some(id)` only for the `openhuman.profile:<id>` shape
/// [`workspace_policy_id`] produces (and only when `<id>` is non-empty); every
/// other policy_id — the worktree-isolation ids, test ids, or an empty string —
/// yields `None`, so a non-profile session reads as "no active profile" and the
/// tool gates stay on their shared-path behaviour.
pub fn profile_id_from_policy_id(policy_id: &str) -> Option<&str> {
    policy_id
        .strip_prefix(PROFILE_POLICY_ID_PREFIX)
        .filter(|id| !id.is_empty())
}

/// Outcome of classifying a resolved write/command target against the active
/// profile's isolation boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrossProfileDecision {
    /// The target is not inside a *sibling* profile's workspace — permitted (the
    /// active profile's own dir, the shared `action_dir`, or anywhere outside
    /// `<action_dir>/profiles/` entirely).
    Allow,
    /// The target lands inside `<action_dir>/profiles/<other_id>/` with
    /// `other_id != active_profile`, or is the shared profiles root itself —
    /// blocked. Root targets carry [`PROFILES_ROOT_SENTINEL`].
    Block {
        /// The sibling profile whose workspace the target tried to reach.
        other_id: String,
    },
}

/// Classify a resolved target path against the active profile's cross-profile
/// isolation boundary (the hermes `classify_cross_profile_target` analogue).
///
/// `action_dir` is the agent's **broad** action root; sibling profile
/// workspaces live under `<action_dir>/profiles/<id>/`. `active_profile` is the
/// id of the profile the turn runs under. `target` is the write/command target
/// — ideally already resolved to an absolute path, but relative inputs are
/// joined onto `action_dir` first so the classifier is robust to either.
///
/// Returns [`CrossProfileDecision::Block`] iff the canonicalized `target` is
/// the shared `<action_dir>/profiles/` root or is inside
/// `<action_dir>/profiles/<Q>/` for some `Q != active_profile`, otherwise
/// [`CrossProfileDecision::Allow`].
///
/// **Symlink safety.** The comparison is done on canonicalized paths: the
/// profiles root and the target's deepest *existing* ancestor are both resolved
/// through the filesystem, so a symlink inside profile `P` pointing at profile
/// `Q`'s dir still classifies as a cross-profile target. A not-yet-existing
/// target (a fresh write) canonicalizes its nearest existing ancestor and
/// re-appends the missing tail, matching the `validate_parent_path` strategy.
pub fn classify_cross_profile_target(
    action_dir: &Path,
    active_profile: &str,
    target: &Path,
) -> CrossProfileDecision {
    let profiles_root = action_dir.join("profiles");
    // Resolve the profiles root through the filesystem so a symlinked
    // action_dir (macOS `/tmp` -> `/private/tmp`, sandbox bind-mounts) compares
    // against the same canonical prefix the target resolves to.
    let canon_profiles_root = canonicalize_best_effort(&profiles_root);

    let absolute_target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        action_dir.join(target)
    };
    let canon_target = canonicalize_deepest_existing(&absolute_target);

    let Ok(relative) = canon_target.strip_prefix(&canon_profiles_root) else {
        return CrossProfileDecision::Allow;
    };
    if relative.as_os_str().is_empty() {
        return CrossProfileDecision::Block {
            other_id: PROFILES_ROOT_SENTINEL.to_string(),
        };
    }
    // First component under `profiles/` is the owning profile id.
    let Some(Component::Normal(owner)) = relative.components().next() else {
        return CrossProfileDecision::Allow;
    };
    let owner = owner.to_string_lossy();
    if owner == active_profile {
        CrossProfileDecision::Allow
    } else {
        CrossProfileDecision::Block {
            other_id: owner.into_owned(),
        }
    }
}

/// Best-effort scan of a process `command` for a token that targets a sibling
/// profile's workspace, given the command's working directory `cwd` (the active
/// profile's own dir).
///
/// # Guarantee level (read before relying on this)
///
/// This is **best-effort defense-in-depth for model-facing process tools, not a
/// hard boundary.** It is a static, pre-execution token scan — it cannot see
/// what the process will actually do at runtime. Known, deliberate gaps:
///
/// - **Variable / command substitution.** `$HOME`, `${VAR}`, `$(cmd)`, and
///   backtick substitution resolve to paths only at runtime; a token like
///   `$SOME_VAR` is not path-shaped textually and is skipped.
/// - **Paths embedded inside interpreter code.** `python -c 'open("../bob/x","w")'`
///   or any inline script hides the path inside a program string. The tokenizer
///   below splits on quotes/parens/commas/`=` so the *simple* embedded cases are
///   still isolated and classified, but an arbitrary interpreter can construct a
///   path the scanner never sees.
///
/// The **hard** cross-profile boundary for file mutations is
/// [`SecurityPolicy::validate_path`](crate::openhuman::security) at the file-tool
/// call site (every write funnels through it). Process commands do **not** funnel
/// through that gate, so this scan is their only in-Rust backstop — and
/// airtight process confinement (an OS sandbox: cwd_jail / Seatbelt / Landlock
/// restricting the process to its own subtree) is deliberate follow-up work, not
/// provided here. Do not treat a `None` result as proof a command cannot reach a
/// sibling profile.
///
/// # What it does catch
///
/// It splits the command into simple `;` / `&&` / `||` segments, tracks a
/// leading literal `cd <path>` across those segments, then splits each segment
/// on whitespace, redirect/pipe operators, and common
/// shell punctuation (quotes, parens, backtick, `,`, `=`, `{`, `}`, `&`), keeps
/// path-shaped tokens (those containing a path separator or a leading `~`),
/// plus bare operands naming an existing profile when the tracked cwd is the
/// profiles root, resolves each against `cwd`, and classifies it via
/// [`classify_cross_profile_target`]. This reliably catches the realistic
/// non-adversarial escape vectors: an absolute path into `profiles/<Q>`, a
/// `../<Q>/…` traversal, a `--flag=../<Q>/…` form, and simple quoted paths.
/// Returns the first sibling profile id it would write into, or `None` when no
/// scanned token lands in another profile.
pub fn scan_command_for_cross_profile(
    command: &str,
    cwd: &Path,
    action_dir: &Path,
    active_profile: &str,
) -> Option<String> {
    let mut effective_cwd = cwd.to_path_buf();
    for segment in split_command_segments(command) {
        if let Some(other_id) =
            scan_command_segment(segment, &effective_cwd, action_dir, active_profile)
        {
            return Some(other_id);
        }

        let Some(next_cwd) = simple_cd_target(segment, &effective_cwd) else {
            continue;
        };
        if let CrossProfileDecision::Block { other_id } =
            classify_cross_profile_target(action_dir, active_profile, &next_cwd)
        {
            return Some(other_id);
        }
        effective_cwd = next_cwd;
    }
    None
}

/// Split at top-level shell sequencing operators while leaving separators
/// inside simple quotes alone. This is intentionally not a complete shell
/// parser; it only supplies enough ordering for literal `cd` tracking.
fn split_command_segments(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if matches!(byte, b'\'' | b'"' | b'`') {
            if quote == Some(byte) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(byte);
            }
            i += 1;
            continue;
        }
        if quote.is_none()
            && (byte == b';'
                || (i + 1 < bytes.len()
                    && ((byte == b'&' && bytes[i + 1] == b'&')
                        || (byte == b'|' && bytes[i + 1] == b'|'))))
        {
            segments.push(&command[start..i]);
            i += if byte == b';' { 1 } else { 2 };
            start = i;
            continue;
        }
        i += 1;
    }
    segments.push(&command[start..]);
    segments
}

fn scan_command_segment(
    command: &str,
    cwd: &Path,
    action_dir: &Path,
    active_profile: &str,
) -> Option<String> {
    let profiles_root = canonicalize_best_effort(&action_dir.join("profiles"));
    let cwd_is_action_root = canonicalize_best_effort(cwd) == canonicalize_best_effort(action_dir);
    let cwd_is_profiles_root = canonicalize_best_effort(cwd) == profiles_root;
    let mut token_index = 0usize;
    // Split on shell punctuation as well as whitespace/redirects so a path
    // embedded in a quoted string, a `flag=value`, a comma list, or a brace
    // expansion is isolated into its own token. Splitting only ever produces
    // substrings of the original command, so it cannot invent a path that
    // resolves into a sibling — it can only surface one that was genuinely
    // referenced (no new false positives, strictly better coverage).
    for raw in command.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '>' | '<' | '|' | ';' | ',' | '=' | '"' | '\'' | '(' | ')' | '`' | '{' | '}' | '&'
            )
    }) {
        // Residual wrapper chars the split didn't consume at the edges.
        let token = raw.trim_matches(|c| matches!(c, '"' | '\'' | '(' | ')' | '`'));
        if token.is_empty() {
            continue;
        }
        let is_command_word = token_index == 0;
        token_index += 1;
        // Ordinarily only path-shaped tokens can reach another directory. Once
        // a preceding `cd` has moved cwd to `<action_dir>/profiles`, however, a
        // bare operand can name a sibling directly (`rm -rf bob`). Scan such an
        // operand only when it resolves to an existing profile directory; this
        // avoids treating ordinary arguments (`echo hi`) as profile ids.
        let path_shaped =
            token == ".." || token.contains('/') || token.contains('\\') || token.starts_with('~');
        let bare_profile_operand = cwd_is_profiles_root
            && !is_command_word
            && !token.starts_with('-')
            && cwd.join(token).is_dir();
        // From the shared action root, the literal bare operand `profiles`
        // names the protected collection root itself (`rm -rf profiles`,
        // `mv profiles backup`). It has no slash, so classify it explicitly
        // before spawning just as we do bare sibling ids from inside that root.
        let bare_profiles_root_operand =
            cwd_is_action_root && !is_command_word && token == "profiles";
        if !path_shaped && !bare_profile_operand && !bare_profiles_root_operand {
            continue;
        }
        let expanded = crate::openhuman::config::expand_tilde(token);
        let candidate = Path::new(&expanded);
        let absolute = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        if let CrossProfileDecision::Block { other_id } =
            classify_cross_profile_target(action_dir, active_profile, &absolute)
        {
            return Some(other_id);
        }
    }
    None
}

/// Resolve a literal leading `cd` for the next sequenced command. Dynamic
/// targets (`$VAR`, command substitution) remain in the documented best-effort
/// gap. Nonexistent/non-directory targets are ignored because the shell's `cd`
/// would fail and leave cwd unchanged.
fn simple_cd_target(segment: &str, cwd: &Path) -> Option<PathBuf> {
    let mut words = segment.split_whitespace();
    if words.next()? != "cd" {
        return None;
    }
    let mut target = words.next()?;
    if target == "--" {
        target = words.next()?;
    }
    let target = target.trim_matches(|c| matches!(c, '"' | '\''));
    if target.is_empty() || target.contains('$') || target.contains('`') || target.contains("$(") {
        return None;
    }
    let expanded = crate::openhuman::config::expand_tilde(target);
    let path = Path::new(&expanded);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    candidate
        .is_dir()
        .then(|| canonicalize_best_effort(&candidate))
}

/// Canonicalize `path`, falling back to the raw path when it does not exist.
fn canonicalize_best_effort(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Canonicalize `path` when it exists; otherwise canonicalize its deepest
/// existing ancestor and re-append the missing tail. Mirrors
/// `SecurityPolicy::validate_parent_path`'s symlink-safe resolution so a fresh
/// (not-yet-created) write target is still classified against the real
/// filesystem layout.
fn canonicalize_deepest_existing(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    // Walk up to the deepest existing ancestor, collecting the non-existent tail.
    let mut existing = path;
    let mut tail: Vec<Component<'_>> = Vec::new();
    loop {
        if existing.exists() {
            break;
        }
        match (existing.parent(), existing.components().next_back()) {
            (Some(parent), Some(comp)) => {
                tail.push(comp);
                existing = parent;
            }
            _ => break,
        }
    }
    let mut resolved = canonicalize_best_effort(existing);
    for component in tail.into_iter().rev() {
        resolved.push(component);
    }
    resolved
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
