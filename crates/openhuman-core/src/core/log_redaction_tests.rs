use super::{floor_scrub_text, scrub_secrets, scrub_with, FLOOR_MAX_INPUT_BYTES, SUPPRESSED};

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

/// The shared URL corpus: the floor is the one URL redactor for log fields that
/// hold a whole URL (the former util, storage and migration helpers), so the
/// credentials and query of every corpus input must be gone from its output.
#[test]
fn shared_url_corpus_loses_userinfo_and_query_but_keeps_plain_urls() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/security-redaction-corpus.json"
    )))
    .unwrap();
    for case in corpus["urls"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let out = scrub_secrets(input);
        for needle in ["alice", "hunter2", "key=secret", "private"] {
            assert!(
                !out.contains(needle),
                "case {} leaked {needle}: {out}",
                case["case"]
            );
        }
        if !input.contains('@') && !input.contains('?') {
            assert_eq!(out, input, "case {}", case["case"]);
        }
    }
}

#[test]
fn mongodb_style_storage_urls_lose_password_and_option_tokens() {
    let out =
        scrub_secrets("mongodb://app:pw@db/x?authMechanismProperties=AWS_SESSION_TOKEN%3Asecret");
    assert!(
        !out.contains("pw@") && !out.contains("AWS_SESSION_TOKEN"),
        "{out}"
    );
    assert_eq!(scrub_secrets("sqlite:/tmp/x"), "sqlite:/tmp/x");
}

#[test]
fn floor_text_scrubs_a_body_larger_than_the_floor_limit_instead_of_suppressing_it() {
    let line = "ordinary line of output\n";
    let mut body = line.repeat(FLOOR_MAX_INPUT_BYTES / line.len() + 10);
    body.push_str("Authorization: Bearer abcdefghijklmnop\n");
    body.push_str(&line.repeat(100));
    assert!(body.len() > FLOOR_MAX_INPUT_BYTES);
    let out = floor_scrub_text(&body);
    assert!(!out.contains("abcdefghijklmnop"), "secret survived");
    assert_ne!(out, SUPPRESSED);
    assert!(out.starts_with(line) && out.ends_with(line));
}

#[test]
fn floor_text_suppresses_only_an_oversized_single_line() {
    let long = "x".repeat(FLOOR_MAX_INPUT_BYTES);
    let body = format!("keep me\n{long}\nand me\n");
    let out = floor_scrub_text(&body);
    assert!(out.contains("keep me\n") && out.contains("and me\n"));
    assert!(out.contains(SUPPRESSED));
    assert!(!out.contains(&long));
}
