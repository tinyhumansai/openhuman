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

/// The TinyBox sandbox module, available to hosts that compose sandbox and
/// gateway operations before the core RPC server starts.
#[cfg(feature = "modules")]
pub mod tinybox {
    /// The registry id used by the process module loader.
    pub use openhuman_core::modules::tinybox::MODULE_ID;

    /// TinyBox's versioned serialized bus contract.
    pub use openhuman_core::modules::tinybox::contract;

    pub use openhuman_core::modules::tinybox::{
        abort_file_write, analyze_command, analyze_shell, begin_file_read, begin_file_write,
        cancel, capabilities, close, close_forward, create, describe, exec, finish_file_read,
        finish_file_write, forward, inspect, is_running, jail_status, read_file_chunk, read_output,
        release_output, reserve, run_docker, shutdown, spawn, start_exec, start_jailed_exec,
        write_file_chunk,
    };
}
