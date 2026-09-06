use super::*;

#[test]
fn normalize_domain_strips_scheme_path_and_case() {
    let got = normalize_domain("  HTTPS://Docs.Example.com/path ").unwrap();
    assert_eq!(got, "docs.example.com");
}

#[test]
fn normalize_allowed_domains_deduplicates() {
    let got = normalize_allowed_domains(vec![
        "example.com".into(),
        "EXAMPLE.COM".into(),
        "https://example.com/".into(),
    ]);
    assert_eq!(got, vec!["example.com".to_string()]);
}

#[test]
fn validate_accepts_exact_domain() {
    let allow = vec!["example.com".to_string()];
    let got = validate_url("https://example.com/docs", &allow).unwrap();
    assert_eq!(got, "https://example.com/docs");
}

#[test]
fn validate_accepts_http() {
    let allow = vec!["example.com".to_string()];
    assert!(validate_url("http://example.com", &allow).is_ok());
}

#[test]
fn validate_accepts_subdomain() {
    let allow = vec!["example.com".to_string()];
    assert!(validate_url("https://api.example.com/v1", &allow).is_ok());
}

#[test]
fn validate_rejects_allowlist_miss() {
    let allow = vec!["example.com".to_string()];
    let err = validate_url("https://google.com", &allow)
        .unwrap_err()
        .to_string();
    assert!(err.contains("allowed websites"));
}

#[test]
fn validate_wildcard_allows_any_public_host() {
    let allow = vec!["*".to_string()];
    assert!(validate_url("https://example.com/docs", &allow).is_ok());
    assert!(validate_url("https://www.cnbc.com/markets", &allow).is_ok());
    assert!(validate_url("https://sub.deep.example.org", &allow).is_ok());
}

#[test]
fn validate_wildcard_still_blocks_local_and_private() {
    // "Allow all sites" must NOT defeat the SSRF guard.
    let allow = vec!["*".to_string()];
    assert!(validate_url("https://localhost:8080", &allow)
        .unwrap_err()
        .to_string()
        .contains("local/private"));
    assert!(validate_url("https://192.168.1.5", &allow)
        .unwrap_err()
        .to_string()
        .contains("local/private"));
}

#[test]
fn validate_rejects_localhost() {
    let allow = vec!["localhost".to_string()];
    let err = validate_url("https://localhost:8080", &allow)
        .unwrap_err()
        .to_string();
    assert!(err.contains("local/private"));
}

#[test]
fn validate_rejects_private_ipv4() {
    let allow = vec!["192.168.1.5".to_string()];
    let err = validate_url("https://192.168.1.5", &allow)
        .unwrap_err()
        .to_string();
    assert!(err.contains("local/private"));
}

#[test]
fn validate_rejects_whitespace() {
    let allow = vec!["example.com".to_string()];
    let err = validate_url("https://example.com/hello world", &allow)
        .unwrap_err()
        .to_string();
    assert!(err.contains("whitespace"));
}

#[test]
fn validate_rejects_userinfo() {
    let allow = vec!["example.com".to_string()];
    let err = validate_url("https://user@example.com", &allow)
        .unwrap_err()
        .to_string();
    assert!(err.contains("userinfo"));
}

// Empty allowed_domains = open mode: any public host is permitted.
// This keeps web-fetch working when no domain list is configured and
// makes behaviour consistent between default and external-LLM routing.
// (#2700)
#[test]
fn validate_empty_allowlist_allows_public_host() {
    assert!(validate_url("https://example.com", &[]).is_ok());
    assert!(validate_url("https://www.cnbc.com/markets", &[]).is_ok());
}

#[test]
fn validate_empty_allowlist_still_blocks_private_hosts() {
    let err = validate_url("https://192.168.1.5", &[])
        .unwrap_err()
        .to_string();
    assert!(err.contains("local/private"));

    let err = validate_url("https://localhost", &[])
        .unwrap_err()
        .to_string();
    assert!(err.contains("local/private"));
}

// ── normalize_allowed_domains: fail-closed on malformed-only input ──

#[test]
fn normalize_all_invalid_entries_stays_fail_closed() {
    // A non-empty list that fully normalizes to nothing must NOT produce
    // an empty slice (which would silently enter open mode). (#2738)
    let got = normalize_allowed_domains(vec!["   ".into(), "https://".into()]);
    assert!(
        !got.is_empty(),
        "normalized result must be non-empty to stay in strict mode"
    );
    // The sentinel must not match any real public host.
    assert!(
        !host_matches_allowlist("example.com", &got),
        "sentinel must not grant access to real hosts"
    );
    assert!(
        !host_matches_allowlist("api.example.com", &got),
        "sentinel must not grant access to subdomains"
    );
}

