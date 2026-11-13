use super::*;
use serde_json::json;

fn params(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object params")
}

#[test]
fn params_default_to_get_with_no_headers_or_body() {
    let parsed = parse_params(params(json!({
        "url": "https://api.example.test/resource",
        "maxAmountAtomic": 2500,
    })))
    .expect("valid params");
    assert_eq!(parsed.url, "https://api.example.test/resource");
    assert_eq!(parsed.method, reqwest::Method::GET);
    assert!(parsed.headers.is_empty());
    assert_eq!(parsed.body, None);
    assert_eq!(parsed.max_amount_atomic, 2500);
    assert_eq!(parsed.network, None);
    assert!(!parsed.sign_in);
}

#[test]
fn params_accept_a_network_pin_and_sign_in() {
    for network in [
        "eip155:8453",
        "eip155",
        "solana",
        "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
    ] {
        let parsed = parse_params(params(json!({
            "url": "https://api.example.test/resource",
            "maxAmountAtomic": 1,
            "network": network,
            "signIn": true,
        })))
        .expect("valid params");
        assert_eq!(parsed.network.as_deref(), Some(network));
        assert!(parsed.sign_in);
    }
    let blank = parse_params(params(json!({
        "url": "https://api.example.test/resource",
        "maxAmountAtomic": 1,
        "network": " ",
    })))
    .expect("a blank pin is no pin");
    assert_eq!(blank.network, None);
}

#[test]
fn params_reject_a_malformed_network_pin() {
    for network in [
        "eip155:8453:1",
        ":8453",
        "eip155:",
        "eip 155",
        "eip155;drop",
        &"a".repeat(65),
    ] {
        let error = parse_params(params(json!({
            "url": "https://api.example.test/resource",
            "maxAmountAtomic": 1,
            "network": network,
        })))
        .expect_err("malformed network is refused");
        assert!(error.contains("network"), "{error}");
    }
}

#[test]
fn params_accept_a_post_with_headers_and_body() {
    let parsed = parse_params(params(json!({
        "url": "https://api.example.test/resource",
        "method": "post",
        "headers": {"Content-Type": "application/json"},
        "body": "{}",
        "maxAmountAtomic": 1,
    })))
    .expect("valid params");
    assert_eq!(parsed.method, reqwest::Method::POST);
    assert_eq!(
        parsed.headers,
        vec![("Content-Type".to_string(), "application/json".to_string())]
    );
    assert_eq!(parsed.body.as_deref(), Some("{}"));
}

#[test]
fn params_reject_a_non_https_url() {
    for url in [
        "http://api.example.test/resource",
        "ftp://api.example.test/resource",
        "not a url",
    ] {
        let error = parse_params(params(json!({"url": url, "maxAmountAtomic": 1})))
            .expect_err("non-https URL is refused");
        assert!(error.contains("https"), "{error}");
    }
}

#[test]
fn params_reject_a_method_other_than_get_or_post() {
    let error = parse_params(params(json!({
        "url": "https://api.example.test/resource",
        "method": "PUT",
        "maxAmountAtomic": 1,
    })))
    .expect_err("PUT is refused");
    assert!(error.contains("method"), "{error}");
}

#[test]
fn params_reject_a_body_on_get() {
    let error = parse_params(params(json!({
        "url": "https://api.example.test/resource",
        "body": "x",
        "maxAmountAtomic": 1,
    })))
    .expect_err("GET with a body is refused");
    assert!(error.contains("body"), "{error}");
}

#[test]
fn params_reject_caller_supplied_payment_headers() {
    for name in [
        "PAYMENT-SIGNATURE",
        "payment-signature",
        "X-PAYMENT",
        "x-Payment",
        "SIGN-IN-WITH-X",
        "sign-in-with-x",
    ] {
        let error = parse_params(params(json!({
            "url": "https://api.example.test/resource",
            "headers": {name: "forged"},
            "maxAmountAtomic": 1,
        })))
        .expect_err("payment header is refused");
        assert!(error.contains("header"), "{error}");
    }
}

