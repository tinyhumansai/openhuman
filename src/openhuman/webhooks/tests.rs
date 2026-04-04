use std::collections::HashMap;

use base64::Engine;
use serde_json::json;

use crate::openhuman::webhooks::{ops::build_echo_response, WebhookRequest};

#[test]
fn echo_response_round_trips_request_payload() {
    let request = WebhookRequest {
        correlation_id: "corr-echo".to_string(),
        tunnel_id: "tid-1".to_string(),
        tunnel_uuid: "uuid-1".to_string(),
        tunnel_name: "Echo Test".to_string(),
        method: "POST".to_string(),
        path: "/echo".to_string(),
        headers: HashMap::from([(String::from("content-type"), json!("application/json"))]),
        query: HashMap::from([(String::from("mode"), String::from("test"))]),
        body: base64::engine::general_purpose::STANDARD.encode("{\"hello\":\"world\"}"),
    };

    let response = build_echo_response(&request);
    assert_eq!(response.status_code, 200);
    assert_eq!(
        response
            .headers
            .get("x-openhuman-webhook-target")
            .map(String::as_str),
        Some("echo")
    );

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(response.body)
        .expect("decode echo response body");
    let parsed: serde_json::Value =
        serde_json::from_slice(&decoded).expect("parse echo response body json");

    assert_eq!(parsed["ok"], json!(true));
    assert_eq!(parsed["echo"]["tunnelUuid"], json!("uuid-1"));
    assert_eq!(parsed["echo"]["path"], json!("/echo"));
    assert_eq!(parsed["echo"]["bodyBase64"], request.body);
}
