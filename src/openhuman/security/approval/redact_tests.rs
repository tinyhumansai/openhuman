use super::*;
use serde_json::json;

#[test]
fn sensitive_string_field_is_replaced_with_marker() {
    let args = json!({ "body": "hello world", "action": "execute" });
    let red = redact_args(&args);
    assert_eq!(red["action"], json!("execute"));
    assert!(
        red["body"]
            .as_str()
            .unwrap()
            .starts_with("<redacted: string ("),
        "got {:?}",
        red["body"]
    );
}

#[test]
fn plaintext_field_is_redacted_for_encrypted_dm_tools() {
    let args = json!({
        "recipient": "@alice",
        "plaintext": "meet me at the usual spot",
        "associatedData": { "topic": "private message" }
    });
    let red = redact_args(&args);

    assert!(
        red["plaintext"]
            .as_str()
            .unwrap()
            .starts_with("<redacted: string ("),
        "got {:?}",
        red["plaintext"]
    );
    assert!(
        red["recipient"]
            .as_str()
            .unwrap()
            .starts_with("<redacted: string ("),
        "got {:?}",
        red["recipient"]
    );
    assert_eq!(red["associatedData"]["topic"], "private message");
}

#[test]
fn email_verification_code_is_redacted() {
    let args = json!({
        "cryptoId": "did:example:alice",
        "email": "alice@example.test",
        "code": "123456",
    });
    let red = redact_args(&args);

    assert_eq!(red["cryptoId"], "did:example:alice");
    assert!(
        red["email"]
            .as_str()
            .unwrap()
            .starts_with("<redacted: string ("),
        "got {:?}",
        red["email"]
    );
    assert!(
        red["code"]
            .as_str()
            .unwrap()
            .starts_with("<redacted: string ("),
        "got {:?}",
        red["code"]
    );
}

#[test]
fn network_write_content_fields_are_redacted() {
    let args = json!({
        "title": "Build my thing",
        "description": "Long private task brief",
        "coverLetter": "I can do this because...",
        "note": "Submission context",
        "reason": "Dispute details",
        "amount": "5",
        "asset": "USDC"
    });
    let red = redact_args(&args);

    for key in ["title", "description", "coverLetter", "note", "reason"] {
        assert!(
            red[key]
                .as_str()
                .unwrap()
                .starts_with("<redacted: string ("),
            "{key} was not redacted: {:?}",
            red[key]
        );
    }
    assert_eq!(red["amount"], "5");
    assert_eq!(red["asset"], "USDC");
}

#[test]
fn profile_update_fields_are_redacted() {
    let args = json!({
        "cryptoId": "did:example:alice",
        "update": {
            "displayName": "Alice Example",
            "bio": "Private bio",
            "avatar": "https://example.test/avatar.png",
            "links": ["https://example.test/private"],
            "tags": ["private-tag"],
            "actorType": "agent"
        }
    });
    let red = redact_args(&args);
    let update = red["update"].as_object().unwrap();

    assert_eq!(red["cryptoId"], "did:example:alice");
    for key in ["displayName", "bio", "avatar", "links", "tags"] {
        assert!(
            update[key].as_str().unwrap().starts_with("<redacted:"),
            "{key} was not redacted: {:?}",
            update[key]
        );
    }
    assert_eq!(update["actorType"], "agent");
}

#[test]
fn nested_sensitive_object_fields_are_redacted() {
    let args = json!({
        "action": "execute",
        "params": {
            "message": "secret",
            "channel_id": "C123",
            "tool_slug": "SLACK_SEND",
        }
    });
    let red = redact_args(&args);
    let params = red.get("params").unwrap().as_object().unwrap();
    assert!(params["message"]
        .as_str()
        .unwrap()
        .starts_with("<redacted: string"));
    assert_eq!(params["channel_id"], json!("C123"));
    assert_eq!(params["tool_slug"], json!("SLACK_SEND"));
}

#[test]
fn case_insensitive_match_on_sensitive_keys() {
    let args = json!({ "Body": "x", "TOKEN": "y" });
    let red = redact_args(&args);
    assert!(red["Body"].as_str().unwrap().starts_with("<redacted"));
    assert!(red["TOKEN"].as_str().unwrap().starts_with("<redacted"));
}

#[test]
fn array_field_redacts_to_count_marker() {
    let args = json!({ "recipients": ["a@x", "b@y", "c@z"] });
    let red = redact_args(&args);
    assert_eq!(
        red["recipients"].as_str().unwrap(),
        "<redacted: array (3 items)>"
    );
}

#[test]
fn home_path_in_unredacted_string_is_scrubbed() {
    let args = json!({ "action": "list", "cwd": "/Users/oxoxdev/work/openhuman" });
    let red = redact_args(&args);
    let cwd = red["cwd"].as_str().unwrap();
    assert!(!cwd.contains("oxoxdev"), "got {cwd}");
    assert!(cwd.contains("<HOME>"));
    assert!(cwd.ends_with("/work/openhuman"));
}

#[test]
fn windows_home_path_is_scrubbed() {
    let args = json!({ "action": "list", "cwd": "C:\\Users\\oxoxdev\\work\\openhuman" });
    let red = redact_args(&args);
    let cwd = red["cwd"].as_str().unwrap();
    assert!(!cwd.contains("oxoxdev"), "got {cwd}");
    assert!(cwd.contains("<HOME>"));
    assert!(cwd.ends_with("\\work\\openhuman"));
}

#[test]
fn linux_home_path_is_scrubbed() {
    let args = json!({ "action": "list", "cwd": "/home/jane/project" });
    let red = redact_args(&args);
    let cwd = red["cwd"].as_str().unwrap();
    assert!(!cwd.contains("jane"), "got {cwd}");
    assert!(cwd.contains("<HOME>"));
    assert!(cwd.ends_with("/project"));
}

