use super::*;

#[test]
fn build_system_prompt_default_omits_topics() {
    let p = build_system_prompt(false, None);
    assert!(!p.contains("\"topics\""));
    assert!(!p.contains("Topics are"));
    assert!(p.contains("ALL three top-level fields"));
    assert!(p.contains("entities, importance"));
}

#[test]
fn build_system_prompt_with_flag_includes_topics() {
    let p = build_system_prompt(true, None);
    assert!(p.contains("\"topics\""));
    assert!(p.contains("Topics are short free-form theme labels"));
    assert!(p.contains("ALL four top-level fields"));
    assert!(p.contains("entities, topics, importance"));
}

#[test]
fn build_system_prompt_includes_output_language_directive() {
    let p = build_system_prompt(true, Some("zh-CN"));
    assert!(p.contains("Simplified Chinese"));
    assert!(p.contains("Keep JSON keys"));
    assert!(p.contains("\"importance_reason\""));
}

#[test]
fn extraction_output_parses_topics_when_present() {
    let json = r#"{"entities":[],"topics":["rate limiting","memory tree"],"importance":0.6,"importance_reason":"r"}"#;
    let parsed: LlmExtractionOutput = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.topics, vec!["rate limiting", "memory tree"]);
}

#[test]
fn extraction_output_tolerates_missing_topics() {
    // Default extractor (emit_topics=false) — model won't emit topics
    // and parsing must still succeed.
    let json = r#"{"entities":[],"importance":0.6,"importance_reason":"r"}"#;
    let parsed: LlmExtractionOutput = serde_json::from_str(json).unwrap();
    assert!(parsed.topics.is_empty());
}

#[test]
fn parse_kind_normalisation() {
    assert_eq!(parse_kind("Person"), Some(EntityKind::Person));
    assert_eq!(parse_kind("organisation"), Some(EntityKind::Organization));
    assert_eq!(parse_kind(" PRODUCT "), Some(EntityKind::Product));
    assert!(parse_kind("Spaceship").is_none());
}

#[test]
fn parse_kind_accepts_new_semantic_kinds_and_synonyms() {
    // Datetime
    for s in ["datetime", "date", "time", "timestamp", " DateTime "] {
        assert_eq!(parse_kind(s), Some(EntityKind::Datetime), "input={s:?}");
    }
    // Technology
    for s in [
        "technology",
        "tech",
        "tool",
        "framework",
        "library",
        "language",
        "service",
    ] {
        assert_eq!(parse_kind(s), Some(EntityKind::Technology), "input={s:?}");
    }
    // Artifact
    for s in [
        "artifact",
        "reference",
        "ref",
        "pr",
        "ticket",
        "file",
        "commit",
    ] {
        assert_eq!(parse_kind(s), Some(EntityKind::Artifact), "input={s:?}");
    }
    // Quantity
    for s in ["quantity", "amount", "metric", "number", "money"] {
        assert_eq!(parse_kind(s), Some(EntityKind::Quantity), "input={s:?}");
    }
}

#[test]
fn find_char_span_handles_unicode() {
    let text = "中 Alice met Bob";
    let span = find_char_span(text, "Alice").unwrap();
    assert_eq!(span, (2, 7));
}

#[test]
fn find_char_span_returns_none_for_missing() {
    assert!(find_char_span("hello world", "absent").is_none());
}

#[test]
fn find_char_span_from_advances_past_prior_match() {
    let text = "Alice met Bob then Alice left";
    let (s1, e1, byte_after) = find_char_span_from(text, "Alice", 0, 0).unwrap();
    assert_eq!((s1, e1), (0, 5));
    // Resuming from the cursor must find the second Alice.
    let (s2, e2, _) = find_char_span_from(text, "Alice", byte_after, e1).unwrap();
    assert_eq!((s2, e2), (19, 24));
}

#[test]
fn find_char_span_from_returns_none_after_exhaustion() {
    let text = "Alice met Bob";
    let (_, _, byte_after) = find_char_span_from(text, "Alice", 0, 0).unwrap();
    // No second Alice → None.
    assert!(find_char_span_from(text, "Alice", byte_after, 5).is_none());
}

