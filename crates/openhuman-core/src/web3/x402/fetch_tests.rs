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
    for name in ["PAYMENT-SIGNATURE", "payment-signature", "X-PAYMENT", "x-Payment"] {
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
    for url in ["https://127.0.0.1/x", "https://10.0.0.1/x", "https://localhost/x"] {
        assert!(guard_destination(url).await.is_err(), "{url}");
    }
}

#[test]
fn body_text_truncates_at_the_cap_and_flags_it() {
    assert_eq!(body_text(b"hello".to_vec(), 5), ("hello".to_string(), false));
    assert_eq!(body_text(b"hello!".to_vec(), 5), ("hello".to_string(), true));
    // A cut inside a multi-byte character never yields invalid UTF-8.
    let (text, truncated) = body_text("aé".as_bytes().to_vec(), 2);
    assert!(truncated);
    assert_eq!(text, "a");
}

#[test]
fn a_refused_authorization_is_reported_with_the_refusal_prefix() {
    let refused = prepare_error(
        crate::modules::wallet_host::X402RpcPermitOutcome::Refused("over cap"),
        "module said no",
    );
    assert!(refused.starts_with(REFUSED_PREFIX), "{refused}");
    assert!(refused.contains("over cap"), "{refused}");

    let failed = prepare_error(
        crate::modules::wallet_host::X402RpcPermitOutcome::Unused,
        "challenge invalid",
    );
    assert!(!failed.starts_with(REFUSED_PREFIX), "{failed}");
    assert!(failed.contains("challenge invalid"), "{failed}");
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
