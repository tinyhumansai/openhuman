use super::{scrub_secrets, scrub_with, FLOOR_MAX_INPUT_BYTES, SUPPRESSED};

#[test]
fn scrubs_bearer_token() {
    assert_eq!(
        scrub_secrets("Authorization: Bearer abc123xyz"),
        "Authorization: Bearer [REDACTED]"
    );
}

#[test]
fn scrubs_api_key_assignment() {
    assert_eq!(scrub_secrets("api_key=sk-abc123"), "api_key=[REDACTED]");
}

#[test]
fn scrubs_anthropic_key() {
    assert_eq!(
        scrub_secrets("key: sk-ant-api03-abcdefghijklmnop"),
        "key: [REDACTED]"
    );
}

#[test]
fn scrubs_bare_generic_sk_key() {
    assert_eq!(scrub_secrets("sk-abcdefghijklmnopqrstuvwx"), "[REDACTED]");
}

#[test]
fn scrubs_generic_sk_key_with_separators() {
    // A `_` or `-` mid-suffix must not leave a trailing fragment unredacted.
    assert_eq!(scrub_secrets("sk-abcdefghijklmnopqrst_uv"), "[REDACTED]");
    assert_eq!(scrub_secrets("sk-abcdefghij-klmnopqrst_uv"), "[REDACTED]");
}

#[test]
fn leaves_plain_diagnostics_intact() {
    let msg = "profile 42: derived rate clamp exceeded (max_iterations=8)";
    assert_eq!(scrub_secrets(msg), msg);
}

#[test]
fn scrubs_url_userinfo_query_and_fragment() {
    assert_eq!(
        scrub_secrets("GET https://bob:hunter2@api.example.com/v1/x?token=abc&b=2#frag failed"),
        "GET https://[REDACTED]@api.example.com/v1/x?[REDACTED] failed"
    );
}

#[test]
fn scrubs_cookie_lines_and_basic_credentials() {
    assert_eq!(
        scrub_secrets("Cookie: sid=abc; other=def\nx"),
        "Cookie: [REDACTED]\nx"
    );
    assert_eq!(
        scrub_secrets("Authorization: Basic dXNlcjpwYXNzd29yZA=="),
        "Authorization: Basic [REDACTED]"
    );
}

#[test]
fn scrubs_password_and_secret_assignments() {
    assert_eq!(
        scrub_secrets("password=hunter2 ok"),
        "password=[REDACTED] ok"
    );
    assert_eq!(
        scrub_secrets(r#"{"client_secret": "abcdef"}"#),
        r#"{"client_secret": "[REDACTED]"}"#
    );
}

#[test]
fn scrubs_home_directory_prefixes() {
    assert_eq!(
        scrub_secrets("open /home/alice/report.txt"),
        "open /home/[REDACTED]/report.txt"
    );
    assert_eq!(
        scrub_secrets("open /Users/alice/report.txt"),
        "open /Users/[REDACTED]/report.txt"
    );
    assert_eq!(
        scrub_secrets("open C:\\Users\\alice\\report.txt"),
        "open C:\\Users\\[REDACTED]\\report.txt"
    );
}

#[test]
fn oversized_input_is_suppressed_not_printed() {
    let big = "a".repeat(super::FLOOR_MAX_INPUT_BYTES + 1);
    assert_eq!(scrub_secrets(&big), super::SUPPRESSED);
}

#[test]
fn floor_is_idempotent() {
    let once = scrub_secrets("https://u:p@h.example/p?q=1 Bearer abc password=x /home/al/x");
    assert_eq!(scrub_secrets(&once), once);
}

/// ADR 7 conformance: every shared-corpus case the floor owns (`host_scrub`)
/// loses its needle. The module-side half of the subset check runs in the
/// tinysecurity repository against the same corpus file.
#[test]
fn shared_corpus_host_scrub_cases_lose_their_needle() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/tinysecurity/crates/tinysecurity-redact/tests/fixtures/security-redaction-corpus.json");
    let corpus: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("corpus fixture")).unwrap();
    let mut checked = 0;
    for case in corpus["text"].as_array().unwrap() {
        let owned = case["removed_by"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "host_scrub");
        if !owned {
            continue;
        }
        let out = scrub_secrets(case["input"].as_str().unwrap());
        let needle = case["needle"].as_str().unwrap();
        assert!(!out.contains(needle), "case {} leaked", case["case"]);
        checked += 1;
    }
    assert!(checked > 0, "corpus lost its host_scrub cases");
}

#[test]
fn floor_never_removes_ordinary_corpus_text() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/tinysecurity/crates/tinysecurity-redact/tests/fixtures/security-redaction-corpus.json");
    let corpus: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for case in corpus["text"].as_array().unwrap() {
        if case["case"] == "ordinary_text" {
            let input = case["input"].as_str().unwrap();
            assert_eq!(scrub_secrets(input), input);
        }
    }
}

#[test]
fn module_pass_sees_only_floor_output_and_may_tighten_it() {
    fn tighten(text: &str) -> Option<String> {
        assert!(
            !text.contains("hunter2"),
            "module saw unfloored text: {text}"
        );
        Some(text.replace("alice@example.com", "[REDACTED]"))
    }
    let out = scrub_with("password=hunter2 mail alice@example.com", Some(tighten));
    assert!(
        !out.contains("hunter2") && !out.contains("alice@example.com"),
        "{out}"
    );
}

#[test]
fn unavailable_module_leaves_the_floor_result_never_the_original() {
    fn down(_: &str) -> Option<String> {
        None
    }
    let out = scrub_with("Bearer abcdefghijklmnop", Some(down));
    assert!(!out.contains("abcdefghijklmnop"), "{out}");
}

#[test]
fn oversized_input_is_suppressed_without_calling_the_module() {
    fn never(_: &str) -> Option<String> {
        panic!("module must not see suppressed input");
    }
    let big = "a".repeat(FLOOR_MAX_INPUT_BYTES + 1);
    assert_eq!(scrub_with(&big, Some(never)), SUPPRESSED);
}