#[test]
fn normalize_empty_input_stays_empty_for_open_mode() {
    // Explicitly empty input should return empty (open mode is intentional).
    assert!(normalize_allowed_domains(vec![]).is_empty());
}

#[tokio::test]
async fn dns_check_with_empty_allowlist_allows_public_resolved_host() {
    // Open mode (empty allowlist) must still pass DNS check for public IPs.
    let got = validate_url_with_dns_check_with_resolver(
        "https://example.com",
        &[],
        |host, port| async move {
            assert_eq!(host, "example.com");
            assert_eq!(port, 443);
            Ok(vec!["93.184.216.34".parse().unwrap()])
        },
    )
    .await
    .unwrap();
    assert_eq!(got, "https://example.com");
}

#[tokio::test]
async fn dns_check_with_empty_allowlist_blocks_private_resolved_ip() {
    // Even in open mode, DNS rebinding to a private IP must be blocked.
    let err = validate_url_with_dns_check_with_resolver("https://example.com", &[], |_, _| async {
        Ok(vec!["10.0.0.1".parse().unwrap()])
    })
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("DNS rebinding blocked"));
}

#[test]
fn validate_rejects_ftp_scheme() {
    let allow = vec!["example.com".to_string()];
    let err = validate_url("ftp://example.com", &allow)
        .unwrap_err()
        .to_string();
    assert!(err.contains("http://") || err.contains("https://"));
}

#[test]
fn validate_rejects_empty_url() {
    let allow = vec!["example.com".to_string()];
    let err = validate_url("", &allow).unwrap_err().to_string();
    assert!(err.contains("empty"));
}

#[test]
fn validate_rejects_ipv6_host() {
    let allow = vec!["example.com".to_string()];
    let err = validate_url("http://[::1]:8080/path", &allow)
        .unwrap_err()
        .to_string();
    assert!(err.contains("IPv6"));
}

#[test]
fn blocks_multicast_ipv4() {
    assert!(is_private_or_local_host("224.0.0.1"));
    assert!(is_private_or_local_host("239.255.255.255"));
}

#[test]
fn blocks_broadcast() {
    assert!(is_private_or_local_host("255.255.255.255"));
}

#[test]
fn blocks_reserved_ipv4() {
    assert!(is_private_or_local_host("240.0.0.1"));
    assert!(is_private_or_local_host("250.1.2.3"));
}

#[test]
fn blocks_documentation_ranges() {
    assert!(is_private_or_local_host("192.0.2.1"));
    assert!(is_private_or_local_host("198.51.100.1"));
    assert!(is_private_or_local_host("203.0.113.1"));
}

#[test]
fn blocks_benchmarking_range() {
    assert!(is_private_or_local_host("198.18.0.1"));
    assert!(is_private_or_local_host("198.19.255.255"));
}

#[test]
fn blocks_ipv6_localhost() {
    assert!(is_private_or_local_host("::1"));
    assert!(is_private_or_local_host("[::1]"));
}

#[test]
fn blocks_ipv6_multicast() {
    assert!(is_private_or_local_host("ff02::1"));
}

#[test]
fn blocks_ipv6_link_local() {
    assert!(is_private_or_local_host("fe80::1"));
}

#[test]
fn blocks_ipv6_unique_local() {
    assert!(is_private_or_local_host("fd00::1"));
}

#[test]
fn blocks_ipv4_mapped_ipv6() {
    assert!(is_private_or_local_host("::ffff:127.0.0.1"));
    assert!(is_private_or_local_host("::ffff:192.168.1.1"));
    assert!(is_private_or_local_host("::ffff:10.0.0.1"));
}

#[test]
fn allows_public_ipv4() {
    assert!(!is_private_or_local_host("8.8.8.8"));
    assert!(!is_private_or_local_host("1.1.1.1"));
    assert!(!is_private_or_local_host("93.184.216.34"));
}

#[test]
fn blocks_ipv6_documentation_range() {
    assert!(is_private_or_local_host("2001:db8::1"));
}

#[test]
fn allows_public_ipv6() {
    assert!(!is_private_or_local_host("2607:f8b0:4004:800::200e"));
}

#[test]
fn blocks_shared_address_space() {
    assert!(is_private_or_local_host("100.64.0.1"));
    assert!(is_private_or_local_host("100.127.255.255"));
    assert!(!is_private_or_local_host("100.63.0.1"));
    assert!(!is_private_or_local_host("100.128.0.1"));
}