#[test]
fn params_reject_a_non_string_header_value() {
    assert!(parse_params(params(json!({
        "url": "https://api.example.test/resource",
        "headers": {"X-Count": 3},
        "maxAmountAtomic": 1,
    })))
    .is_err());
}

#[test]
fn params_require_a_positive_max_amount() {
    assert!(parse_params(params(json!({"url": "https://api.example.test/resource"}))).is_err());
    let error = parse_params(params(json!({
        "url": "https://api.example.test/resource",
        "maxAmountAtomic": 0,
    })))
    .expect_err("zero cap is refused");
    assert!(error.contains("maxAmountAtomic"), "{error}");
}

#[tokio::test]
async fn destination_guard_refuses_loopback_and_private_hosts() {
    for url in [
        "https://127.0.0.1/x",
        "https://10.0.0.1/x",
        "https://localhost/x",
    ] {
        assert!(guard_destination(url).await.is_err(), "{url}");
    }
}

#[test]
fn body_text_truncates_at_the_cap_and_flags_it() {
    assert_eq!(
        body_text(b"hello".to_vec(), 5),
        ("hello".to_string(), false)
    );
    assert_eq!(
        body_text(b"hello!".to_vec(), 5),
        ("hello".to_string(), true)
    );
    // A cut inside a multi-byte character never yields invalid UTF-8.
    let (text, truncated) = body_text("aé".as_bytes().to_vec(), 2);
    assert!(truncated);
    assert_eq!(text, "a");
}

#[test]
fn a_refused_authorization_is_reported_with_the_refusal_prefix() {
    let refused = prepare_error(
        crate::modules::x402_service::X402RpcPermitOutcome::Refused("over cap"),
        "module said no",
    );
    assert!(refused.starts_with(REFUSED_PREFIX), "{refused}");
    assert!(refused.contains("over cap"), "{refused}");

    let failed = prepare_error(
        crate::modules::x402_service::X402RpcPermitOutcome::Unused,
        "challenge invalid",
    );
    assert!(!failed.starts_with(REFUSED_PREFIX), "{failed}");
    assert!(failed.contains("challenge invalid"), "{failed}");
}

#[test]
fn module_refusals_before_signing_are_reported_with_the_refusal_prefix() {
    // The module's own refusals reach the host as one opaque failed call whose
    // text is the module's message; the host callback was never consulted.
    for message in [
        "ai.tinyhumans.tinywallet.Error.X402OperationFailed: x402 daily budget exceeded: 9000000/10000000 atomic units",
        "ai.tinyhumans.tinywallet.Error.X402OperationFailed: x402 monthly budget exceeded: 99000000/100000000 atomic units",
        "ai.tinyhumans.tinywallet.Error.X402OperationFailed: x402 amount 2000000 exceeds per-request cap 1000000",
        "ai.tinyhumans.tinywallet.Error.X402OperationFailed: x402 network solana:unknown is not supported; payments are accepted only in USDC on known networks",
        "ai.tinyhumans.tinywallet.Error.X402OperationFailed: x402 asset 0xabc is not the USDC accepted on eip155:8453",
        "ai.tinyhumans.tinywallet.Error.X402OperationFailed: no supported payment option (Solana exact or EVM exact) in 402 challenge",
    ] {
        let error = prepare_error(
            crate::modules::x402_service::X402RpcPermitOutcome::Unused,
            message,
        );
        assert!(error.starts_with(REFUSED_PREFIX), "{error}");
        assert!(error.contains(message), "the module's reason is kept: {error}");
    }
}

#[test]
fn failures_that_are_not_refusals_keep_the_payment_failed_wording() {
    for message in [
        "x402 service handle is unavailable",
        "x402 pending-payment capacity reached",
        "transport: connection closed",
    ] {
        let error = prepare_error(
            crate::modules::x402_service::X402RpcPermitOutcome::Unused,
            message,
        );
        assert!(!error.starts_with(REFUSED_PREFIX), "{error}");
        assert!(error.starts_with("x402_fetch: payment failed:"), "{error}");
    }
}

