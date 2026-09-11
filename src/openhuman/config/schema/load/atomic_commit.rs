//! The commit half of an atomic `config.toml` write.
//!
//! `Config::save` writes and fsyncs a temporary file, then hands it here to be
//! swapped into place. Splitting the two apart is what makes the commit point
//! nameable: everything before this function can fail with nothing written,
//! and everything after the rename inside it has already taken effect.
//!
//! That distinction is load-bearing for callers, not cosmetic. Several of them
//! roll in-memory state back when `save` returns `Err` — the config-migration
//! runner reverts both `schema_version` and any entries it seeded. A failure
//! reported *after* the replacement had committed would leave the process
//! disagreeing with the file on disk (#6205 review), which is the
//! asymmetric-rollback shape #6143 was about, reached from the other side.

use std::path::Path;

use anyhow::Context;
use tokio::fs;

/// Swap `temp_path` over `config_path`, preserving the replaced config as
/// `.bak` — verbatim unless this save is upgrading its secrets, which
/// [`back_up_previous`] explains.
///
/// Returns `Err` **only while the live config is still the old one**, so a
/// caller may treat an error as "nothing was written" and roll back safely.
pub(super) async fn commit_replacement(
    temp_path: &Path,
    config_path: &Path,
    parent_dir: &Path,
    backup_path: &Path,
) -> anyhow::Result<()> {
    back_up_previous(config_path, backup_path).await;

    // Parallel test/runtime cleanup can remove an otherwise valid config
    // directory after the temporary file is written. Recreate it directly
    // before the rename so the atomic replacement retains its guarantee.
    fs::create_dir_all(parent_dir).await.with_context(|| {
        format!(
            "Failed to recreate config directory before atomic replace: {}",
            parent_dir.display()
        )
    })?;

    let replace = fs::rename(temp_path, config_path).await;
    // A concurrently-cleaning test or runtime can still remove the parent in
    // the narrow window after the create_dir_all above. Retry the rename once
    // after recreating it; the temp file lives alongside the target and remains
    // available across that directory-entry race.
    let replace = match replace {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(parent_dir).await.with_context(|| {
                format!(
                    "Failed to recreate config directory for atomic replace retry: {}",
                    parent_dir.display()
                )
            })?;
            fs::rename(temp_path, config_path).await
        }
        result => result,
    };

    if let Err(e) = replace {
        // Nothing to restore. `fs::rename` is all-or-nothing, so a failure here
        // leaves the live config exactly as it was — already correct. The
        // previous code copied `.bak` back over it, which (with `.bak` holding
        // the *new* bytes) published the very write this function is about to
        // report as failed, on the error path. Discard the temp file and let
        // the untouched original stand.
        let _ = fs::remove_file(temp_path).await;
        anyhow::bail!("Failed to atomically replace config file: {e}");
    }

    // Past the commit point: the rename above has replaced the live config and
    // every reader on this machine already sees the new bytes. The file's own
    // contents were fsynced before it, so all this directory fsync adds is
    // durability of the *rename* across a power loss — a weaker guarantee, not
    // a failed save. Warn rather than return `Err`, so the promise in this
    // function's doc comment holds.
    if let Err(e) = super::sync_directory(parent_dir).await {
        tracing::warn!(
            path = %parent_dir.display(),
            error = %e,
            "[config] config was replaced but its directory entry could not be \
             fsynced; the write is visible now but may not survive a power loss"
        );
    }

    Ok(())
}

