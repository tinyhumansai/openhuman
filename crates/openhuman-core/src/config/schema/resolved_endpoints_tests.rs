use super::*;

fn endpoints() -> ResolvedEndpoints {
    ResolvedEndpoints {
        managed_hosts: vec![
            "api.example.test".into(),
            ".example.test".into(),
            "bare-name".into(),
        ],
        ..Default::default()
    }
}

#[test]
fn nothing_is_managed_without_registered_hosts() {
    assert!(!ResolvedEndpoints::default().is_managed_host("https://api.example.test/x"));
}

#[test]
fn exact_and_suffix_patterns_match_the_host_only() {
    let e = endpoints();
    assert!(e.is_managed_host("https://api.example.test/openai/v1"));
    assert!(e.is_managed_host("HTTPS://User@Staging-Api.Example.Test:8443/x"));
    assert!(e.is_managed_host("bare-name"));
    assert!(e.is_managed_host("http://bare-name:7788"));
}

#[test]
fn lookalikes_and_path_mentions_do_not_match() {
    let e = endpoints();
    assert!(!e.is_managed_host("https://evilexample.test"));
    assert!(!e.is_managed_host("https://example.test.evil.invalid"));
    assert!(!e.is_managed_host("https://other.invalid/api.example.test"));
    assert!(!e.is_managed_host(""));
}

#[test]
fn an_at_sign_after_the_query_fragment_or_path_is_not_userinfo() {
    let e = endpoints();
    assert!(!e.is_managed_host("https://evil.invalid?@api.example.test/v1"));
    assert!(!e.is_managed_host("https://evil.invalid#@api.example.test/v1"));
    assert!(!e.is_managed_host("https://evil.invalid\\@api.example.test/v1"));
    assert!(e.is_managed_host("https://api.example.test?next=a@b.invalid"));
}

/// `host_matches_any` is not replaceable by
/// `tinyagents_harness::url_guard::host_matches_allowlist`: the allowlist
/// treats a bare `example.com` as covering its subdomains, never matches a
/// leading-dot pattern, compares case-sensitively and honours `*`. A managed
/// host list depends on the opposite (exact unless `.suffix`), so this pins
/// both behaviours side by side.
#[test]
fn host_matching_differs_from_the_url_guard_allowlist() {
    use tinyagents_harness::url_guard::host_matches_allowlist;
    // (pattern, host, host_matches_any, host_matches_allowlist)
    let cases = [
        (".example.com", "example.com", false, false),
        (".example.com", "sub.example.com", true, false),
        (".example.com", "badexample.com", false, false),
        ("example.com", "example.com", true, true),
        ("example.com", "sub.example.com", false, true),
        ("example.com", "badexample.com", false, false),
        (".example.com", "SUB.Example.COM", true, false),
        ("*", "example.com", false, true),
    ];
    for (pattern, host, ours, allowlist) in cases {
        let patterns = vec![pattern.to_string()];
        assert_eq!(
            host_matches_any(host, &patterns),
            ours,
            "host_matches_any({host:?}, [{pattern:?}])"
        );
        assert_eq!(
            host_matches_allowlist(host, &patterns),
            allowlist,
            "host_matches_allowlist({host:?}, [{pattern:?}])"
        );
    }
}
