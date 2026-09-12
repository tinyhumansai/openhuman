//! Pure helpers for the connectivity diag controller.
//!
//! These are intentionally tiny so they can be unit-tested in isolation
//! without spinning up the global `SocketManager`. The RPC handler in
//! `rpc.rs` composes them.

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener};

/// Probe whether a TCP listener can bind to `127.0.0.1:<port>`.
///
/// Returns `true` when the bind fails (i.e. something is already listening)
/// and `false` when the port is free. We probe with a fresh ephemeral
/// listener and immediately drop it — this is the same trick the core
/// uses to detect a takeable stale listener and is cheap (sub-millisecond).
///
/// Used by the diag endpoint to surface "the sidecar believes it's running
/// but its port is bound by some other process" early, before the user hits
/// confusing 401/transport errors.
pub fn is_port_in_use(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    match TcpListener::bind(addr) {
        Ok(listener) => {
            // Bound cleanly — port was free. Drop returns it to the OS.
            drop(listener);
            false
        }
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            // Another listener owns this port — exactly what we're probing for.
            log::trace!("[connectivity][ops] is_port_in_use: port {port} in use");
            true
        }
        Err(err) => {
            // Permission denied, address not available, etc. — not "in use".
            // Return false so callers don't misreport the port as occupied.
            // (addresses @coderabbitai on ops.rs:36)
            log::warn!(
                "[connectivity][ops] is_port_in_use: unexpected bind error port={port}: {err}"
            );
            false
        }
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
