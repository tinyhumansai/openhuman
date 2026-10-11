//! OS keychain providers installed by desktop, CLI and TUI hosts before boot.
//! Loading the RPC library or serving a SaaS gateway does not install them.

mod backend;
mod master_key;
pub use backend::OsKeyringBackend;
pub use master_key::OsMasterKeySource;

/// Supply an OS master key for encrypted-file storage. The explicit `os`
/// override also installs the native secret backend. Product host boot selects
/// persistent storage separately; installing these sources alone leaves the
/// basic library backend unchanged.
pub fn install() {
    use crate::core_host::security::secret_storage::{
        install_master_key_source, install_secret_backend, installed_master_key_source,
        installed_secret_backend,
    };
    use std::sync::Arc;
    if installed_master_key_source().is_none() {
        install_master_key_source(Arc::new(OsMasterKeySource::default()));
    }
    if installed_secret_backend().is_none()
        && std::env::var("OPENHUMAN_KEYRING_BACKEND")
            .is_ok_and(|v| v.trim().eq_ignore_ascii_case("os"))
    {
        install_secret_backend(Arc::new(OsKeyringBackend::new()));
    }
}
