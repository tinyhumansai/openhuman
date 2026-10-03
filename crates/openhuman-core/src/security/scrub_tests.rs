use super::*;

#[test]
fn redacts_an_api_key_in_text() {
    let out = sanitize_text("token sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789 here");
    assert!(out.report.changed());
    assert!(!out.value.contains("abcdefghijklmnopqrstuvwxyz0123456789"));
}

#[test]
fn leaves_a_timestamp_alone_under_the_host_policy() {
    let out = sanitize_text("at 1700000000000 ms");
    assert_eq!(out.value, "at 1700000000000 ms");
}

#[test]
fn drops_sensitive_json_keys() {
    let out = sanitize_json(&serde_json::json!({"password": "hunter2", "name": "x"}));
    assert!(out.value.get("password").is_none() || out.value["password"] != "hunter2");
    assert_eq!(out.value["name"], "x");
}

#[test]
fn flags_a_likely_secret() {
    assert!(has_likely_secret(
        "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB"
    ));
    assert!(!has_likely_secret("hello world"));
}
