use super::*;
use serde_json::json;

fn required_input_names(s: &ControllerSchema) -> Vec<&'static str> {
    s.inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect()
}

#[test]
fn catalog_is_the_six_hosted_tunnel_functions_under_webhooks() {
    let schemas = all_webhooks_controller_schemas();
    let names: Vec<&str> = schemas.iter().map(|s| s.function).collect();
    assert_eq!(names, FUNCTIONS);
    assert!(schemas.iter().all(|s| s.namespace == "webhooks"));
    assert_eq!(all_webhooks_registered_controllers().len(), FUNCTIONS.len());
}

#[test]
fn input_contracts_are_unchanged() {
    assert!(webhooks_schemas("list_tunnels").inputs.is_empty());
    assert!(webhooks_schemas("get_bandwidth").inputs.is_empty());
    assert_eq!(
        required_input_names(&webhooks_schemas("create_tunnel")),
        vec!["name"]
    );
    for f in ["get_tunnel", "delete_tunnel", "update_tunnel"] {
        assert_eq!(required_input_names(&webhooks_schemas(f)), vec!["id"]);
    }
    let update = webhooks_schemas("update_tunnel");
    for optional in ["name", "description", "isActive"] {
        assert!(update
            .inputs
            .iter()
            .any(|f| f.name == optional && !f.required));
    }
}

#[test]
fn unknown_function_returns_error_fallback_schema() {
    let s = webhooks_schemas("nope");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.outputs[0].name, "error");
}

#[test]
fn update_params_accept_camel_case_is_active() {
    let mut params = Map::new();
    params.insert("id".into(), json!("t-1"));
    params.insert("isActive".into(), json!(false));
    let parsed: WebhookUpdateTunnelParams = deserialize_params(params).unwrap();
    assert_eq!(parsed.id, "t-1");
    assert_eq!(parsed.is_active, Some(false));
    assert!(deserialize_params::<WebhookTunnelIdParams>(Map::new()).is_err());
}