/// Preserve the config that is about to be replaced, as `.bak`.
///
/// Sourced from the live config, not from `temp_path`. Backing up the temp file
/// made `.bak` a duplicate of the config being written the moment the rename
/// landed, so the previous one was gone and `load_or_init`'s corruption
/// recovery had nothing older to fall back to — the one job this file has.
///
/// **Byte-for-byte, except when the save is a secret upgrade.** A verbatim copy
/// is what a backup should be: it keeps whatever the previous file held,
/// including keys this build does not model and would drop on a round trip
/// through `Config` — and a rollback or a bad migration is exactly when that
/// matters. So that is the default.
///
/// The exception is narrow and load-bearing. `encrypt_config_secrets` only
/// encrypts values that are not already encrypted, so a save can be an in-place
/// upgrade of the file it is replacing: plaintext to `enc2:`, or the forced
/// `enc:` (XOR) to `enc2:` migration that `load_or_init` triggers a save for
/// with the express purpose of making "the insecure ciphertext stop living on
/// disk (audit C8)". Copying those bytes into a `.bak` that nothing ever cleans
/// would defeat that save exactly, so in that case the *upgraded* form is
/// written instead. This is what `config_secrets_encrypted_on_save_decrypted_on_load`
/// pins, and it is why the backup used to be taken from the temp file — which
/// bought that property by giving the recovery guarantee away.
///
/// The test for "is this an upgrade" is the upgrade itself: re-encrypt a copy
/// of the previous config and see whether anything changed. Nothing changed
/// means the file was already at rest, which is the overwhelmingly common case,
/// and it takes the verbatim path.
///
/// Best-effort throughout. A backup is a convenience for a corrupt-config
/// recovery that may never happen; none of its failure modes justify failing a
/// save that is otherwise fine, and every early return here leaves any existing
/// `.bak` untouched rather than replacing it with something worse.
async fn back_up_previous(config_path: &Path, backup_path: &Path) {
    // No existing config: a first-ever write has nothing to preserve. Read
    // rather than test-then-read, so a concurrent removal is the same path.
    let Ok(existing) = fs::read_to_string(config_path).await else {
        return;
    };
    let Ok(mut previous) = toml::from_str::<crate::openhuman::config::Config>(&existing) else {
        // Unparseable, so it cannot be checked for at-rest secrets and cannot
        // be trusted not to hold one. It is also worthless to recovery, which
        // parses what it finds here.
        tracing::debug!(
            path = %config_path.display(),
            "[config] previous config did not parse; skipping backup"
        );
        return;
    };
    // The parsed copy carries no `config_path` — it is not a serialized field —
    // and `encrypt_config_secrets` resolves its key material relative to one.
    previous.config_path = config_path.to_path_buf();

    let mut upgraded = previous.clone();
    if let Err(e) = super::secrets::encrypt_config_secrets(&mut upgraded) {
        tracing::debug!(
            path = %config_path.display(),
            error = %e,
            "[config] could not check the previous config for at-rest secrets; skipping backup"
        );
        return;
    }
    let (Ok(as_found), Ok(as_upgraded)) = (
        toml::to_string_pretty(&previous),
        toml::to_string_pretty(&upgraded),
    ) else {
        return;
    };

    if as_found == as_upgraded {
        // Already at rest: nothing this save would strip, so keep the file
        // exactly as it is, unknown keys and all.
        if fs::copy(config_path, backup_path).await.is_err() {
            return;
        }
    } else if fs::write(backup_path, as_upgraded).await.is_err() {
        return;
    }
    harden_backup(backup_path).await;
}

/// Restrict a freshly written `.bak` to `0600`.
///
/// `fs::copy` carries the source file's mode and `fs::write` creates at
/// `0o666 & ~umask` (0644 under the usual 022) — and a config predating the
/// load-time hardening can still be `0644` on disk. The backup holds the same
/// `enc2:` provider keys and channel tokens as the config itself, which `save`
/// hardens for exactly that reason. Best-effort for the
/// same reason that hardening is: filesystems without chmod (CIFS/SMB, exFAT,
/// some FUSE mounts) must not turn a working save into a hard failure.
async fn harden_backup(backup_path: &Path) {
    #[cfg(unix)]
    {
        use std::{fs::Permissions, os::unix::fs::PermissionsExt};
        if let Err(e) = fs::set_permissions(backup_path, Permissions::from_mode(0o600)).await {
            tracing::warn!(
                path = %backup_path.display(),
                error = %e,
                "[security][config] could not restrict config backup to 0600; \
                 it may be readable by other local users"
            );
        }
    }
    #[cfg(not(unix))]
    let _ = backup_path;
}
