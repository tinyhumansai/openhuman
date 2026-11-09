use anyhow::Result;
use std::path::Path;
use tokio::fs;

use super::super::Config;
use super::branches::default_config_boxed;
use super::{ConfigLoadFallback, ConfigParseError};

pub(crate) async fn parse_config_with_recovery(
    config_path: &Path,
    contents: &str,
) -> (Config, bool) {
    let (config, recovered) = Box::pin(parse_config_boxed(config_path, contents)).await;
    (*config, recovered)
}

/// [`parse_config_with_recovery`] with the config kept behind a `Box`, so the
/// async chain that threads it through (load, recover, migrate, save) moves a
/// pointer instead of an ~8 KB value per hop.
pub(super) async fn parse_config_boxed(config_path: &Path, contents: &str) -> (Box<Config>, bool) {
    let parse_err = match parse_toml_off_worker(contents.to_string()).await {
        Ok(mut config) => {
            config.load_diagnostics_mut().warn_unknown_keys_once();
            tracing::debug!(
                path = %config_path.display(),
                "[config] Config parsed successfully"
            );
            return (config, false);
        }
        Err(parse_err) => parse_err,
    };
    let parse_error = ConfigParseError::from_error(config_path.to_path_buf(), &parse_err);

    let backup_path = config_path.with_extension("toml.bak");
    if tokio::fs::try_exists(&backup_path).await.unwrap_or(false) {
        tracing::warn!(
            path = %config_path.display(),
            backup = %backup_path.display(),
            error = %parse_error.safe_summary(),
            "[config] Config file is corrupted — attempting recovery from backup"
        );
        match fs::read_to_string(&backup_path).await {
            Ok(bak_contents) => match parse_toml_off_worker(bak_contents).await {
                Ok(bak_config) => {
                    tracing::info!(
                        path = %config_path.display(),
                        backup = %backup_path.display(),
                        "[config] Recovered config from backup"
                    );
                    let mut config = *bak_config;
                    let diagnostics = config.load_diagnostics_mut();
                    diagnostics.parse_error = Some(parse_error);
                    diagnostics.fallback = ConfigLoadFallback::Backup;
                    diagnostics.warn_unknown_keys_once();
                    return (Box::new(config), true);
                }
                Err(bak_err) => {
                    tracing::warn!(
                        path = %config_path.display(),
                        backup = %backup_path.display(),
                        error = %ConfigParseError::from_error(backup_path.clone(), &bak_err)
                            .safe_summary(),
                        "[config] Backup is also corrupted; resetting to defaults"
                    );
                }
            },
            Err(read_err) => {
                tracing::warn!(
                    path = %config_path.display(),
                    backup = %backup_path.display(),
                    error = %read_err,
                    "[config] Failed to read backup; resetting to defaults"
                );
            }
        }
    } else {
        tracing::warn!(
            path = %config_path.display(),
            error = %parse_error.safe_summary(),
            "[config] Config file is corrupted (no backup found); resetting to defaults"
        );
    }

    let mut config = default_config_boxed();
    let diagnostics = config.load_diagnostics_mut();
    diagnostics.parse_error = Some(parse_error);
    diagnostics.fallback = ConfigLoadFallback::Defaults;
    (config, true)
}

/// Return the source text that should be inspected by migrations. A readable
/// backup may be the source used to build the config when the primary TOML is
/// malformed.
pub(super) async fn migration_source(config_path: &Path, contents: &str) -> String {
    if parse_toml_off_worker(contents.to_owned()).await.is_ok() {
        return contents.to_owned();
    }
    let backup_path = config_path.with_extension("toml.bak");
    match fs::read_to_string(backup_path).await {
        Ok(backup) if parse_toml_off_worker(backup.clone()).await.is_ok() => backup,
        _ => contents.to_owned(),
    }
}

pub(super) async fn parse_toml_off_worker(contents: String) -> Result<Box<Config>> {
    match tokio::task::spawn_blocking(move || {
        super::parse::config_from_toml_str(&contents).map(Box::new)
    })
    .await
    {
        Ok(Ok(config)) => Ok(config),
        Ok(Err(parse_error)) => Err(parse_error),
        Err(join_error) => {
            Err(anyhow::Error::new(join_error).context("blocking-pool config parser failed"))
        }
    }
}

/// Marker appended to a config-read failure when the file is owned by a
/// different uid than the process trying to read it.
///
/// `core::observability::expected_error_kind` keys on this to keep the failure
/// paging instead of demoting it as an unpreventable user-environment denial:
/// a uid mismatch on a file *we* created with mode 0600 is an OpenHuman defect
/// (typically a container whose entrypoint chowned only the workspace
/// directory, leaving a stale-uid `config.toml` inside it), not an ACL the user
/// has to fix for us.
pub(crate) const CONFIG_OWNER_MISMATCH_MARKER: &str = "[config owner mismatch]";

/// Numeric ownership/permission facts about the config file, appended to the
/// read-failure context.
///
/// An `EACCES` on a file whose `exists()` check just succeeded is fully
/// explained by four numbers we can always obtain: the file's uid/gid, its
/// mode, and the process's effective uid/gid. Without them the operator sees
/// only "Permission denied (os error 13)" and cannot tell a foreign-owner
/// container volume (our bug) from a genuine ACL / antivirus denial (theirs).
///
/// Numeric ids and mode bits are not PII, so this is safe both in the local log
/// and in the error chain that reaches the client.
#[cfg(unix)]
pub(super) async fn describe_config_ownership(path: &Path) -> String {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let Ok(meta) = fs::metadata(path).await else {
        return String::new();
    };
    // SAFETY: `geteuid`/`getegid` take no arguments, mutate no process state,
    // and are documented as always succeeding.
    let (euid, egid) = unsafe { (libc::geteuid(), libc::getegid()) };
    let mismatch = if meta.uid() == euid {
        String::new()
    } else {
        format!("{CONFIG_OWNER_MISMATCH_MARKER} ")
    };
    format!(
        " {mismatch}(file uid={} gid={} mode={:04o}; process euid={euid} egid={egid})",
        meta.uid(),
        meta.gid(),
        meta.permissions().mode() & 0o777,
    )
}

#[cfg(not(unix))]
pub(super) async fn describe_config_ownership(_path: &Path) -> String {
    String::new()
}
