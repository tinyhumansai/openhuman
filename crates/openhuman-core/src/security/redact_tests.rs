use super::*;

#[cfg(feature = "security-module")]
/// A config whose module is the published archive, downloaded into a scratch
/// install under `target/tmp` (a `TMPDIR` under `/tmp` breaks the policy tests).
///
/// One install serves the whole test process: the log bridge is process-global
/// and keeps the config it was first armed with, so a per-test install that is
/// deleted when its test ends would pull the module out from under every later
/// test. The directory lives until the process exits.
pub(crate) fn released_config() -> ((), crate::config::Config) {
    static INSTALL: std::sync::OnceLock<(tempfile::TempDir, crate::config::Config)> =
        std::sync::OnceLock::new();
    let (_dir, config) = INSTALL.get_or_init(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(&root).unwrap();
        let base = dir.path().canonicalize().unwrap();
        let mut config = crate::config::Config::default();
        config.modules.enabled = true;
        config.modules.allow_download = true;
        config.modules.install_dir = Some(base.join("modules").to_string_lossy().into_owned());
        config.action_dir = base.join("action");
        config.workspace_dir = base.join("state");
        (dir, config)
    });
    ((), config.clone())
}

/// The runtime every module-backed test runs on. The module connection and the
/// sync seam belong to the runtime that first started them, and a per-test
/// runtime is dropped when its test ends, which would pull them out from under
/// every later test in the process.
#[cfg(feature = "security-module")]
pub(crate) fn shared_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("shared module test runtime")
    })
}

/// Load the module and arm the log bridge through an ordinary async call.
///
/// The first call downloads and starts the archive, which can outlast the
/// client's load budget; the load carries on in the background and the client
/// stays retryable, so ask again until it answers.
#[cfg(feature = "security-module")]
pub(crate) async fn arm(config: &crate::config::Config) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let answer = crate::modules::security::text::redact(
            config,
            crate::modules::security::text::RedactMode::Text,
            "Bearer abcdefghijklmnopqrstuvwxyz0123456789",
        )
        .await;
        match answer {
            Ok(_) => return,
            Err(error) if std::time::Instant::now() < deadline && error.contains("loading") => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(error) => panic!("the released module must answer a redaction: {error:?}"),
        }
    }
}

