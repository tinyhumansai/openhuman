//! Loadable native modules, as far as a host configures them.

/// Point module installs at artifacts bundled with the host (an app's
/// resources directory) instead of downloading them. Set once, before boot;
/// `Err` returns the path when one was already set.
#[cfg(feature = "modules")]
pub use openhuman_core::modules::ops::set_bundled_releases_dir;

/// The browser-control module.
#[cfg(feature = "modules")]
pub mod browser {
    /// Its registry id, for a host that checks whether it is installed.
    pub use openhuman_core::modules::browser::MODULE_ID;
}

/// Pre-core calls through the process-wide loader, with explicit runtime configuration.
pub use openhuman_core::modules::client::{ModuleCallError, ModuleClient};

#[cfg(all(test, not(feature = "modules"), not(feature = "default")))]
#[path = "modules_tests.rs"]
mod tests;
