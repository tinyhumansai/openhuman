//! Per-profile "home" materialization — hermes-agent-style agent homes.
//!
//! Each profile can own an identity file (`SOUL.md`, re-read on every prompt
//! build), a curated per-profile memory file (`MEMORY.md`), an optional
//! dedicated memory subtree, and an optional agent-writable workspace. The two
//! roots are deliberately split (see [`crate::openhuman::config`]):
//!
//! ```text
//! <workspace>/personalities/<id>/SOUL.md      identity (hot-read each prompt)
//! <workspace>/personalities/<id>/MEMORY.md    curated per-profile memory
//! <action_dir>/profiles/<id>/                 agent-writable workspace
//! ```
//!
//! Identity + memory files live under `workspace_dir` (core-managed reads only —
//! the agent's write tools cannot reach there, enforced fail-closed by
//! `is_workspace_internal_path`). The agent's *writable* working dir lives under
//! `action_dir`, which acting tools (shell/file/git) are allowed to touch.

use std::io;
use std::path::{Path, PathBuf};

use super::types::{AgentProfile, DEFAULT_PROFILE_ID};

/// Directory holding a profile's core-managed identity + memory files:
/// `<workspace>/personalities/<id>/`.
pub fn profile_home(workspace_dir: &Path, profile_id: &str) -> PathBuf {
    workspace_dir.join("personalities").join(profile_id)
}

/// The agent-writable per-profile workspace: `<action_dir>/profiles/<id>/`.
///
/// Because this lives under `action_dir`, the `SecurityPolicy` already permits
/// acting tools to read/write here — no hardening change is needed.
pub fn profile_action_workspace(action_dir: &Path, profile_id: &str) -> PathBuf {
    action_dir.join("profiles").join(profile_id)
}

/// A profile's private skills directory: `<workspace>/personalities/<id>/skills/`.
///
/// SKILL.md / WORKFLOW.md bundles placed here are discovered ONLY for turns
/// running under this profile (see
/// `skills::discover_workflows_with_profile`). Seeded empty by
/// [`ensure_profile_home`].
pub fn profile_skills_dir(workspace_dir: &Path, profile_id: &str) -> PathBuf {
    profile_home(workspace_dir, profile_id).join("skills")
}

/// The profile-local skills discovery root for `profile_id`, iff the id passes
/// [`validate_profile_id`].
///
/// The discovery/list seam (harness catalog build, `list_workflows` /
/// `describe_workflow` / resource-read tools) passes this into
/// `skills::discover_workflows_with_profile` /
/// `skills::load_workflow_metadata_for_profile`. Returns `None` for legacy ids
/// that fail validation — matching [`profile_skills_dir`]'s companion guards on
/// the home/workspace paths, so a home the read paths would never load never
/// contributes skills either.
pub fn profile_skills_root(workspace_dir: &Path, profile_id: &str) -> Option<PathBuf> {
    if let Err(e) = validate_profile_id(profile_id) {
        tracing::debug!(
            profile_id = %profile_id,
            error = %e,
            "[profiles][home] profile_skills_root: id fails validation, no profile-local skills root"
        );
        return None;
    }
    Some(profile_skills_dir(workspace_dir, profile_id))
}

/// Validate a profile id against the hermes-style name grammar
/// `^[a-z0-9][a-z0-9_-]{0,63}$`.
///
/// Only enforced when creating a *new* custom profile — legacy / built-in ids
/// (which may not satisfy the grammar) keep loading, so this is never applied on
/// the read/resolve paths.
pub fn validate_profile_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("profile id must not be empty".to_string());
    }
    if id.len() > 64 {
        return Err(format!("profile id '{id}' is too long (max 64 characters)"));
    }
    let mut chars = id.chars();
    let first = chars.next().expect("non-empty checked above");
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(format!(
            "profile id '{id}' must start with a lowercase letter or digit"
        ));
    }
    for c in id.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-';
        if !ok {
            return Err(format!(
                "profile id '{id}' may only contain lowercase letters, digits, '_' or '-'"
            ));
        }
    }
    Ok(())
}

