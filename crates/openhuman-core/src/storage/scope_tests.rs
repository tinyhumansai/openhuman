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
