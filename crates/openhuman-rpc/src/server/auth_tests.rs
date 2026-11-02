use super::*;

#[test]
fn extract_query_token_returns_none_on_missing_query() {
    assert_eq!(extract_query_token(None), None);
}

#[test]
fn extract_query_token_returns_none_when_key_absent() {
    assert_eq!(extract_query_token(Some("other=1&foo=bar")), None);
}

#[test]
fn extract_query_token_returns_none_on_empty_value() {
    assert_eq!(extract_query_token(Some("token=")), None);
    assert_eq!(extract_query_token(Some("token=%20%20")), None);
}

#[test]
fn extract_query_token_returns_first_value_on_duplicate_keys() {
    // Last-wins vs first-wins is a question the FE never hits; pin
    // first-wins so any future ambiguity is documented.
    assert_eq!(
        extract_query_token(Some("token=alpha&token=beta")),
        Some("alpha".to_string())
    );
}

#[test]
fn extract_query_token_url_decodes_value() {
    // `encodeURIComponent` on the FE may percent-encode a hex token
    // accidentally (it shouldn't, but defensive); confirm round-trip.
    assert_eq!(
        extract_query_token(Some("token=cafe%2Dbabe")),
        Some("cafe-babe".to_string())
    );
}

#[test]
fn login_callbacks_are_no_longer_public() {
    // The desktop `/auth` fallback and `/auth/telegram` exchanged login tokens
    // inside the core; the host (openhuman_tinyhumans::session) owns that now.
    assert!(!PUBLIC_PATHS.contains(&"/auth"));
    assert!(!PUBLIC_PATHS.contains(&"/auth/telegram"));
}

#[test]
fn agentbox_run_and_jobs_paths_are_no_longer_public() {
    // These bypassed bearer auth only to serve the AgentBox marketplace
    // surface, which moved to tinybox. Nothing mounts them now, so they
    // must authenticate like any other path — a re-added entry here would
    // silently open an unauthenticated route.
    assert!(!is_public_path("/run"));
    assert!(!is_public_path("/jobs/abc-123"));
    assert!(!is_public_path(
        "/jobs/00000000-0000-0000-0000-000000000000"
    ));
    // Sanity: still protect the executable surface.
    assert!(!is_public_path("/rpc"));
    assert!(!is_public_path("/v1/chat/completions"));
}

#[test]
fn is_external_inference_path_matches_only_v1_routes() {
    assert!(is_external_inference_path("/v1"));
    assert!(is_external_inference_path("/v1/models"));
    assert!(is_external_inference_path("/v1/chat/completions"));
    assert!(!is_external_inference_path("/rpc"));
    assert!(!is_external_inference_path("/v10/models"));
}

#[test]
fn startup_preflight_allows_only_health_and_approval_rpc_methods() {
    assert!(super::preflight_http_path_allowed(
        &axum::http::Method::GET,
        "/health"
    ));
    assert!(super::preflight_http_path_allowed(
        &axum::http::Method::POST,
        "/rpc"
    ));
    assert!(!super::preflight_http_path_allowed(
        &axum::http::Method::GET,
        "/schema"
    ));
    assert!(!super::preflight_http_path_allowed(
        &axum::http::Method::GET,
        "/events"
    ));
    assert!(!super::preflight_http_path_allowed(
        &axum::http::Method::POST,
        "/v1/chat/completions"
    ));

    assert!(super::preflight_rpc_method_allowed("core.version"));
    assert!(super::preflight_rpc_method_allowed("core.ping"));
    assert!(super::preflight_rpc_method_allowed(
        "openhuman.service_status"
    ));
    assert!(super::preflight_rpc_method_allowed(
        "openhuman.update_version"
    ));
    assert!(super::preflight_rpc_method_allowed(
        "openhuman.approval_list_pending"
    ));
    assert!(super::preflight_rpc_method_allowed(
        "openhuman.approval_decide"
    ));
    assert!(!super::preflight_rpc_method_allowed("core.config"));
    assert!(!super::preflight_rpc_method_allowed("openhuman.config_get"));
    assert!(!super::preflight_rpc_method_allowed(
        "openhuman.update_check"
    ));
    assert!(!super::preflight_rpc_method_allowed(
        "openhuman.approval_get_gate_state"
    ));
}

#[test]
fn startup_preflight_gate_opens_the_normal_surface_after_decryption() {
    let gate = super::PreflightGate::new_active();
    assert!(gate.is_active());
    gate.open();
    assert!(!gate.is_active());
}

#[test]
fn external_inference_bearer_requires_a_matching_stored_key() {
    assert!(external_inference_bearer_matches(
        "external-test-key",
        Some("external-test-key")
    ));
    assert!(external_inference_bearer_matches(
        "external-test-key",
        Some(" external-test-key ")
    ));
    assert!(!external_inference_bearer_matches(
        "wrong-key",
        Some("external-test-key")
    ));
    assert!(!external_inference_bearer_matches(
        "external-test-key",
        None
    ));
    assert!(!external_inference_bearer_matches(
        " ",
        Some("external-test-key")
    ));
}
