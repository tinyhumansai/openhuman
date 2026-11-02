use super::is_public_bind;

#[test]
fn localhost_variants_are_not_public_binds() {
    for host in ["127.0.0.1", "localhost", "::1", "[::1]", "0:0:0:0:0:0:0:1"] {
        assert!(!is_public_bind(host), "{host}");
    }
}

#[test]
fn non_loopback_hosts_are_public_binds() {
    for host in ["0.0.0.0", "192.168.1.100", "10.0.0.1"] {
        assert!(is_public_bind(host), "{host}");
    }
}
