use super::*;

#[test]
fn build_system_prompt_default_omits_topics() {
    let p = build_system_prompt(false);
    assert!(!p.contains("\"topics\""));
    assert!(!p.contains("Topics are"));
    assert!(p.contains("ALL three top-level fields"));
    assert!(p.contains("entities, importance"));
}

#[test]
fn build_system_prompt_with_flag_includes_topics() {
    let p = build_system_prompt(true);
    assert!(p.contains("\"topics\""));
    assert!(p.contains("Topics are short free-form theme labels"));
    assert!(p.contains("ALL four top-level fields"));
    assert!(p.contains("entities, topics, importance"));
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
    use crate::openhuman::memory::tree::chat::{ChatPrompt, ChatProvider};
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
    use crate::openhuman::memory::tree::chat::{ChatPrompt, ChatProvider};
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
    use crate::openhuman::memory::tree::chat::{ChatPrompt, ChatProvider};
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
    use crate::openhuman::memory::tree::chat::{ChatPrompt, ChatProvider};
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
