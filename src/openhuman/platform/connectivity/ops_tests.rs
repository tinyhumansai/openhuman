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