#[test]
fn ssrf_blocks_loopback_127_range() {
    assert!(is_private_or_local_host("127.0.0.1"));
    assert!(is_private_or_local_host("127.0.0.2"));
    assert!(is_private_or_local_host("127.255.255.255"));
}

#[test]
fn ssrf_blocks_rfc1918_10_range() {
    assert!(is_private_or_local_host("10.0.0.1"));
    assert!(is_private_or_local_host("10.255.255.255"));
}

#[test]
fn ssrf_blocks_rfc1918_172_range() {
    assert!(is_private_or_local_host("172.16.0.1"));
    assert!(is_private_or_local_host("172.31.255.255"));
}

#[test]
fn ssrf_blocks_unspecified_address() {
    assert!(is_private_or_local_host("0.0.0.0"));
}

#[test]
fn ssrf_blocks_dot_localhost_subdomain() {
    assert!(is_private_or_local_host("evil.localhost"));
    assert!(is_private_or_local_host("a.b.localhost"));
}

#[test]
fn ssrf_blocks_dot_local_tld() {
    assert!(is_private_or_local_host("service.local"));
}

#[test]
fn ssrf_ipv6_unspecified() {
    assert!(is_private_or_local_host("::"));
}

// ── Defense-in-depth: alternate IP notations rejected by allowlist
//
// Rust's IpAddr::parse() rejects octal, hex, decimal, and
// zero-padded notations. They fall through as hostnames and get
// rejected by the allowlist instead. These tests pin that
// behaviour so a parser change can't silently re-open SSRF.

#[test]
fn ssrf_octal_loopback_not_parsed_as_ip() {
    assert!(!is_private_or_local_host("0177.0.0.1"));
}

#[test]
fn ssrf_hex_loopback_not_parsed_as_ip() {
    assert!(!is_private_or_local_host("0x7f000001"));
}

#[test]
fn ssrf_decimal_loopback_not_parsed_as_ip() {
    assert!(!is_private_or_local_host("2130706433"));
}

#[test]
fn ssrf_zero_padded_loopback_not_parsed_as_ip() {
    assert!(!is_private_or_local_host("127.000.000.001"));
}

#[test]
fn ssrf_alternate_notations_rejected_by_validate_url() {
    let allow = vec!["example.com".to_string()];
    for notation in [
        "http://0177.0.0.1",
        "http://0x7f000001",
        "http://2130706433",
        "http://127.000.000.001",
    ] {
        let err = validate_url(notation, &allow).unwrap_err().to_string();
        assert!(
            err.contains("allowed websites"),
            "Expected allowlist rejection for {notation}, got: {err}"
        );
    }
}

// ── DNS rebinding protection ─────────────────────────────────

#[tokio::test]
async fn dns_check_blocks_localhost_resolution() {
    // "localhost" resolves to 127.0.0.1 on most systems. Even if
    // someone adds it to the allowlist, the DNS check should block it.
    let allow = vec!["localhost".to_string()];
    // validate_url itself already blocks "localhost" via the hostname check,
    // but validate_url_with_dns_check should also catch it.
    let err = validate_url_with_dns_check("https://localhost", &allow)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("local/private") || err.contains("rebinding"),
        "Expected SSRF block for localhost, got: {err}"
    );
}

#[tokio::test]
async fn dns_check_passes_for_public_resolved_ip() {
    let allow = vec!["example.com".to_string()];
    let got = validate_url_with_dns_check_with_resolver(
        "https://example.com",
        &allow,
        |host, port| async move {
            assert_eq!(host, "example.com");
            assert_eq!(port, 443);
            Ok(vec!["93.184.216.34".parse().unwrap()])
        },
    )
    .await
    .unwrap();
    assert_eq!(got, "https://example.com");
}

#[tokio::test]
async fn dns_check_blocks_private_resolved_ip() {
    let allow = vec!["example.com".to_string()];
    let err =
        validate_url_with_dns_check_with_resolver("https://example.com", &allow, |_, _| async {
            Ok(vec!["127.0.0.1".parse().unwrap()])
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("DNS rebinding blocked"));
}

#[tokio::test]
async fn dns_check_uses_explicit_port_for_resolution() {
    let allow = vec!["api.example.com".to_string()];
    let got = validate_url_with_dns_check_with_resolver(
        "http://api.example.com:8080/status",
        &allow,
        |host, port| async move {
            assert_eq!(host, "api.example.com");
            assert_eq!(port, 8080);
            Ok(vec!["93.184.216.34".parse().unwrap()])
        },
    )
    .await
    .unwrap();
    assert_eq!(got, "http://api.example.com:8080/status");
}

#[tokio::test]
async fn dns_check_returns_resolver_failure() {
    let allow = vec!["example.com".to_string()];
    let err = validate_url_with_dns_check_with_resolver(
        "https://example.com",
        &allow,
        |host, _| async move {
            anyhow::bail!("DNS resolution failed for '{host}': resolver unavailable")
        },
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("DNS resolution failed"));
}

#[tokio::test]
async fn dns_check_rejects_ip_literal_private() {
    let allow = vec!["10.0.0.1".to_string()];
    let err = validate_url_with_dns_check("https://10.0.0.1", &allow)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("local/private"));
}

