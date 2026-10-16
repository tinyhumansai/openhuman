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