#[test]
fn lowercase_windows_home_path_is_scrubbed() {
    // Regression: the fast-path guard was case-sensitive while the matcher is
    // case-insensitive, so a lowercase drive/`users` casing slipped through
    // unredacted and leaked the username.
    let args = json!({ "action": "list", "cwd": "c:\\users\\alice\\work" });
    let red = redact_args(&args);
    let cwd = red["cwd"].as_str().unwrap();
    assert!(!cwd.contains("alice"), "got {cwd}");
    assert!(cwd.contains("<HOME>"));
    assert!(cwd.ends_with("\\work"));
}

#[test]
fn uppercase_linux_home_path_is_scrubbed() {
    let args = json!({ "action": "list", "cwd": "/HOME/alice/work" });
    let red = redact_args(&args);
    let cwd = red["cwd"].as_str().unwrap();
    assert!(!cwd.contains("alice"), "got {cwd}");
    assert!(cwd.contains("<HOME>"));
    assert!(cwd.ends_with("/work"));
}

#[test]
fn mixed_case_home_path_is_scrubbed() {
    let args = json!({ "action": "list", "cwd": "/Home/alice/work" });
    let red = redact_args(&args);
    let cwd = red["cwd"].as_str().unwrap();
    assert!(!cwd.contains("alice"), "got {cwd}");
    assert!(cwd.contains("<HOME>"));
    assert!(cwd.ends_with("/work"));
}

#[test]
fn non_path_string_without_home_marker_is_unchanged() {
    // The guard's fast-path must still return unrelated strings verbatim.
    let args = json!({ "action": "run", "command": "echo hello world" });
    let red = redact_args(&args);
    assert_eq!(red["command"].as_str().unwrap(), "echo hello world");
}

#[test]
fn multiple_home_paths_in_same_string_all_scrubbed() {
    let args = json!({
        "action": "list",
        "summary": "from /Users/alice/a.txt to /Users/bob/b.txt",
    });
    let red = redact_args(&args);
    let summary = red["summary"].as_str().unwrap();
    assert!(!summary.contains("alice"));
    assert!(!summary.contains("bob"));
    assert_eq!(summary.matches("<HOME>").count(), 2);
}

#[test]
fn file_handoff_links_are_redacted() {
    // A presigned storage link is a bearer capability: anyone holding the
    // URL can fetch the file until it expires. Redacted args are shown on
    // the approval card AND persisted, so these must never appear clear.
    let args = json!({
        "attachment": "https://files.example.test/f_1?sig=SECRETSIGNATURE",
        "file_to_upload": "https://files.example.test/f_2?sig=ANOTHERSIG",
        "file_url": "https://files.example.test/f_3?sig=THIRDSIG",
        "public_url": "https://files.example.test/f_4?sig=FOURTHSIG",
        // A bare `url` (e.g. an `http_request` node's destination, or a
        // Composio action's benign url arg) is NOT a file-handoff key and
        // MUST stay visible so the human approver can judge the action.
        "url": "https://webhook.site/VISIBLE-DESTINATION",
    });
    let red = redact_args(&args);
    let blob = red.to_string();
    for leaked in [
        "SECRETSIGNATURE",
        "ANOTHERSIG",
        "THIRDSIG",
        "FOURTHSIG",
        "files.example.test",
    ] {
        assert!(
            !blob.contains(leaked),
            "presigned link leaked through redaction ({leaked}): {blob}"
        );
    }
    assert!(
        blob.contains("webhook.site/VISIBLE-DESTINATION"),
        "a bare `url` (e.g. an http_request destination) must NOT be redacted: {blob}"
    );
}

#[test]
fn summarize_action_pulls_safe_fields() {
    let args = json!({
        "action": "execute",
        "tool_slug": "SLACK_SEND",
        "params": { "body": "hi" }
    });
    let summary = summarize_action("composio", &args);
    assert!(summary.contains("composio"));
    assert!(summary.contains("action=execute"));
    assert!(summary.contains("tool_slug=SLACK_SEND"));
    assert!(!summary.contains("hi"));
}

#[test]
fn summarize_action_falls_back_to_size_only() {
    let args = json!({});
    let summary = summarize_action("pushover", &args);
    assert!(summary.contains("pushover"));
    assert!(summary.contains("bytes"));
}

#[test]
fn summarize_action_skill_install_is_human_readable() {
    let args = json!({ "entry_id": "notion" });
    let summary = summarize_action("skill_registry_install", &args);
    // Friendly sentence, not a key=value/byte dump (#3993).
    assert_eq!(
        summary,
        "Install the \"notion\" skill to complete your task"
    );
    assert!(!summary.contains("bytes"));
}

#[test]
fn redact_preserves_toolkit_slug_for_connect_card() {
    // The inline connect card (#3993) reads `toolkit` out of the redacted
    // args to drive the OAuth handoff, so a non-sensitive slug must survive
    // redaction verbatim while real PII alongside it is still scrubbed.
    let args = json!({ "toolkit": "gmail", "body": "secret message" });
    let redacted = redact_args(&args);
    assert_eq!(redacted["toolkit"], json!("gmail"));
    assert_ne!(redacted["body"], json!("secret message"));
}

#[test]
fn summarize_action_skill_install_without_entry_id_falls_back() {
    let args = json!({});
    let summary = summarize_action("skill_registry_install", &args);
    // Missing slug → generic fallback so we never panic or mislabel.
    assert!(summary.contains("skill_registry_install"));
    assert!(summary.contains("bytes"));
}