#[test]
fn find_char_span_from_preserves_utf8() {
    // Two "中" characters (3 bytes each in UTF-8); "Alice" between.
    let text = "中 Alice 中 Alice";
    let (s1, e1, byte_after) = find_char_span_from(text, "Alice", 0, 0).unwrap();
    assert_eq!((s1, e1), (2, 7));
    let (s2, e2, _) = find_char_span_from(text, "Alice", byte_after, e1).unwrap();
    // First "中 Alice " = 2 + 5 + 1 + 1 + 1 chars; second Alice starts at char 10.
    assert_eq!((s2, e2), (10, 15));
}

#[test]
fn find_char_span_from_rejects_non_char_boundary() {
    // "中" is 3 bytes; offsets 1 and 2 are mid-codepoint.
    let text = "中Alice";
    assert!(find_char_span_from(text, "Alice", 1, 0).is_none());
}

#[test]
fn into_extracted_entities_gives_distinct_spans_to_duplicate_mentions() {
    // Two "Alice" mentions in source → two distinct ExtractedEntity rows
    // with non-overlapping spans. Previously both got (0, 5).
    let out = LlmExtractionOutput {
        entities: vec![
            LlmEntity {
                kind: "person".into(),
                text: "Alice".into(),
            },
            LlmEntity {
                kind: "person".into(),
                text: "Alice".into(),
            },
        ],
        topics: vec![],
        importance: None,
        importance_reason: None,
    };
    let cfg = LlmExtractorConfig::default();
    let e = out.into_extracted_entities("Alice met Bob then Alice left", &cfg);
    assert_eq!(e.entities.len(), 2);
    assert_eq!((e.entities[0].span_start, e.entities[0].span_end), (0, 5));
    assert_eq!((e.entities[1].span_start, e.entities[1].span_end), (19, 24));
}

#[test]
fn into_extracted_entities_drops_extra_duplicate_when_source_only_has_one() {
    // Three "Alice" mentions returned by LLM, only one in source → keep
    // one, drop the rest as exhausted-duplicate.
    let out = LlmExtractionOutput {
        entities: vec![
            LlmEntity {
                kind: "person".into(),
                text: "Alice".into(),
            },
            LlmEntity {
                kind: "person".into(),
                text: "Alice".into(),
            },
            LlmEntity {
                kind: "person".into(),
                text: "Alice".into(),
            },
        ],
        topics: vec![],
        importance: None,
        importance_reason: None,
    };
    let cfg = LlmExtractorConfig::default();
    let e = out.into_extracted_entities("Alice met Bob", &cfg);
    assert_eq!(e.entities.len(), 1);
}

#[tokio::test]
async fn extract_soft_fallback_on_provider_failure() {
    // Provider always errors. extract() must NOT return Err — it must
    // return an empty ExtractedEntities with a warn log after retry
    // exhaustion.
    // #002: this path now sets the process-global "structure degraded" flag.
    // Hold the shared health test-guard so the flag is reset on entry and the
    // signal doesn't leak into parallel status tests.
    let _health_guard = crate::openhuman::memory_tree::health::test_guard();
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct FailingProvider;
    #[async_trait]
    impl ChatProvider for FailingProvider {
        fn name(&self) -> &str {
            "test:failing"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            Err(anyhow::anyhow!("simulated transport failure"))
        }
    }

    let ex = LlmEntityExtractor::new(LlmExtractorConfig::default(), Arc::new(FailingProvider));
    let out = ex.extract("some text").await.unwrap();
    assert!(out.entities.is_empty());
    assert!(out.topics.is_empty());
    assert!(out.llm_importance.is_none());
}