/// Write `contents` through a temp file in the same directory, then move it to
/// `target`. Unix `rename` atomically replaces an existing target. Windows
/// `rename` cannot overwrite, so rewrites remove the old target immediately
/// before the move; this is the platform-safe remove/replace fallback used by
/// Settings SOUL edits and clear tombstones. First-time seeds stay atomic on
/// every platform, and no reader can observe a partially-written file.
fn seed_file_atomic(dir: &Path, target: &Path, contents: &[u8]) -> io::Result<()> {
    let base = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("seed");
    let tmp = dir.join(format!(".{base}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, contents)?;

    #[cfg(windows)]
    match std::fs::remove_file(target) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(error);
        }
    }

    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Best-effort cleanup so a failed rename doesn't leave a stray temp.
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Render the default seed persona for a profile lacking an inline `soul_md`.
///
/// Kept intentionally short — a real persona is authored by the user by editing
/// the seeded `SOUL.md`. Never contains PII or secrets.
fn default_soul_template(profile: &AgentProfile) -> String {
    let name = if profile.name.trim().is_empty() {
        profile.id.as_str()
    } else {
        profile.name.trim()
    };
    let description = profile.description.trim();
    let mut out = format!("# {name}\n\n");
    if !description.is_empty() {
        out.push_str(description);
        out.push_str("\n\n");
    }
    out.push_str(&format!(
        "You are {name}. Keep your own voice, working style, and memory distinct \
         from other profiles. Edit this file to shape your identity.\n"
    ));
    out
}

/// Materialize a profile's home on disk. Idempotent — existing files are never
/// overwritten, so a user's edited `SOUL.md` / `MEMORY.md` survive re-runs.
///
/// Creates:
/// - `<workspace>/personalities/<id>/` (always),
/// - `SOUL.md` seeded from `profile.soul_md` when non-empty, else a short
///   default persona template (only when the file is absent),
/// - an empty `MEMORY.md` (only when absent),
/// - `<action_dir>/profiles/<id>/` when `profile.dedicated_workspace`.
pub fn ensure_profile_home(
    workspace_dir: &Path,
    action_dir: &Path,
    profile: &AgentProfile,
) -> io::Result<()> {
    // Guard: only materialize a home for ids the read paths will actually load.
    // `resolve_personality_soul`, `dedicated_workspace_dir`, and
    // `effective_memory_suffix` all skip ids that fail `validate_profile_id`, so
    // seeding `personalities/<id>/` for such an id would leave a home nothing
    // ever reads. Early-return (no dir, no seed) to keep write/read symmetric.
    if let Err(e) = validate_profile_id(&profile.id) {
        tracing::warn!(
            profile_id = %profile.id,
            error = %e,
            "[profiles][home] ensure_profile_home skipped: id fails validation, \
             no home materialized (read paths would never load it)"
        );
        return Ok(());
    }

    let home = profile_home(workspace_dir, &profile.id);
    tracing::debug!(
        profile_id = %profile.id,
        home = %home.display(),
        dedicated_memory = profile.dedicated_memory,
        dedicated_workspace = profile.dedicated_workspace,
        "[profiles][home] ensure_profile_home entry"
    );
    std::fs::create_dir_all(&home).map_err(|e| {
        tracing::debug!(
            profile_id = %profile.id,
            home = %home.display(),
            error = %e,
            "[profiles][home] create_dir_all home failed"
        );
        e
    })?;

    let soul_path = home.join("SOUL.md");
    let has_inline_soul = profile
        .soul_md
        .as_ref()
        .is_some_and(|soul| !soul.trim().is_empty());
    if soul_path.exists() {
        tracing::debug!(
            profile_id = %profile.id,
            "[profiles][home] SOUL.md already present, not overwriting"
        );
    } else if profile.id == DEFAULT_PROFILE_ID && !has_inline_soul {
        // Backward compatibility: the built-in Default profile represents the
        // legacy workspace identity. Selecting it must not create a generic
        // profile-local template that shadows the user's root SOUL.md. Once a
        // user authors a Default soul it is seeded normally; an explicit clear
        // writes an existing empty tombstone in `sync_soul_md_on_upsert`.
        tracing::debug!(
            profile_id = %profile.id,
            "[profiles][home] Default has no authored soul; preserving root SOUL.md fallback"
        );
    } else {
        let contents = profile
            .soul_md
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                // Persist inline souls with a trailing newline for tidy files.
                if s.ends_with('\n') {
                    s.to_string()
                } else {
                    format!("{s}\n")
                }
            })
            .unwrap_or_else(|| default_soul_template(profile));
        let seeded_from_inline = has_inline_soul;
        seed_file_atomic(&home, &soul_path, contents.as_bytes()).map_err(|e| {
            tracing::debug!(
                profile_id = %profile.id,
                soul_path = %soul_path.display(),
                error = %e,
                "[profiles][home] seed SOUL.md failed"
            );
            e
        })?;
        tracing::debug!(
            profile_id = %profile.id,
            seeded_from_inline,
            "[profiles][home] SOUL.md seeded"
        );
    }

    let memory_path = home.join("MEMORY.md");
    if memory_path.exists() {
        tracing::debug!(
            profile_id = %profile.id,
            "[profiles][home] MEMORY.md already present, not overwriting"
        );
    } else {
        seed_file_atomic(&home, &memory_path, b"").map_err(|e| {
            tracing::debug!(
                profile_id = %profile.id,
                memory_path = %memory_path.display(),
                error = %e,
                "[profiles][home] create empty MEMORY.md failed"
            );
            e
        })?;
        tracing::debug!(
            profile_id = %profile.id,
            "[profiles][home] MEMORY.md created (empty)"
        );
    }

    // Profile-local skills root: `<workspace>/personalities/<id>/skills/`.
    // Created empty so the user has an obvious place to drop private SKILL.md
    // bundles; discovery surfaces them only for this profile's turns.
    let skills_dir = profile_skills_dir(workspace_dir, &profile.id);
    std::fs::create_dir_all(&skills_dir).map_err(|e| {
        tracing::debug!(
            profile_id = %profile.id,
            skills_dir = %skills_dir.display(),
            error = %e,
            "[profiles][home] create profile skills dir failed"
        );
        e
    })?;
    tracing::debug!(
        profile_id = %profile.id,
        skills_dir = %skills_dir.display(),
        "[profiles][home] profile skills dir ensured"
    );

    if let Some(ws) = dedicated_workspace_dir(action_dir, profile) {
        std::fs::create_dir_all(&ws).map_err(|e| {
            tracing::debug!(
                profile_id = %profile.id,
                workspace = %ws.display(),
                error = %e,
                "[profiles][home] create dedicated workspace failed"
            );
            e
        })?;
        tracing::debug!(
            profile_id = %profile.id,
            workspace = %ws.display(),
            "[profiles][home] dedicated workspace ensured"
        );
    }

    tracing::debug!(profile_id = %profile.id, "[profiles][home] ensure_profile_home ok");
    Ok(())
}

