use super::*;

#[test]
fn native_failure_copy_uses_supported_locale_and_interpolates_retry_delay() {
    let rendered = render_channel_failure(
        "zh-CN",
        "Your AI provider is rate-limiting requests. Retry in 12 seconds.",
        Some("chat_error.rate_limited"),
        Some(&serde_json::json!({ "retry_after_secs": 12 })),
    );

    assert!(rendered.contains("你的 AI 提供商"));
    assert!(rendered.contains("12"));
    assert!(!rendered.contains("Your AI provider"));
}

#[test]
fn native_failure_copy_preserves_queued_prefix_and_sanitized_detail() {
    let rendered = render_channel_failure(
        "fr",
        "queued turn was not run: authentication failed\n\n> sanitized provider detail",
        Some("chat_error.auth_error"),
        Some(&serde_json::json!({ "detail": "sanitized provider detail" })),
    );

    assert!(rendered.starts_with("queued turn was not run: "));
    assert!(rendered.contains("problème d'authentification"));
    assert!(rendered.ends_with("> sanitized provider detail"));
    assert!(!rendered.contains("authentication failed"));
}

#[test]
fn unknown_locale_key_and_parameter_types_fall_back_safely() {
    assert_eq!(supported_channel_locale(None), None);
    assert_eq!(supported_channel_locale(Some(" xx ")), None);
    assert_eq!(supported_channel_locale(Some(" zh-CN ")), Some("zh-CN"));

    let english = render_channel_failure(
        "xx",
        "safe English fallback",
        Some("chat_error.rate_limited"),
        Some(&serde_json::json!({ "retry_after_secs": "not-a-number", "detail": 4 })),
    );
    assert_eq!(
        english,
        chat_error_catalog::translation("en", "chat_error.rate_limited").unwrap()
    );
    assert!(!english.contains("not-a-number"));
    assert!(!english.contains('4'));

    let unknown = render_channel_failure(
        "fr",
        "safe English fallback",
        Some("chat_error.from_a_newer_core"),
        None,
    );
    assert_eq!(unknown, "safe English fallback");
}

const CATALOG_KEYS: &[&str] = &[
    "chat_error.action_budget",
    "chat_error.auth_error",
    "chat_error.budget_exhausted",
    "chat_error.capability_unsupported",
    "chat_error.chat_template_rejected",
    "chat_error.codex_session_expired",
    "chat_error.context_overflow",
    "chat_error.empty_response",
    "chat_error.inference",
    "chat_error.local_session_managed_unavailable",
    "chat_error.malformed_history",
    "chat_error.managed_budget_exhausted",
    "chat_error.managed_internal",
    "chat_error.managed_malformed_request",
    "chat_error.managed_rate_limited",
    "chat_error.managed_request_rejected",
    "chat_error.managed_unavailable",
    "chat_error.max_iterations",
    "chat_error.model_config_rejected",
    "chat_error.model_unavailable",
    "chat_error.network",
    "chat_error.payload_too_large",
    "chat_error.provider_unavailable",
    "chat_error.rate_limited",
    "chat_error.rate_limited_billing",
    "chat_error.request_rejected",
    "chat_error.retryHint.aboutMinute",
    "chat_error.retryHint.aboutMinutes",
    "chat_error.retryHint.immediately",
    "chat_error.retryHint.oneSecond",
    "chat_error.retryHint.seconds",
    "chat_error.session_expired",
    "chat_error.timeout",
    "chat_error.turn_timeout",
];

#[test]
fn catalog_has_non_empty_copy_for_every_supported_locale_and_key() {
    for locale in chat_error_catalog::SUPPORTED_LOCALES {
        for key in CATALOG_KEYS {
            let text = chat_error_catalog::translation(locale, key)
                .unwrap_or_else(|| panic!("missing {locale} / {key}"));
            assert!(!text.trim().is_empty(), "empty {locale} / {key}");
            // The `{n}` placeholder exists exactly where the English source has one.
            let english_has = chat_error_catalog::translation("en", key)
                .unwrap()
                .contains("{n}");
            assert_eq!(
                text.contains("{n}"),
                english_has,
                "placeholder drift {locale} / {key}"
            );
        }
    }
}

#[test]
fn catalog_rejects_unknown_locale_or_key() {
    assert!(chat_error_catalog::translation("xx", "chat_error.timeout").is_none());
    assert!(chat_error_catalog::translation("en", "chat_error.nope").is_none());
}

#[test]
fn every_locale_renders_each_retry_hint_bucket() {
    for locale in chat_error_catalog::SUPPORTED_LOCALES {
        for (secs, expect_n) in [
            (0u64, None),
            (1, None),
            (30, Some("30")),
            (60, None),
            (150, Some("3")),
        ] {
            let out = render_channel_failure(
                locale,
                "fallback",
                Some("chat_error.rate_limited"),
                Some(&serde_json::json!({ "retry_after_secs": secs })),
            );
            assert_ne!(out, "fallback");
            assert!(!out.contains("{n}"), "{locale} {secs}: {out}");
            if let Some(n) = expect_n {
                assert!(out.contains(n), "{locale} {secs}: {out}");
            }
        }
    }
}
