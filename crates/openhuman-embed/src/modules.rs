//! Loadable native modules, as far as a host configures them.

/// Point module installs at artifacts bundled with the host (an app's
/// resources directory) instead of downloading them. Set once, before boot;
/// `Err` returns the path when one was already set.
#[cfg(feature = "modules")]
pub use openhuman_core::modules::ops::set_bundled_releases_dir;

/// Register the in-process factory for a module linked into the host
/// executable. Local-development builds only: `openhuman-modules-linked` calls
/// this for each module it links; no shipped host does.
#[cfg(feature = "modules")]
pub use openhuman_core::modules::linked::{
    is_registered as is_linked_registered, register_linked, LinkedFactory,
};

/// The browser-control module.
#[cfg(feature = "modules")]
pub mod browser {
    /// Its registry id, for a host that checks whether it is installed.
    pub use openhuman_core::modules::browser::MODULE_ID;
}

/// Pre-core calls through the process-wide loader, with explicit runtime
/// configuration: [`client`] builds a [`ModuleClient`] over the same resolver
/// the core uses, so a module loads once per process whoever asks first.
pub use openhuman_core::modules::client;
pub use tinybus::module::{ModuleCallError, ModuleClient};

#[cfg(test)]
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