/// Reconcile the on-disk `SOUL.md` with an edited inline `soul_md` on an
/// **explicit profile save** (upsert only — never on select).
///
/// [`ensure_profile_home`] seeds `SOUL.md` only when the file is *absent*, so a
/// persona edited in Settings after the home already exists would update
/// `agent_profiles.json` but leave the file stale — and because
/// [`resolve_personality_soul`](super::paths::resolve_personality_soul) reads
/// the file first, the agent would keep using the old identity. This closes that
/// gap by overwriting the file (atomic temp+rename, same as the seed path) when:
/// - the id passes [`validate_profile_id`] (read paths would load it),
/// - `profile.soul_md` is `Some(non-empty)` and differs from the previously
///   persisted inline value, and
/// - the trimmed inline content differs from the current file content.
///
/// When `soul_md` was already empty/`None`, the file is left untouched so a
/// manual `SOUL.md` remains authoritative. A transition from a previously
/// non-empty inline value to empty replaces the synced file with an empty
/// tombstone, allowing the normal root fallback while preventing a later
/// `ensure_profile_home` call from re-seeding the default template. Only ever
/// called from the upsert path; select must not clobber a manually edited file
/// with a stale inline value. Returns `Ok(true)` when the file was rewritten.
pub fn sync_soul_md_on_upsert(
    workspace_dir: &Path,
    profile: &AgentProfile,
    previous_soul_md: Option<&str>,
) -> io::Result<bool> {
    if let Err(e) = validate_profile_id(&profile.id) {
        tracing::debug!(
            profile_id = %profile.id,
            error = %e,
            "[profiles][home] sync_soul_md_on_upsert skipped: id fails validation"
        );
        return Ok(false);
    }
    let home = profile_home(workspace_dir, &profile.id);
    let soul_path = home.join("SOUL.md");

    // Clearing a previously persisted inline soul is an explicit Settings edit:
    // replace the file that prior inline value created with an empty tombstone.
    // `ensure_profile_home` treats that existing file as materialized, while
    // `resolve_personality_soul` treats it as empty and falls back to root.
    // If inline was already empty, preserve any manual file exactly as before.
    let desired = match profile
        .soul_md
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(s) => s,
        None => {
            let previously_inline = previous_soul_md
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
            if previously_inline {
                std::fs::create_dir_all(&home)?;
                seed_file_atomic(&home, &soul_path, b"")?;
                tracing::debug!(
                    profile_id = %profile.id,
                    "[profiles][home] sync_soul_md_on_upsert: cleared synced SOUL.md with tombstone"
                );
                return Ok(true);
            }
            tracing::debug!(
                profile_id = %profile.id,
                "[profiles][home] sync_soul_md_on_upsert: inline soul_md empty, file left as-is"
            );
            return Ok(false);
        }
    };

    // The editor submits the complete profile on every save. If the inline
    // soul did not change, this is an unrelated settings update and the
    // hot-read file remains authoritative (it may have been edited manually
    // since the profile was loaded).
    if previous_soul_md.map(str::trim) == Some(desired) {
        tracing::debug!(
            profile_id = %profile.id,
            "[profiles][home] sync_soul_md_on_upsert: inline soul_md unchanged, file left as-is"
        );
        return Ok(false);
    }

    // No-op when the file already matches the edited value (compare trimmed so a
    // trailing-newline difference doesn't churn the file).
    if let Ok(current) = std::fs::read_to_string(&soul_path) {
        if current.trim() == desired {
            tracing::debug!(
                profile_id = %profile.id,
                "[profiles][home] sync_soul_md_on_upsert: file already matches inline soul_md"
            );
            return Ok(false);
        }
    }

    // Persist with a trailing newline for tidy files, matching the seed path.
    let contents = format!("{desired}\n");
    std::fs::create_dir_all(&home)?;
    seed_file_atomic(&home, &soul_path, contents.as_bytes()).map_err(|e| {
        tracing::debug!(
            profile_id = %profile.id,
            soul_path = %soul_path.display(),
            error = %e,
            "[profiles][home] sync_soul_md_on_upsert: overwrite SOUL.md failed"
        );
        e
    })?;
    tracing::debug!(
        profile_id = %profile.id,
        "[profiles][home] sync_soul_md_on_upsert: SOUL.md overwritten from edited inline soul_md"
    );
    Ok(true)
}

