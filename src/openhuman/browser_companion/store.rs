//! Persistence for the Browser Companion domain.
//!
//! Thin by design for increment 1: the only durable state owned outright by
//! this domain is the pairing secret file. Everything else
//! (`enabled` / `port` / `extension_id`) rides the existing `Config`
//! persistence path (`[browser_companion]` in `config.toml`) — this domain
//! reads the live `Config` passed in by its callers rather than building a
//! bespoke store for it.

use std::path::PathBuf;

use tinyflows::companion::SecretStore;

use crate::openhuman::browser_companion::LOG_PREFIX;
use crate::openhuman::config::Config;

/// Resolves the pairing-secret file path:
/// `{workspace_dir}/browser_companion/relay.secret`, creating the parent
/// directory if needed.
///
/// The secret itself is never logged; only paths and outcomes are.
pub(crate) fn resolve_secret_store(config: &Config) -> std::io::Result<SecretStore> {
    let dir = secret_dir(config);
    log::debug!("{LOG_PREFIX} resolving secret store dir={}", dir.display());
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("relay.secret");
    Ok(SecretStore::new(path))
}

/// The directory the pairing secret lives in, without touching the
/// filesystem — split out so tests can assert on the path shape without a
/// real workspace dir.
pub(crate) fn secret_dir(config: &Config) -> PathBuf {
    config.workspace_dir.join("browser_companion")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(workspace_dir: PathBuf) -> Config {
        Config {
            workspace_dir,
            ..Config::default()
        }
    }

    #[test]
    fn secret_dir_is_scoped_under_workspace() {
        let config = test_config(PathBuf::from("/tmp/does-not-exist-oh-test-workspace"));
        let dir = secret_dir(&config);
        assert_eq!(
            dir,
            PathBuf::from("/tmp/does-not-exist-oh-test-workspace/browser_companion")
        );
    }

    #[test]
    fn resolve_secret_store_creates_dir_and_scoped_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = test_config(tmp.path().to_path_buf());

        let store = resolve_secret_store(&config).expect("resolve secret store");
        assert_eq!(store.path(), secret_dir(&config).join("relay.secret"));
        assert!(secret_dir(&config).is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn load_or_create_persists_secret_with_owner_only_perms() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().expect("tempdir");
        let config = test_config(tmp.path().to_path_buf());
        let store = resolve_secret_store(&config).expect("resolve secret store");

        let secret = store.load_or_create().expect("load_or_create");
        assert!(!secret.expose().is_empty());

        let metadata = std::fs::metadata(store.path()).expect("secret file metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "pairing secret file must be owner-only (0600)");

        // Loading again returns the same secret rather than regenerating it.
        let reloaded = store.load_or_create().expect("reload secret");
        assert_eq!(secret.expose(), reloaded.expose());
    }
}