#[test]
fn an_unpaid_response_has_a_null_payment() {
    assert_eq!(
        response_value(200, Some("text/plain".into()), "ok".into(), false, None),
        json!({
            "status": 200,
            "contentType": "text/plain",
            "body": "ok",
            "truncated": false,
            "payment": null,
        })
    );
}

#[test]
fn a_paid_response_carries_the_payment_facts() {
    let payment = FetchPayment {
        payment_id: "x402_1".into(),
        amount_atomic: 2500,
        asset: "USDC".into(),
        recipient: "0x01".into(),
        network: "eip155:8453".into(),
        settled: true,
        settlement_response: Some("receipt".into()),
    };
    assert_eq!(
        response_value(200, None, "ok".into(), true, Some(payment)),
        json!({
            "status": 200,
            "body": "ok",
            "truncated": true,
            "payment": {
                "paymentId": "x402_1",
                "amountAtomic": 2500,
                "asset": "USDC",
                "recipient": "0x01",
                "network": "eip155:8453",
                "settled": true,
                "settlementResponse": "receipt",
            },
        })
    );
}

#[test]
fn the_payment_amount_must_not_exceed_the_caller_cap() {
    assert!(ensure_within_cap(2500, 2500).is_ok());
    let error = ensure_within_cap(2501, 2500).expect_err("over cap");
    assert!(error.starts_with(REFUSED_PREFIX), "{error}");
}

fn loopback_target(server: &wiremock::MockServer, path: &str) -> ValidatedUrl {
    ValidatedUrl {
        url: format!("{}{path}", server.uri()),
        host: "127.0.0.1".into(),
        addrs: vec![*server.address()],
    }
}

#[tokio::test]
async fn the_sign_in_retry_adds_the_credential_and_keeps_the_post() {
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/x-accounts/start"))
        .and(header(SIGN_IN_HEADER, "siwx-credential"))
        .and(header("Idempotency-Key", "k1"))
        .and(body_string(r#"{"username":"teeny"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .mount(&server)
        .await;
    let params = FetchParams {
        url: String::new(),
        method: reqwest::Method::POST,
        headers: vec![("Idempotency-Key".into(), "k1".into())],
        body: Some(r#"{"username":"teeny"}"#.into()),
        max_amount_atomic: 1,
        network: None,
        sign_in: true,
    };
    let target = loopback_target(&server, "/api/x-accounts/start");

    let response = send(&target, &params, Some((SIGN_IN_HEADER, "siwx-credential")))
        .await
        .expect("the retry reaches the server");
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn the_first_attempt_carries_no_sign_in_credential() {
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let params = parse_params(params(json!({
        "url": "https://twitr.test/x", "maxAmountAtomic": 1, "signIn": true
    })))
    .expect("params");
    send(&loopback_target(&server, "/x"), &params, None)
        .await
        .expect("sent");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].headers.contains_key(SIGN_IN_HEADER));
}

#[test]
fn a_signed_in_response_matches_the_declared_fetch_outputs() {
    let value = with_signed_in_as(
        response_value(
            200,
            Some("application/json".into()),
            "{}".into(),
            false,
            None,
        ),
        "0x0000000000000000000000000000000000000001".into(),
    );
    let declared = super::super::schemas::schemas("fetch").outputs;
    let object = value.as_object().expect("object response");
    for key in object.keys() {
        assert!(
            declared.iter().any(|field| field.name == key),
            "response key {key} is not declared in x402.fetch outputs"
        );
    }
    assert_eq!(
        object["signedInAs"],
        "0x0000000000000000000000000000000000000001"
    );
    assert_eq!(object["payment"], Value::Null);
}

#[test]
fn params_treat_a_null_sign_in_and_network_as_unset() {
    let parsed = parse_params(params(json!({
        "url": "https://api.example.test/resource",
        "maxAmountAtomic": 2500,
        "signIn": null,
        "network": null,
    })))
    .expect("null optionals are accepted, as the schema advertises");
    assert!(!parsed.sign_in);
    assert_eq!(parsed.network, None);
}