/// Resolve the agent-writable workspace directory for a profile *iff* it opts
/// into a dedicated workspace and its id passes [`validate_profile_id`].
///
/// Returns `None` for shared-workspace profiles (the common case) and for
/// legacy ids that would fail id validation — in both cases the caller falls
/// back to the shared `action_dir`. This is the seam the session builder uses to
/// derive a per-profile default cwd (section D): a `WorkspaceDescriptor` rooted
/// at this path is threaded into the top-level chat turn.
pub fn dedicated_workspace_dir(action_dir: &Path, profile: &AgentProfile) -> Option<PathBuf> {
    if !profile.dedicated_workspace {
        return None;
    }
    if let Err(e) = validate_profile_id(&profile.id) {
        tracing::warn!(
            profile_id = %profile.id,
            error = %e,
            "[profiles][home] dedicated_workspace requested but id fails validation, \
             falling back to shared action_dir"
        );
        return None;
    }
    Some(profile_action_workspace(action_dir, &profile.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_profile(id: &str) -> AgentProfile {
        let mut p = crate::openhuman::agent::profiles::store::built_in_default_profile();
        p.id = id.to_string();
        p.name = id.to_string();
        p.built_in = false;
        p.is_master = false;
        p.memory_dir_suffix = None;
        p.soul_md = None;
        p.dedicated_memory = false;
        p.dedicated_workspace = false;
        p
    }

    #[test]
    fn profile_home_and_action_workspace_paths() {
        let ws = Path::new("/tmp/ws");
        let action = Path::new("/tmp/act");
        assert_eq!(
            profile_home(ws, "alice"),
            Path::new("/tmp/ws/personalities/alice")
        );
        assert_eq!(
            profile_action_workspace(action, "alice"),
            Path::new("/tmp/act/profiles/alice")
        );
    }

    #[test]
    fn validate_profile_id_matrix() {
        // Valid.
        let max_len = "x".repeat(64);
        for id in [
            "a",
            "a1",
            "alice",
            "alice-bob",
            "alice_bob",
            "0",
            max_len.as_str(),
        ] {
            assert!(validate_profile_id(id).is_ok(), "expected ok: {id}");
        }
        // Invalid.
        for id in [
            "",          // empty
            "-alice",    // leading dash
            "_alice",    // leading underscore
            "Alice",     // uppercase
            "alice bob", // space
            "alice.bob", // dot
            "alice/bob", // slash
            "über",      // non-ascii
        ] {
            assert!(validate_profile_id(id).is_err(), "expected err: {id}");
        }
        // Too long (65).
        assert!(validate_profile_id(&"a".repeat(65)).is_err());
    }

    #[test]
    fn ensure_profile_home_creates_and_seeds() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        let mut profile = test_profile("alice");
        profile.description = "A tidy writer.".to_string();

        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");

        let home = profile_home(ws.path(), "alice");
        assert!(home.is_dir());
        let soul = std::fs::read_to_string(home.join("SOUL.md")).unwrap();
        // Default template used (no inline soul_md).
        assert!(soul.contains("alice"));
        assert!(soul.contains("A tidy writer."));
        assert!(home.join("MEMORY.md").exists());
        // No dedicated workspace requested.
        assert!(!profile_action_workspace(action.path(), "alice").exists());
    }

    #[test]
    fn ensure_profile_home_seeds_soul_from_inline() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        let mut profile = test_profile("bob");
        profile.soul_md = Some("I am Bob, terse and exact.".to_string());

        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
        let soul = std::fs::read_to_string(profile_home(ws.path(), "bob").join("SOUL.md")).unwrap();
        assert_eq!(soul, "I am Bob, terse and exact.\n");
    }

    #[test]
    fn ensure_default_profile_without_authored_soul_preserves_root_fallback() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        std::fs::write(ws.path().join("SOUL.md"), "Established root identity").unwrap();
        let mut profile = test_profile(DEFAULT_PROFILE_ID);
        profile.built_in = true;
        profile.soul_md = None;

        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");

        assert!(profile_home(ws.path(), DEFAULT_PROFILE_ID)
            .join("MEMORY.md")
            .exists());
        assert!(!profile_home(ws.path(), DEFAULT_PROFILE_ID)
            .join("SOUL.md")
            .exists());
        assert_eq!(
            super::super::paths::resolve_personality_soul(ws.path(), &profile),
            None,
            "no profile override leaves prompt construction on the root SOUL.md"
        );
    }

    #[test]
    fn ensure_profile_home_is_idempotent_and_preserves_edits() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        let profile = test_profile("carol");

        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure 1");
        let home = profile_home(ws.path(), "carol");
        // User edits both files.
        std::fs::write(home.join("SOUL.md"), "EDITED SOUL").unwrap();
        std::fs::write(home.join("MEMORY.md"), "EDITED MEMORY").unwrap();

        // Second run must not clobber.
        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure 2");
        assert_eq!(
            std::fs::read_to_string(home.join("SOUL.md")).unwrap(),
            "EDITED SOUL"
        );
        assert_eq!(
            std::fs::read_to_string(home.join("MEMORY.md")).unwrap(),
            "EDITED MEMORY"
        );
    }

    #[test]
    fn ensure_profile_home_creates_empty_skills_dir() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        let profile = test_profile("frank");

        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");

        let skills = profile_skills_dir(ws.path(), "frank");
        assert!(skills.is_dir(), "profile skills dir must be created");
        // Empty — the user drops private SKILL.md bundles here.
        assert_eq!(std::fs::read_dir(&skills).unwrap().count(), 0);
    }

    #[test]
    fn profile_skills_dir_and_root_paths() {
        let ws = Path::new("/tmp/ws");
        assert_eq!(
            profile_skills_dir(ws, "alice"),
            Path::new("/tmp/ws/personalities/alice/skills")
        );
        // Valid id → Some(root); invalid id → None (read paths never load it).
        assert_eq!(
            profile_skills_root(ws, "alice"),
            Some(Path::new("/tmp/ws/personalities/alice/skills").to_path_buf())
        );
        assert_eq!(profile_skills_root(ws, "Bad Id"), None);
    }

    #[test]
    fn ensure_profile_home_creates_dedicated_workspace_when_opted_in() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        let mut profile = test_profile("dave");
        profile.dedicated_workspace = true;

        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
        assert!(profile_action_workspace(action.path(), "dave").is_dir());
    }

    #[test]
    fn ensure_profile_home_skips_invalid_id() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        // An id that fails validate_profile_id must not materialize a home — the
        // read paths would never load it, so a seeded dir would be dead weight.
        let mut profile = test_profile("placeholder");
        profile.id = "Bad Id".to_string();
        profile.dedicated_workspace = true;

        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");

        assert!(
            !profile_home(ws.path(), "Bad Id").exists(),
            "no home dir should be materialized for an invalid id"
        );
        assert!(
            !profile_action_workspace(action.path(), "Bad Id").exists(),
            "no dedicated workspace should be materialized for an invalid id"
        );
        // The `personalities/` root itself must not be created for it either.
        assert!(!ws.path().join("personalities").join("Bad Id").exists());
    }

    #[test]
    fn sync_soul_md_on_upsert_overwrites_edited_inline_soul() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        let mut profile = test_profile("grace");
        profile.soul_md = Some("Original identity.".to_string());
        // Seed the home once (writes SOUL.md from the original inline value).
        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
        let soul_path = profile_home(ws.path(), "grace").join("SOUL.md");
        assert_eq!(
            std::fs::read_to_string(&soul_path).unwrap(),
            "Original identity.\n"
        );

        // User edits the persona in Settings → the stored inline value changes.
        profile.soul_md = Some("Rewritten identity from Settings.".to_string());
        let rewritten =
            sync_soul_md_on_upsert(ws.path(), &profile, Some("Original identity.")).expect("sync");
        assert!(
            rewritten,
            "differing inline soul_md must overwrite the file"
        );
        assert_eq!(
            std::fs::read_to_string(&soul_path).unwrap(),
            "Rewritten identity from Settings.\n"
        );

        // Idempotent: a second sync with the same value is a no-op.
        let again = sync_soul_md_on_upsert(
            ws.path(),
            &profile,
            Some("Rewritten identity from Settings."),
        )
        .expect("sync 2");
        assert!(!again, "matching inline soul_md must not rewrite the file");
    }

    #[test]
    fn sync_soul_md_on_upsert_leaves_file_when_inline_empty() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        // Seed with a default template (no inline soul_md).
        let mut profile = test_profile("heidi");
        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
        let soul_path = profile_home(ws.path(), "heidi").join("SOUL.md");
        // User edits the file manually; inline soul_md stays empty/None.
        std::fs::write(&soul_path, "MANUALLY EDITED SOUL").unwrap();

        profile.soul_md = None;
        let none_written = sync_soul_md_on_upsert(ws.path(), &profile, None).expect("sync none");
        assert!(!none_written);
        profile.soul_md = Some("   ".to_string()); // whitespace-only → treated as empty
        let blank_written = sync_soul_md_on_upsert(ws.path(), &profile, None).expect("sync blank");
        assert!(!blank_written);

        // The manual edit stays authoritative.
        assert_eq!(
            std::fs::read_to_string(&soul_path).unwrap(),
            "MANUALLY EDITED SOUL"
        );
    }

    #[test]
    fn sync_soul_md_on_upsert_persists_empty_tombstone_when_inline_is_cleared() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        let mut profile = test_profile("judy");
        profile.soul_md = Some("Settings identity".to_string());
        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
        let soul_path = profile_home(ws.path(), "judy").join("SOUL.md");
        assert!(soul_path.exists());

        profile.soul_md = None;
        assert!(
            sync_soul_md_on_upsert(ws.path(), &profile, Some("Settings identity"))
                .expect("clear synced soul")
        );
        assert_eq!(std::fs::read_to_string(&soul_path).unwrap(), "");

        // Selecting/materializing the profile again must not resurrect the
        // default profile template over the explicit clear.
        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure after clear");
        assert_eq!(std::fs::read_to_string(&soul_path).unwrap(), "");
    }

    #[test]
    fn sync_soul_md_on_upsert_skips_invalid_id() {
        let ws = TempDir::new().unwrap();
        let mut profile = test_profile("placeholder");
        profile.id = "Bad Id".to_string();
        profile.soul_md = Some("ignored".to_string());
        assert!(!sync_soul_md_on_upsert(ws.path(), &profile, None).expect("sync"));
        assert!(!profile_home(ws.path(), "Bad Id").join("SOUL.md").exists());
    }

    #[test]
    fn sync_soul_md_on_upsert_preserves_manual_file_when_inline_unchanged() {
        let ws = TempDir::new().unwrap();
        let action = TempDir::new().unwrap();
        let mut profile = test_profile("ivy");
        profile.soul_md = Some("Stored identity.".to_string());
        ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
        let soul_path = profile_home(ws.path(), "ivy").join("SOUL.md");
        std::fs::write(&soul_path, "MANUALLY EDITED IDENTITY\n").unwrap();

        let rewritten = sync_soul_md_on_upsert(ws.path(), &profile, Some("Stored identity."))
            .expect("sync unchanged");

        assert!(!rewritten);
        assert_eq!(
            std::fs::read_to_string(soul_path).unwrap(),
            "MANUALLY EDITED IDENTITY\n"
        );
    }

    #[test]
    fn dedicated_workspace_dir_gates_on_flag_and_id() {
        let action = Path::new("/tmp/act");
        let mut shared = test_profile("eve");
        assert_eq!(dedicated_workspace_dir(action, &shared), None);

        shared.dedicated_workspace = true;
        assert_eq!(
            dedicated_workspace_dir(action, &shared),
            Some(Path::new("/tmp/act/profiles/eve").to_path_buf())
        );

        // Legacy invalid id + dedicated_workspace → None (falls back to shared).
        let mut legacy = test_profile("Legacy Id");
        legacy.id = "Legacy Id".to_string();
        legacy.dedicated_workspace = true;
        assert_eq!(dedicated_workspace_dir(action, &legacy), None);
    }
}
