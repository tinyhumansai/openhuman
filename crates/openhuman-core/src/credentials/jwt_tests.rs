use super::*;
use serde_json::json;

fn jwt_with_payload(payload_json: &str) -> String {
    use base64::Engine;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json);
    format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig")
}

#[test]
fn bearer_value_trims_surrounding_whitespace_only() {
    assert_eq!(bearer_authorization_value("my_token"), "Bearer my_token");
    assert_eq!(
        bearer_authorization_value("  spaced_token  "),
        "Bearer spaced_token"
    );
    assert_eq!(bearer_authorization_value(""), "Bearer ");
    assert_eq!(bearer_authorization_value("   "), "Bearer ");
    // Interior whitespace is preserved — see the doc comment.
    assert_eq!(
        bearer_authorization_value("token with spaces"),
        "Bearer token with spaces"
    );
}

#[test]
fn decode_jwt_payload_reads_claims_and_accepts_padded_base64() {
    let token = jwt_with_payload(r#"{"sub":"u1","exp":1700000000}"#);
    let payload = decode_jwt_payload(&token).unwrap();
    assert_eq!(payload["sub"], "u1");
    assert_eq!(payload["exp"], 1_700_000_000);

    use base64::Engine;
    let padded = base64::engine::general_purpose::URL_SAFE.encode(r#"{"sub":"padded"}"#);
    let token = format!("h.{padded}.s");
    assert_eq!(decode_jwt_payload(&token).unwrap()["sub"], "padded");
}

#[test]
fn decode_jwt_payload_none_for_non_jwt_input() {
    assert!(decode_jwt_payload("").is_none());
    assert!(decode_jwt_payload("not-a-jwt").is_none());
    assert!(decode_jwt_payload("a.!!!.c").is_none());
    assert!(decode_jwt_payload("embedded.harness.local").is_none());
}

#[test]
fn decode_jwt_exp_reads_integer_exp() {
    let token = jwt_with_payload(r#"{"sub":"u1","exp":1700000000}"#);
    assert_eq!(
        decode_jwt_exp(&token),
        DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
    );
}

#[test]
fn decode_jwt_exp_reads_float_exp() {
    let token = jwt_with_payload(r#"{"exp":1700000000.0}"#);
    assert_eq!(
        decode_jwt_exp(&token),
        DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
    );
}

#[test]
fn decode_jwt_exp_none_when_exp_absent() {
    let token = jwt_with_payload(r#"{"sub":"u1"}"#);
    assert_eq!(decode_jwt_exp(&token), None);
}

#[test]
fn decode_jwt_exp_none_for_non_jwt_or_garbage() {
    assert_eq!(decode_jwt_exp("not-a-jwt"), None);
    assert_eq!(decode_jwt_exp(""), None);
    assert_eq!(decode_jwt_exp("a.b"), None);
    // Local offline session sentinel (not a JWT) must not panic.
    assert_eq!(decode_jwt_exp("local-session-xyz"), None);
}

#[test]
fn extracts_id_from_root() {
    let payload1 = json!({ "id": "123" });
    let payload2 = json!({ "_id": "456" });
    let payload3 = json!({ "userId": "789" });

    assert_eq!(user_id_from_profile_payload(&payload1).unwrap(), "123");
    assert_eq!(user_id_from_profile_payload(&payload2).unwrap(), "456");
    assert_eq!(user_id_from_profile_payload(&payload3).unwrap(), "789");
}

#[test]
fn extracts_id_from_data_nested() {
    let payload = json!({
        "data": { "id": "abc" }
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "abc");
}

#[test]
fn extracts_id_from_user_nested() {
    let payload = json!({
        "user": { "id": "def" }
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "def");
}

#[test]
fn extracts_id_from_data_user_nested() {
    let payload = json!({
        "data": {
            "user": { "userId": "ghi" }
        }
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "ghi");
}

#[test]
fn ignores_whitespace_only_ids() {
    let payload = json!({
        "data": {
            "id": "   ",
            "_id": "real_id"
        }
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "real_id");
}

#[test]
fn trims_extracted_ids() {
    let payload = json!({
        "id": "  padded_id  "
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "padded_id");
}

#[test]
fn rejects_non_string_ids() {
    let payload = json!({
        "id": 123,
        "_id": ["not_a_string"],
        "userId": "valid_id"
    });
    assert_eq!(user_id_from_profile_payload(&payload).unwrap(), "valid_id");
}

#[test]
fn returns_none_for_missing_ids() {
    let payload = json!({
        "data": { "name": "alice" }
    });
    assert!(user_id_from_profile_payload(&payload).is_none());
}

#[test]
fn returns_none_for_non_object_payload() {
    let payload = json!("just a string");
    assert!(user_id_from_profile_payload(&payload).is_none());
}