#[tokio::test]
async fn extract_routes_through_chat_provider_and_parses_response() {
    // Mock provider returns canned NER+importance JSON. Verify the
    // extractor parses it, recovers spans by string search, and emits the
    // expected entities + importance signal.
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct MockProvider {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl ChatProvider for MockProvider {
        fn name(&self) -> &str {
            "test:mock"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(r#"{
                "entities": [
                    {"kind":"person","text":"Alice"},
                    {"kind":"organization","text":"Anthropic"}
                ],
                "importance": 0.8,
                "importance_reason": "factual"
            }"#
            .to_string())
        }
    }

    let mock = Arc::new(MockProvider {
        calls: AtomicUsize::new(0),
    });
    let ex = LlmEntityExtractor::new(LlmExtractorConfig::default(), mock.clone());
    let out = ex.extract("Alice met Anthropic today.").await.unwrap();
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    assert_eq!(out.entities.len(), 2);
    assert_eq!(out.entities[0].text, "Alice");
    assert_eq!(out.entities[0].kind, EntityKind::Person);
    assert_eq!(out.entities[1].text, "Anthropic");
    assert_eq!(out.llm_importance, Some(0.8));
    assert_eq!(out.llm_importance_reason.as_deref(), Some("factual"));
}

#[tokio::test]
async fn extract_returns_empty_on_malformed_provider_response() {
    // Provider returns garbage. Caller must NOT see an Err — the parse
    // failure path returns empty entities (retrying the same input would
    // yield the same garbage, so we don't burn retries).
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct GarbageProvider;
    #[async_trait]
    impl ChatProvider for GarbageProvider {
        fn name(&self) -> &str {
            "test:garbage"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            Ok("not json at all".to_string())
        }
    }

    let ex = LlmEntityExtractor::new(LlmExtractorConfig::default(), Arc::new(GarbageProvider));
    let out = ex.extract("text").await.unwrap();
    assert!(out.entities.is_empty());
    assert!(out.llm_importance.is_none());
}

