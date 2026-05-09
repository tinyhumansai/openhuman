use super::key_bytes_from_string;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;

#[test]
fn decodes_base64url_no_pad() {
    // A 32-byte key that, when base64url-encoded, contains both `-` and `_`.
    let raw = [
        0xff_u8, 0xfb, 0xef, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa,
        0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
        0x0b, 0x0c, 0x0d,
    ];
    let url_key = URL_SAFE_NO_PAD.encode(raw);
    assert!(url_key.contains('-') || url_key.contains('_'));
    let decoded = key_bytes_from_string(&url_key).unwrap();
    assert_eq!(decoded, raw);
}

#[test]
fn decodes_standard_base64() {
    let raw = [0x41_u8; 32];
    let std_key = STANDARD.encode(raw);
    let decoded = key_bytes_from_string(&std_key).unwrap();
    assert_eq!(decoded, raw);
}

#[test]
fn decodes_raw_32_byte_key() {
    let raw = "abcdefghijklmnopqrstuvwxyz012345";
    assert_eq!(raw.len(), 32);
    let decoded = key_bytes_from_string(raw).unwrap();
    assert_eq!(decoded, raw.as_bytes());
}

#[test]
fn trims_whitespace() {
    let raw = [0x42_u8; 32];
    let url_key = format!("  {}\n", URL_SAFE_NO_PAD.encode(raw));
    let decoded = key_bytes_from_string(&url_key).unwrap();
    assert_eq!(decoded, raw);
}

#[test]
fn rejects_wrong_length() {
    let err = key_bytes_from_string("tooshort").unwrap_err();
    assert!(err.to_string().contains("must decode to 32 raw bytes"));
}

use super::user_id_from_profile_payload;
use serde_json::json;

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
