use super::*;

#[test]
fn catalog_keeps_the_auth_oauth_wire_names() {
    let names: Vec<String> = all_oauth_controller_schemas()
        .iter()
        .map(|s| format!("{}.{}", s.namespace, s.function))
        .collect();
    assert_eq!(
        names,
        vec![
            "auth.oauth_connect",
            "auth.oauth_list_integrations",
            "auth.oauth_fetch_integration_tokens",
            "auth.oauth_revoke_integration",
        ]
    );
    assert_eq!(all_oauth_registered_controllers().len(), FUNCTIONS.len());
}

#[test]
fn connect_requires_provider_and_tokens_require_id_and_key() {
    let connect = oauth_schemas("auth_oauth_connect");
    let required: Vec<_> = connect
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["provider"]);
    let tokens = oauth_schemas("auth_oauth_fetch_integration_tokens");
    let required: Vec<_> = tokens
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["integrationId", "key"]);
    assert_eq!(oauth_schemas("nope").function, "unknown");
}

#[test]
fn params_are_camel_case() {
    let params: AuthOauthConnectParams = deserialize_params(
        serde_json::json!({"provider": "github", "skillId": "s"})
            .as_object()
            .unwrap()
            .clone(),
    )
    .unwrap();
    assert_eq!(params.skill_id.as_deref(), Some("s"));
    assert!(deserialize_params::<AuthOauthRevokeParams>(Map::new()).is_err());
}