#[tokio::test]
async fn extract_clears_structure_latch_on_completed_empty_extraction() {
    // #3365: the structure-degraded latch is a LIVENESS signal ("the extraction
    // model is timing out"), set only after transport-failure retries are
    // exhausted. A *completed* call — even one that yields no entities (a
    // valid-but-empty result for entity-free text) — proves the model is
    // responding, so it must clear the latch. Previously only a NON-empty result
    // cleared it, so after the model recovered the latch stayed stuck whenever
    // later texts were entity-light.
    let _health_guard = crate::openhuman::memory_tree::health::test_guard();
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct EmptyButOkProvider;
    #[async_trait]
    impl ChatProvider for EmptyButOkProvider {
        fn name(&self) -> &str {
            "test:empty-ok"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            Ok(r#"{"entities": [], "topics": []}"#.to_string())
        }
    }

    crate::openhuman::memory_tree::health::mark_structure_degraded(
        crate::openhuman::memory_tree::health::FailureCode::ExtractionTimeout,
    );
    assert!(
        crate::openhuman::memory_tree::health::current_degraded_state().structure,
        "precondition: structure-degraded latch is set"
    );

    let ex = LlmEntityExtractor::new(LlmExtractorConfig::default(), Arc::new(EmptyButOkProvider));
    let out = ex.extract("entity-free text").await.unwrap();
    assert!(out.entities.is_empty(), "no entities in the response");

    assert!(
        !crate::openhuman::memory_tree::health::current_degraded_state().structure,
        "a completed extraction must clear the structure-degraded latch even when empty"
    );
}

#[test]
fn into_extracted_entities_drops_hallucinations() {
    let out = LlmExtractionOutput {
        entities: vec![
            LlmEntity {
                kind: "person".into(),
                text: "Alice".into(),
            },
            LlmEntity {
                kind: "person".into(),
                text: "ImaginaryPerson".into(),
            },
        ],
        topics: vec![],
        importance: Some(0.7),
        importance_reason: Some("substantive".into()),
    };
    let cfg = LlmExtractorConfig::default();
    let e = out.into_extracted_entities("Alice met Bob today.", &cfg);
    // Hallucinated "ImaginaryPerson" dropped; "Alice" kept.
    assert_eq!(e.entities.len(), 1);
    assert_eq!(e.entities[0].text, "Alice");
    assert_eq!(e.llm_importance, Some(0.7));
    assert_eq!(e.llm_importance_reason.as_deref(), Some("substantive"));
}

#[test]
fn into_extracted_entities_clamps_importance() {
    let out = LlmExtractionOutput {
        entities: vec![],
        topics: vec![],
        importance: Some(1.5),
        importance_reason: None,
    };
    let cfg = LlmExtractorConfig::default();
    let e = out.into_extracted_entities("text", &cfg);
    assert_eq!(e.llm_importance, Some(1.0));
}

#[test]
fn into_extracted_entities_strict_drops_unknown_kinds() {
    let out = LlmExtractionOutput {
        entities: vec![LlmEntity {
            kind: "spaceship".into(),
            text: "Enterprise".into(),
        }],
        topics: vec![],
        importance: None,
        importance_reason: None,
    };
    let cfg = LlmExtractorConfig {
        strict_kinds: true,
        ..LlmExtractorConfig::default()
    };
    let e = out.into_extracted_entities("Enterprise launched.", &cfg);
    assert!(e.entities.is_empty());
}

#[test]
fn into_extracted_entities_lenient_falls_back_to_misc() {
    let out = LlmExtractionOutput {
        entities: vec![LlmEntity {
            kind: "spaceship".into(),
            text: "Enterprise".into(),
        }],
        topics: vec![],
        importance: None,
        importance_reason: None,
    };
    let cfg = LlmExtractorConfig::default(); // strict_kinds = false
    let e = out.into_extracted_entities("Enterprise launched.", &cfg);
    assert_eq!(e.entities.len(), 1);
    assert_eq!(e.entities[0].kind, EntityKind::Misc);
}

#[test]
fn into_extracted_entities_disallowed_known_kind_falls_back_to_misc() {
    // "person" is a known kind but might be excluded by allowed_kinds.
    let out = LlmExtractionOutput {
        entities: vec![LlmEntity {
            kind: "person".into(),
            text: "Alice".into(),
        }],
        topics: vec![],
        importance: None,
        importance_reason: None,
    };
    let cfg = LlmExtractorConfig {
        allowed_kinds: vec![EntityKind::Organization], // Person not allowed
        strict_kinds: false,
        ..LlmExtractorConfig::default()
    };
    let e = out.into_extracted_entities("Alice met Bob.", &cfg);
    assert_eq!(e.entities.len(), 1);
    assert_eq!(e.entities[0].kind, EntityKind::Misc);
}

#[test]
fn build_prompt_carries_user_text_and_kind_tag() {
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct NoopProvider;
    #[async_trait]
    impl ChatProvider for NoopProvider {
        fn name(&self) -> &str {
            "test:noop"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            Ok("{}".into())
        }
    }

    let cfg = LlmExtractorConfig {
        model: "test-model".into(),
        ..LlmExtractorConfig::default()
    };
    let ex = LlmEntityExtractor::new(cfg, Arc::new(NoopProvider));
    let prompt = ex.build_prompt("hello");
    assert!(prompt.user.contains("hello"));
    assert!(prompt.user.contains("Return JSON only"));
    assert_eq!(prompt.temperature, 0.0);
    assert_eq!(prompt.kind, "memory_tree::extract");
    // System prompt should describe the JSON schema.
    assert!(prompt.system.contains("\"entities\""));
    assert!(prompt.system.contains("\"importance\""));
}

#[test]
fn truncate_for_log_short_input_unchanged() {
    assert_eq!(truncate_for_log("hi", 10), "hi");
}

#[test]
fn truncate_for_log_long_input_appends_ellipsis() {
    let long = "x".repeat(500);
    let out = truncate_for_log(&long, 10);
    assert_eq!(out.chars().count(), 11); // 10 + "…"
    assert!(out.ends_with('…'));
}

#[tokio::test]
async fn extract_retries_on_truncated_response() {
    // First response is truncated mid-JSON (serde EOF) — a stream cutoff,
    // not a wrong-shape body. It must be treated as retryable rather than
    // silently dropped; the second (complete) response then recovers the
    // entities (bug-report-2026-05-26 I1).
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct TruncatedThenCompleteProvider {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl ChatProvider for TruncatedThenCompleteProvider {
        fn name(&self) -> &str {
            "test:truncated"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                // Array never closes → serde reports EOF (is_eof()).
                Ok(r#"{"entities":[{"kind":"person","text":"Alice"}"#.to_string())
            } else {
                Ok(r#"{"entities":[{"kind":"person","text":"Alice"}],"importance":0.5,"importance_reason":"r"}"#.to_string())
            }
        }
    }

    let mock = Arc::new(TruncatedThenCompleteProvider {
        calls: AtomicUsize::new(0),
    });
    let ex = LlmEntityExtractor::new(LlmExtractorConfig::default(), mock.clone());
    let out = ex.extract("Alice met Bob.").await.unwrap();
    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        2,
        "truncation should trigger a retry, not a silent drop"
    );
    assert_eq!(out.entities.len(), 1);
    assert_eq!(out.entities[0].text, "Alice");
}

#[tokio::test]
async fn extract_does_not_retry_on_wrong_shape_response() {
    // Companion to the truncation test: a *complete* but wrong-shape body is
    // a serde error that is NOT EOF (`is_eof() == false`). Unlike a mid-JSON
    // cutoff it's deterministic and won't fix itself on retry, so it must
    // return immediately (`calls == 1`) with an empty extraction rather than
    // entering the retry loop. Guards the `Err(e) if e.is_eof()` split so a
    // future change can't accidentally retry every parse error
    // (bug-report-2026-05-26 I1).
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct WrongShapeProvider {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl ChatProvider for WrongShapeProvider {
        fn name(&self) -> &str {
            "test:wrong-shape"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            // `entities` must be an array; a scalar is a complete-input type
            // error (not a truncation), so serde reports a non-EOF error.
            Ok(r#"{"entities":123}"#.to_string())
        }
    }

    let mock = Arc::new(WrongShapeProvider {
        calls: AtomicUsize::new(0),
    });
    let ex = LlmEntityExtractor::new(LlmExtractorConfig::default(), mock.clone());
    let out = ex.extract("Alice met Bob.").await.unwrap();
    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        1,
        "a non-EOF wrong-shape response must not trigger a retry"
    );
    assert!(
        out.entities.is_empty(),
        "wrong-shape response should yield an empty extraction"
    );
}

#[test]
fn build_prompt_sets_extraction_max_tokens_cap() {
    // Extraction must cap output tokens so a credit-metered provider
    // (OpenRouter) prices the request against a realistic budget instead of
    // the model's full output window — an unset cap is what 402'd low-balance
    // BYO users on every call (TAURI-RUST-C62). build_prompt is the single
    // source of that cap.
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct NoopProvider;
    #[async_trait]
    impl ChatProvider for NoopProvider {
        fn name(&self) -> &str {
            "test:noop"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            Ok("{}".into())
        }
    }

    let ex = LlmEntityExtractor::new(LlmExtractorConfig::default(), Arc::new(NoopProvider));
    let prompt = ex.build_prompt("hello");
    assert_eq!(prompt.max_tokens, Some(EXTRACTION_MAX_OUTPUT_TOKENS));
    // Lock the value: it must stay well under any plausible per-account
    // OpenRouter balance so the pre-flight clears for a low-balance user.
    assert_eq!(EXTRACTION_MAX_OUTPUT_TOKENS, 8192);
}

#[tokio::test]
async fn extract_does_not_retry_on_permanent_402() {
    // A 402 (the BYO provider account is out of credits) is a permanent client
    // error: retrying reproduces it and fires one more Sentry event per attempt
    // (TAURI-RUST-C62: 12k events from a single user). extract() must call the
    // provider exactly once, return empty, and mark structure degraded with the
    // credits-exhausted cause (not extraction-timeout).
    let _health_guard = crate::openhuman::memory_tree::health::test_guard();
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct InsufficientCreditsProvider {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl ChatProvider for InsufficientCreditsProvider {
        fn name(&self) -> &str {
            "test:402"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            // Verbatim shape of the TAURI-RUST-C62 provider error.
            Err(anyhow::anyhow!(
                "myopenrouter API error (402 Payment Required): This request requires more \
                 credits, or fewer max_tokens. You requested up to 65536 tokens, but can only \
                 afford 49732."
            ))
        }
    }

    let mock = Arc::new(InsufficientCreditsProvider {
        calls: AtomicUsize::new(0),
    });
    let ex = LlmEntityExtractor::new(LlmExtractorConfig::default(), mock.clone());
    let out = ex.extract("some text").await.unwrap();

    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        1,
        "a permanent 402 must not be retried"
    );
    assert!(out.entities.is_empty());

    let state = crate::openhuman::memory_tree::health::current_degraded_state();
    assert!(
        state.structure,
        "a permanent extraction failure must still mark structure degraded"
    );
    assert_eq!(
        state.cause.expect("degraded cause present").code,
        crate::openhuman::memory_tree::health::FailureCode::BudgetExhausted,
        "a 402 must map to the credits-exhausted cause, not extraction-timeout"
    );
}

#[tokio::test]
async fn extract_does_not_retry_on_500_wrapped_monthly_quota() {
    // TAURI-RUST-C9A: the Kiro IDE proxy wraps a 402 monthly-quota refusal
    // inside a 500 envelope. Before the quota classifier, `is_non_retryable`
    // saw only a 500 and treated it as a transient transport failure — so
    // extract() retried 3× and each attempt fired a provider Sentry event
    // (9k events from a single quota-capped user). It must now call the
    // provider exactly once and map to the budget-exhausted cause.
    let _health_guard = crate::openhuman::memory_tree::health::test_guard();
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct MonthlyQuotaProvider {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl ChatProvider for MonthlyQuotaProvider {
        fn name(&self) -> &str {
            "test:kiro"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            // Verbatim shape of the TAURI-RUST-C9A provider error (500 envelope
            // around the inner 402 / MONTHLY_REQUEST_COUNT).
            Err(anyhow::anyhow!(
                "kiro API error (500 Internal Server Error): {{\"error\":{{\"message\":\
                 \"HTTP 402 from Kiro IDE: {{\\\"message\\\":\\\"You have reached the \
                 limit.\\\",\\\"reason\\\":\\\"MONTHLY_REQUEST_COUNT\\\"}}\",\
                 \"type\":\"server_error\"}}}}"
            ))
        }
    }

    let mock = Arc::new(MonthlyQuotaProvider {
        calls: AtomicUsize::new(0),
    });
    let ex = LlmEntityExtractor::new(LlmExtractorConfig::default(), mock.clone());
    let out = ex.extract("some text").await.unwrap();

    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        1,
        "a 500-wrapped monthly-quota refusal must not be retried"
    );
    assert!(out.entities.is_empty());

    let state = crate::openhuman::memory_tree::health::current_degraded_state();
    assert!(
        state.structure,
        "a permanent extraction failure must still mark structure degraded"
    );
    assert_eq!(
        state.cause.expect("degraded cause present").code,
        crate::openhuman::memory_tree::health::FailureCode::BudgetExhausted,
        "monthly-quota exhaustion must map to the budget-exhausted cause"
    );
}

#[tokio::test]
async fn extract_retries_transient_provider_error() {
    // The de-amplification fix must only short-circuit *permanent* errors. A
    // transport/transient failure (no 4xx, no auth marker) must still exhaust
    // the retry budget before falling back, or a flaky-network blip would drop
    // the chunk on the first try.
    let _health_guard = crate::openhuman::memory_tree::health::test_guard();
    use crate::openhuman::memory::chat::{ChatPrompt, ChatProvider};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct TransientProvider {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl ChatProvider for TransientProvider {
        fn name(&self) -> &str {
            "test:transient"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(anyhow::anyhow!(
                "error sending request for url (https://api): connection refused"
            ))
        }
    }

    let mock = Arc::new(TransientProvider {
        calls: AtomicUsize::new(0),
    });
    let ex = LlmEntityExtractor::new(LlmExtractorConfig::default(), mock.clone());
    let out = ex.extract("some text").await.unwrap();

    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        3,
        "a transient error must still exhaust the retry budget"
    );
    assert!(out.entities.is_empty());
}