#[test]
fn wildcard_allows_any_host() {
    let any = vec!["*".to_string()];
    assert!(host_matches_allowlist("docs.rs", &any));
    assert!(host_matches_allowlist("api.github.com", &any));
    assert!(host_matches_allowlist("whatever.example.org", &any));
}

#[tokio::test]
async fn wildcard_still_blocks_private_hosts() {
    // `*` opens public hosts only — SSRF block on private/local hosts stays.
    let any = vec!["*".to_string()];
    let err = validate_url_with_dns_check("https://127.0.0.1", &any)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("local/private"), "got: {err}");
}

#[test]
fn exported_ssrf_predicates_classify_non_global_ips_accurately() {
    use std::net::{Ipv4Addr, Ipv6Addr};

    // IPv4 Non-global checks
    assert!(is_non_global_v4(Ipv4Addr::new(127, 0, 0, 1)));
    assert!(is_non_global_v4(Ipv4Addr::new(10, 0, 0, 1)));
    assert!(is_non_global_v4(Ipv4Addr::new(172, 16, 0, 1)));
    assert!(is_non_global_v4(Ipv4Addr::new(192, 168, 1, 1)));
    assert!(is_non_global_v4(Ipv4Addr::new(169, 254, 1, 1)));
    assert!(is_non_global_v4(Ipv4Addr::new(100, 64, 0, 1))); // CGNAT
    assert!(is_non_global_v4(Ipv4Addr::new(240, 0, 0, 1))); // Class E
    assert!(is_non_global_v4(Ipv4Addr::new(192, 0, 2, 1))); // TEST-NET-1
    assert!(is_non_global_v4(Ipv4Addr::new(198, 51, 100, 1))); // TEST-NET-2
    assert!(is_non_global_v4(Ipv4Addr::new(203, 0, 113, 1))); // TEST-NET-3
    assert!(is_non_global_v4(Ipv4Addr::new(0, 0, 0, 0))); // 0.0.0.0/8
    assert!(is_non_global_v4(Ipv4Addr::new(0, 1, 2, 3))); // 0.0.0.0/8

    // IPv4 Global public IPs
    assert!(!is_non_global_v4(Ipv4Addr::new(8, 8, 8, 8)));
    assert!(!is_non_global_v4(Ipv4Addr::new(1, 1, 1, 1)));
    assert!(!is_non_global_v4(Ipv4Addr::new(140, 82, 121, 4)));
    // Non-TEST-NET IPs in adjacent /24 blocks should not be classified as non-global
    assert!(!is_non_global_v4(Ipv4Addr::new(198, 51, 1, 1)));
    assert!(!is_non_global_v4(Ipv4Addr::new(203, 0, 1, 1)));

    // IPv6 Non-global checks
    assert!(is_non_global_v6(Ipv6Addr::LOCALHOST));
    assert!(is_non_global_v6(Ipv6Addr::UNSPECIFIED));
    assert!(is_non_global_v6("fc00::1".parse().unwrap()));
    assert!(is_non_global_v6("fe80::1".parse().unwrap()));
    assert!(is_non_global_v6("2001:db8::1".parse().unwrap()));

    // IPv6 Global public IPs
    assert!(!is_non_global_v6("2606:4700:4700::1111".parse().unwrap()));

    // Host helper checks (including ASCII case-insensitivity and trailing dot)
    assert!(is_private_or_local_host("localhost"));
    assert!(is_private_or_local_host("LOCALHOST"));
    assert!(is_private_or_local_host("localhost."));
    assert!(is_private_or_local_host("my-service.localhost"));
    assert!(is_private_or_local_host("MY-SERVICE.LOCALHOST"));
    assert!(is_private_or_local_host("device.local"));
    assert!(is_private_or_local_host("DEVICE.LOCAL"));
    assert!(is_private_or_local_host("device.local."));
    assert!(is_private_or_local_host("127.0.0.1"));
    assert!(is_private_or_local_host("[::1]"));
    assert!(!is_private_or_local_host("github.com"));
    assert!(!is_private_or_local_host("api.openai.com"));
}