#[cfg(feature = "security-module")]
fn corpus(name: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Every URL in both shared corpora through the released module: credentials,
/// path, query and fragment are gone from the origin and host forms, and the
/// forms are the ones the removed host helpers produced.
#[cfg(feature = "security-module")]
#[test]
#[cfg(feature = "native-security-tests")]
fn endpoint_helpers_match_the_url_corpus_on_the_released_archive() {
    shared_runtime().block_on(async {
        let (_dir, config) = released_config();
        arm(&config).await;
        for name in [
            "security-redaction-corpus.json",
            "security-redaction-extended-corpus.json",
        ] {
            for case in corpus(name)["urls"].as_array().unwrap() {
                let input = case["input"].as_str().unwrap();
                let origin = endpoint_origin(input);
                let host = endpoint_host(input);
                for secret in ["alice", "hunter2", "key=secret", "private", "s3cr3t", "/v1"] {
                    assert!(
                        !origin.contains(secret) && !host.contains(secret),
                        "case {} leaked {secret}: {origin} / {host}",
                        case["case"]
                    );
                }
                if let Some(expected) = case.get("endpoint").and_then(|v| v.as_str()) {
                    assert_eq!(origin, expected, "origin case {}", case["case"]);
                }
                if let Some(expected) = case.get("inference").and_then(|v| v.as_str()) {
                    assert_eq!(host, expected, "host case {}", case["case"]);
                }
            }
        }
        assert_eq!(
            endpoint_origin("https://user:pass@example.com/path/secret?apiKey=123"),
            "https://example.com"
        );
        assert_eq!(
            endpoint_origin("http://example.com:8080/path?token=secret"),
            "http://example.com:8080"
        );
        assert_eq!(
            endpoint("https://user:pass@example.com/v1/skills?apiKey=123#private"),
            "https://example.com/v1/skills"
        );
    });
}

/// The sync text pass removes every secret shape of the shared text corpora and
/// keeps ordinary text, from a plain thread and from a runtime worker.
#[cfg(feature = "security-module")]
#[test]
#[cfg(feature = "native-security-tests")]
fn text_helper_redacts_the_text_corpus_on_the_released_archive() {
    shared_runtime().block_on(async {
        let (_dir, config) = released_config();
        arm(&config).await;
        for name in [
            "security-redaction-corpus.json",
            "security-redaction-extended-corpus.json",
        ] {
            for case in corpus(name)["text"].as_array().unwrap() {
                let input: String = match case["input_parts"].as_array() {
                    Some(parts) => parts.iter().map(|p| p.as_str().unwrap()).collect(),
                    None => case["input"].as_str().unwrap().to_string(),
                };
                let needle = case["needle"].as_str().unwrap();
                let out = std::thread::spawn({
                    let input = input.clone();
                    move || text(&input)
                })
                .join()
                .unwrap();
                if case["case"] == "ordinary_text" {
                    assert_eq!(out, input, "ordinary text must pass through");
                } else {
                    assert!(!out.contains(needle), "case {} leaked: {out}", case["case"]);
                }
                let inside = text(&input);
                assert_eq!(inside, out, "case {} differs by thread", case["case"]);
            }
        }
    });
}

/// A timestamp and a bare digit run survive (corroborated card policy), and a
/// card with an issuer prefix does not.
#[cfg(feature = "security-module")]
#[test]
#[cfg(feature = "native-security-tests")]
fn text_helper_keeps_timestamps_and_removes_corroborated_cards() {
    shared_runtime().block_on(async {
        let (_dir, config) = released_config();
        arm(&config).await;
        assert_eq!(text("at 1700000000000 ms"), "at 1700000000000 ms");
        let card = text("card 4111 1111 1111 1111 on file");
        assert!(!card.contains("4111 1111 1111 1111"), "{card}");
    });
}

#[test]
fn text_suppresses_the_whole_value_when_module_cannot_answer() {
    let out = answer_or_suppress(None);
    assert_eq!(out, crate::core::log_redaction::SUPPRESSED);
}

#[test]
fn text_prefers_the_module_answer() {
    assert_eq!(
        answer_or_suppress(Some("module said".into())),
        "module said"
    );
}

/// The moved callers end to end on the released archive: the artifact redactor
/// reports a secret-bearing body as rewritten without the secret, leaves a clean
/// body unchanged, and the halt-summary error line loses the secret.
#[cfg(feature = "security-module")]
#[test]
#[cfg(feature = "native-security-tests")]
fn moved_text_callers_redact_through_the_released_archive() {
    shared_runtime().block_on(async {
        use tinyagents_harness::artifacts::ArtifactRedactor;

        let (_dir, config) = released_config();
        arm(&config).await;
        let redactor = crate::agent::harness::artifact_offload::SanitizingRedactor;

        let secret_body = "result ok\nAuthorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789\n";
        let rewritten = redactor.redact(secret_body);
        assert!(rewritten.changed, "a secret-bearing body must be rewritten");
        assert!(!rewritten.text.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(rewritten.text.contains("result ok"), "{}", rewritten.text);

        let clean = redactor.redact("plain output with no secrets\n");
        assert!(!clean.changed);
        assert_eq!(clean.text, "plain output with no secrets\n");

        let failure = format!(
            "request failed: api_key={}-{}",
            "sk", "live-abcdefghijklmnopqrstuvwxyz0123456789"
        );
        let line = crate::agent::session_host::stop_summary::short_error_line(&failure);
        assert!(!line.contains("abcdefghijklmnopqrstuvwxyz"), "{line}");
        assert!(line.contains("request failed"), "{line}");

        let home = dirs::home_dir().expect("home dir resolvable in native test");
        let path = home.join("OpenHuman").join("projects");
        let redacted = crate::config::redact_home(&path);
        assert!(redacted.starts_with("~/"), "{redacted}");
        assert!(
            !redacted.contains(&*home.to_string_lossy()),
            "home path leaked: {redacted}"
        );
        assert!(redacted.contains("OpenHuman/projects"), "{redacted}");
    });
}

#[test]
fn arguments_withheld_when_the_module_cannot_answer_carry_only_the_shape() {
    let args = serde_json::json!({"body": "dear alice, the password is hunter2", "to": "a@b.c"});
    let out = arguments_from(None, &args);
    let rendered = out.to_string();
    assert!(
        !rendered.contains("hunter2") && !rendered.contains("alice"),
        "{rendered}"
    );
    assert_eq!(
        out,
        serde_json::json!("<redacted: object (2 keys); arguments unavailable>")
    );
    assert_eq!(
        arguments_from(None, &serde_json::json!(["a", "b"])),
        serde_json::json!("<redacted: array (2 items); arguments unavailable>")
    );
    assert_eq!(
        arguments_from(None, &serde_json::Value::Null),
        serde_json::Value::Null
    );
}

#[test]
fn arguments_answer_that_is_not_json_is_withheld() {
    let args = serde_json::json!({"k": "v"});
    let out = arguments_from(Some("not json {".to_string()), &args);
    assert_eq!(
        out,
        serde_json::json!("<redacted: object (1 keys); arguments unavailable>")
    );
}

#[test]
fn arguments_use_the_module_answer_when_it_is_json() {
    let args = serde_json::json!({"k": "v"});
    assert_eq!(
        arguments_from(
            Some("{\"k\":\"<redacted: string (1 chars)>\"}".to_string()),
            &args
        ),
        serde_json::json!({"k": "<redacted: string (1 chars)>"})
    );
}

#[test]
fn arguments_without_an_armed_module_never_return_the_original() {
    // The process bridge may or may not be armed by an ignored test that ran
    // earlier in this process; either way the secret must not survive.
    let args = serde_json::json!({"body": "Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123"});
    let out = arguments(&args).to_string();
    assert!(!out.contains("abcdefghijklmnopqrstuvwxyz"), "{out}");
}

/// Whole-URL log fields (storage and backend URLs, the config snapshot) go
/// through `scrub_secrets`: the boot floor, then the module. The floor alone
/// misses a scheme-less `user:pass@host` DSN; the module pass is what removes it,
/// so this is pinned on the released archive rather than assumed.
#[cfg(feature = "security-module")]
#[test]
#[cfg(feature = "native-security-tests")]
fn whole_url_log_fields_lose_their_credentials_with_the_module_armed() {
    shared_runtime().block_on(async {
        let (_dir, config) = released_config();
        arm(&config).await;
        let secret = "hunter2pw";
        for input in [
            format!("https://alice:{secret}@db.example.com/app?x=1"),
            format!("redis://:{secret}@cache.internal:6379/0"),
            format!("postgres://app:{secret}@db.internal:5432/app"),
            format!("alice:{secret}@db.internal:5432/app"),
            format!("db.internal:5432/app?password={secret}"),
            format!("mongodb://app:{secret}@db/x?authSource=admin"),
        ] {
            let out = crate::core::log_redaction::scrub_secrets(&input);
            assert!(!out.contains(secret), "{input} -> {out}");
        }
        // An ordinary URL keeps its host and path.
        let plain = crate::core::log_redaction::scrub_secrets("https://example.com/docs/page");
        assert!(plain.contains("example.com"), "{plain}");
    });
}

/// A result larger than the module's request bound is answered in line-aligned
/// chunks and reassembled byte for byte.
#[test]
fn credentials_chunk_large_text_on_line_boundaries_and_reassemble() {
    let line = "x".repeat(99) + "\n";
    let text = line.repeat(2000); // ~200 KB
    let mut sizes = Vec::new();
    let out = credentials_with(&text, |chunk| {
        sizes.push(chunk.len());
        assert!(chunk.ends_with('\n'), "chunk split mid-line");
        Some(chunk.to_string())
    })
    .unwrap();
    assert_eq!(out, text);
    assert!(sizes.len() > 1);
    assert!(sizes.iter().all(|n| *n <= CREDENTIAL_CHUNK_BYTES));
}

#[test]
fn credentials_split_a_single_huge_line_on_a_char_boundary() {
    let text = "é".repeat(CREDENTIAL_CHUNK_BYTES); // 2 bytes per char, no newline
    let out = credentials_with(&text, |chunk| Some(chunk.to_string())).unwrap();
    assert_eq!(out, text);
}

#[test]
fn credentials_fail_when_any_chunk_cannot_be_answered() {
    let text = "line\n".repeat(CREDENTIAL_CHUNK_BYTES);
    let mut calls = 0;
    let out = credentials_with(&text, |chunk| {
        calls += 1;
        (calls < 2).then(|| chunk.to_string())
    });
    assert!(out.is_none());
}

/// Tool results are withheld when TinySecurity is not built: the diagnostic
/// floor is not the authority for model-visible tool output.
#[cfg(not(feature = "security-module"))]
#[test]
fn credentials_without_security_module_withholds_tool_results() {
    assert_eq!(credentials("hello world"), None);
}
