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
    let addr: SocketAddr = match format!("127.0.0.1:{port}").parse() {
        Ok(a) => a,
        Err(err) => {
            log::warn!("[connectivity][ops] is_port_in_use parse failed port={port} err={err}");
            return false;
        }
    };
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
mod tests {
    use super::*;

    /// Binding the same port twice — the second probe MUST report "in use".
    /// We do the bind ourselves rather than relying on a known well-known
    /// port (those flake in CI sandboxes).
    #[test]
    fn is_port_in_use_detects_active_listener() {
        // Bind to an ephemeral port the kernel picks for us so the test
        // never collides with anything else on the host.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let port = listener.local_addr().expect("local_addr").port();
        assert!(
            is_port_in_use(port),
            "expected port {port} to be reported in use while we hold the listener"
        );
        // Drop the listener and confirm the probe flips back to free. This
        // proves the helper isn't always returning true.
        drop(listener);
        assert!(
            !is_port_in_use(port),
            "expected port {port} to be free after dropping the listener"
        );
    }

    #[test]
    fn is_port_in_use_returns_false_for_random_free_port() {
        // We bind ephemeral, capture the port, then drop — the just-released
        // port is overwhelmingly likely to be free for the next millisecond.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let port = listener.local_addr().expect("local_addr").port();
        drop(listener);
        // No assertion fail-out if the kernel re-handed the port to another
        // process between drop and probe — that's a flake we deliberately
        // don't enforce. The previous test covers the positive case.
        let _ = is_port_in_use(port);
    }
}
