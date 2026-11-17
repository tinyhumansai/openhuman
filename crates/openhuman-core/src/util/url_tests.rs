use super::*;

#[test]
fn join_url_replaces_any_base_path_with_the_absolute_route() {
    assert_eq!(
        join_url("https://h.example", "/auth/me"),
        "https://h.example/auth/me"
    );
    assert_eq!(
        join_url(
            "https://h.example/openai/v1/chat/completions",
            "/agent-integrations/x"
        ),
        "https://h.example/agent-integrations/x"
    );
    assert_eq!(join_url(" https://h.example/ ", ""), "https://h.example");
    assert_eq!(join_url("not a url", "/x"), "not a url/x");
    assert_eq!(join_url("not a url/", "x"), "not a url/x");
}

#[test]
fn normalize_backend_api_base_url_keeps_only_the_origin() {
    assert_eq!(
        normalize_backend_api_base_url("https://h.example/openai/v1/chat/completions?x=1#f"),
        "https://h.example"
    );
    assert_eq!(
        normalize_backend_api_base_url("h.example/openai/v1"),
        "https://h.example"
    );
    assert_eq!(normalize_backend_api_base_url("   "), "");
    assert_eq!(
        normalize_api_base_url(" https://h.example// "),
        "https://h.example"
    );
}

#[test]
fn loopback_host_matches_only_this_machine() {
    for host in [
        "localhost",
        "LOCALHOST.",
        "api.localhost",
        "127.0.0.1",
        "127.8.9.10",
        "0.0.0.0",
        "::1",
        "[::1]",
        "::",
        "::ffff:127.0.0.1",
    ] {
        assert!(loopback_host(host), "{host} is on this machine");
    }
    for host in [
        "192.168.1.10",
        "10.0.0.1",
        "fd00::1",
        "example.com",
        "localhost.example.com",
        "",
    ] {
        assert!(!loopback_host(host), "{host} is not on this machine");
    }
}

#[cfg(not(feature = "security-module"))]
#[test]
fn a_build_without_the_module_classifies_provider_hosts_by_loopback() {
    assert!(provider_host_is_local("localhost"));
    assert!(!provider_host_is_local("192.168.1.10"));
}

#[cfg(feature = "security-module")]
#[test]
fn security_configured_off_classifies_provider_hosts_by_loopback() {
    let _off = crate::security::switch::off_for_this_thread();
    assert!(provider_host_is_local("127.0.0.1"));
    assert!(!provider_host_is_local("api.example.com"));
}

/// Without an armed TinySecurity module locality is unknown, so even loopback
/// hosts are not local: callers that gate a credential on locality (the
/// hosted Langfuse environment classifier) fail closed.
#[cfg(feature = "security-module")]
#[test]
fn provider_locality_fails_closed_without_tinysecurity() {
    let _absent = crate::modules::security::test_host::absent();
    for host in [
        "[::1]",
        "[0:0:0:0:0:0:0:1]",
        "[::]",
        "api.localhost",
        "0.0.0.0",
        "192.168.1.20",
        "10.0.0.5",
    ] {
        assert!(
            !provider_host_is_local(host),
            "{host} must fail closed without TinySecurity"
        );
    }
}
