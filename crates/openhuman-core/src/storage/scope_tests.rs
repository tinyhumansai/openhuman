use super::*;

#[test]
fn profile_is_a_tenant_scope() {
    assert_eq!(scope_for_profile("alice").as_str(), "u:alice");
}

#[test]
fn unprofiled_agent_lives_under_local() {
    assert_eq!(scope_for_agent("asha").as_str(), "local/agent:asha");
    assert_eq!(scope_for_agent("").as_str(), "local/agent:default");
}

#[test]
fn module_scope_follows_the_tenant() {
    assert_eq!(
        scope_for_module(None, "tinymcp").unwrap().as_str(),
        "local/module:tinymcp"
    );
    assert_eq!(
        scope_for_module(Some("alice"), "tinymcp").unwrap().as_str(),
        "u:alice/module:tinymcp"
    );
}

#[test]
fn odd_ids_hash_inside_their_tenant() {
    let scope = scope_for_profile("not a valid id");
    assert!(scope.as_str().starts_with("u:~"));
    let agent = scope.agent("x").unwrap();
    assert!(agent.as_str().starts_with(scope.as_str()));
    assert_eq!(agent.tenant_scope(), Some(scope));
}

#[test]
fn host_parser_rejects_opaque_legacy_values_without_changing_the_value() {
    let legacy = Scope::new("profile:alice").unwrap();
    assert_eq!(legacy.as_str(), "profile:alice");
    assert!(legacy.parsed().is_err());
    assert!(Scope::parse("profile:alice").is_err());

    let current = Scope::parse("u:alice/agent:writer").unwrap();
    assert_eq!(current.to_string(), "u:alice/agent:writer");
    assert_eq!(current.as_str(), "u:alice/agent:writer");
}

#[test]
fn hashed_segments_use_the_stable_lowercase_base32_alphabet() {
    let first = scope_for_profile("alice with spaces");
    let second = scope_for_profile("alice with spaces");
    assert_eq!(first, second);
    let segment = first.as_str().strip_prefix("u:~").unwrap();
    assert_eq!(segment.len(), 26);
    assert_eq!(segment, "sxpjfw7gwtcgocxriok7rbxl7t");
    assert!(segment
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || (b'2'..=b'7').contains(&byte)));
}
