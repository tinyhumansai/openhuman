use tinymemory_api::Error as MemoryApiError;

use super::*;

#[test]
fn every_variant_has_its_stable_code() {
    assert_eq!(MemoryError::Off("x".into()).code(), MEMORY_OFF);
    assert_eq!(MemoryError::Unsupported("x".into()).code(), UNSUPPORTED);
    assert_eq!(MemoryError::invalid("x").code(), INVALID_REQUEST);
    assert_eq!(MemoryError::Unauthorized("x".into()).code(), UNAUTHORIZED);
    assert_eq!(MemoryError::Engine("x".into()).code(), ENGINE);
    assert_eq!(
        MemoryError::InsufficientCredits("x".into()).code(),
        INSUFFICIENT_CREDITS
    );
    assert_eq!(MemoryError::Unavailable("x".into()).code(), UNAVAILABLE);
}

#[test]
fn structured_error_carries_the_code_and_flags_user_states() {
    let off = MemoryError::Off("sign in".into()).to_structured();
    assert_eq!(off.data.as_ref().unwrap()["code"], MEMORY_OFF);
    assert_eq!(off.data.as_ref().unwrap()["kind"], MEMORY_OFF);
    assert!(off.expected_user_state);
    assert!(off.message.contains("sign in"));

    let engine = MemoryError::Engine("boom".into()).to_structured();
    assert!(!engine.expected_user_state);
}

#[test]
fn tinymemory_errors_map_onto_the_host_taxonomy() {
    use tinymemory_api::Error as E;
    let cases = [
        (E::Unsupported("a".into()), UNSUPPORTED),
        (E::InvalidRequest("a".into()), INVALID_REQUEST),
        (E::NotFound("a".into()), INVALID_REQUEST),
        (E::Config("a".into()), INVALID_REQUEST),
        (E::Unauthorized("a".into()), UNAUTHORIZED),
        (E::Conflict("a".into()), ENGINE),
        (E::Unavailable("a".into()), UNAVAILABLE),
        (E::Engine("a".into()), ENGINE),
    ];
    for (error, code) in cases {
        assert_eq!(MemoryError::from(error).code(), code);
    }
}

#[test]
fn memory_error_string_encodes_the_structured_error() {
    let encoded: String = MemoryError::invalid("bad").into();
    assert!(encoded.contains(INVALID_REQUEST), "{encoded}");
    assert!(encoded.contains("bad"), "{encoded}");
}

#[test]
fn typed_credit_errors_are_account_wide_but_other_engine_errors_are_not() {
    use tinymemory_api::Error as E;
    let credits = MemoryError::from(E::InsufficientCredits(
        "memory API recall on h: the account has insufficient credits (HTTP 402)".into(),
    ));
    assert_eq!(credits.code(), INSUFFICIENT_CREDITS);
    assert!(credits.is_account_wide());
    assert!(credits.to_structured().expected_user_state);

    let down = MemoryError::from(E::Unavailable("timed out".into()));
    assert_eq!(down.code(), UNAVAILABLE);
    assert!(down.is_account_wide());

    let other = MemoryError::from(E::Engine("[HTTP_500] boom".into()));
    assert_eq!(other.code(), ENGINE);
    assert!(!other.is_account_wide());
    assert!(!MemoryError::invalid("bad").is_account_wide());
}

/// #8081: memory not in the plan is its own account-wide user state, from
/// both TinyMemory wordings, and its RPC error never reads as an expired
/// session (which signs the user out).
#[test]
fn a_plan_refusal_is_plan_required_not_unauthorized() {
    use tinymemory_api::Error as E;
    for error in [
        E::Unauthorized(
            "[MEMORY_PLAN_REQUIRED] memory API memory/scopes on h: the session expired or the \
             API key was rejected (HTTP 403 Forbidden); re-authenticate — memory is not \
             included in this account's plan"
                .into(),
        ),
        E::Engine(
            "[MEMORY_PLAN_REQUIRED] memory API memory/scopes on h: memory is not included in \
             this account's plan (HTTP 403 Forbidden)"
                .into(),
        ),
    ] {
        let plan = MemoryError::from(error);
        assert_eq!(plan.code(), PLAN_REQUIRED);
        assert!(plan.is_account_wide());
        let structured = plan.to_structured();
        assert!(structured.expected_user_state);
        assert!(
            !crate::core::session_expiry::is_session_expired_error(&String::from(plan)),
            "{}",
            structured.message
        );
    }
    // A mention of the code that is not the leading code stays an engine failure.
    let mention = MemoryError::from(E::Engine(
        "[HTTP_500] memory API recall on h — unexpected [MEMORY_PLAN_REQUIRED] token".into(),
    ));
    assert_eq!(mention.code(), ENGINE);
    assert!(!mention.is_account_wide());
    let expired = MemoryError::from(E::Unauthorized(
        "[UNAUTHORIZED] memory API memory/scopes on h: the session expired (HTTP 401)".into(),
    ));
    assert_eq!(expired.code(), UNAUTHORIZED);
    assert_eq!(
        MemoryError::refusal_from_skip_reason(
            "unauthorized: [MEMORY_PLAN_REQUIRED] memory API memory/scopes (HTTP 403 Forbidden)"
        )
        .map(|e| e.code()),
        Some(PLAN_REQUIRED)
    );
}

#[test]
fn recall_skip_reasons_recover_only_account_wide_refusals() {
    let code =
        |reason: &str| MemoryError::refusal_from_skip_reason(reason).map(|error| error.code());
    assert_eq!(
        code("engine error: [USER_INSUFFICIENT_CREDITS] memory API fetch (HTTP 402)"),
        Some(INSUFFICIENT_CREDITS)
    );
    assert_eq!(
        code("insufficient credits: memory API fetch (HTTP 402)"),
        Some(INSUFFICIENT_CREDITS)
    );
    assert_eq!(code("unavailable: connection refused"), Some(UNAVAILABLE));
    assert_eq!(code("unauthorized: key rejected"), Some(UNAUTHORIZED));
    assert_eq!(code("engine error: index corrupt"), None);
    assert_eq!(code("invalid request: bad filter"), None);
    assert_eq!(code("empty"), None);
}

#[test]
fn upstream_errors_keep_the_memory_rpc_wire_codes_and_messages() {
    let cases = [
        (
            MemoryApiError::Off("sign in".into()),
            "MEMORY_OFF",
            "memory is off: sign in",
            true,
        ),
        (
            MemoryApiError::Unsupported("fetch".into()),
            "UNSUPPORTED",
            "unsupported: fetch",
            true,
        ),
        (
            MemoryApiError::InvalidRequest("bad filter".into()),
            "INVALID_REQUEST",
            "invalid request: bad filter",
            true,
        ),
        (
            MemoryApiError::NotFound("item".into()),
            "INVALID_REQUEST",
            "invalid request: item",
            true,
        ),
        (
            MemoryApiError::Config("missing endpoint".into()),
            "INVALID_REQUEST",
            "invalid request: missing endpoint",
            true,
        ),
        (
            MemoryApiError::Unauthorized("key rejected".into()),
            "UNAUTHORIZED",
            "unauthorized: key rejected",
            true,
        ),
        (
            MemoryApiError::InsufficientCredits("top up".into()),
            "INSUFFICIENT_CREDITS",
            "insufficient credits: top up",
            true,
        ),
        (
            MemoryApiError::Unavailable("timed out".into()),
            "UNAVAILABLE",
            "unavailable: timed out",
            true,
        ),
        (
            MemoryApiError::Conflict("already exists".into()),
            "ENGINE",
            "engine error: already exists",
            false,
        ),
        (
            MemoryApiError::Engine("index corrupt".into()),
            "ENGINE",
            "engine error: index corrupt",
            false,
        ),
    ];

    for (error, code, message, expected_user_state) in cases {
        let structured = crate::core::StructuredRpcError::from(error);
        assert_eq!(structured.data.as_ref().unwrap()["code"], code);
        assert_eq!(structured.data.as_ref().unwrap()["kind"], code);
        assert_eq!(structured.message, message);
        assert_eq!(structured.expected_user_state, expected_user_state);
        assert_eq!(
            crate::core::StructuredRpcError::decode(&structured.encode()),
            Some(structured)
        );
    }
}

#[test]
fn upstream_account_wide_classification_excludes_transient_unavailability() {
    use tinymemory_api::Error as E;

    assert!(E::Off("disabled".into()).is_account_wide());
    assert!(E::Unauthorized("bad key".into()).is_account_wide());
    assert!(E::InsufficientCredits("top up".into()).is_account_wide());
    assert!(!E::Unavailable("timeout".into()).is_account_wide());
    assert!(E::Unavailable("timeout".into()).is_transient());
    assert!(!E::InvalidRequest("bad filter".into()).is_account_wide());
    assert!(!E::Conflict("duplicate".into()).is_account_wide());
    assert!(!E::Engine("corrupt index".into()).is_account_wide());
}

#[test]
fn typed_insufficient_credits_maps_without_parsing_engine_text() {
    let error = crate::core::StructuredRpcError::from(MemoryApiError::InsufficientCredits(
        "HTTP 402".into(),
    ));
    assert_eq!(error.data.as_ref().unwrap()["code"], "INSUFFICIENT_CREDITS");
    assert_eq!(error.message, "insufficient credits: HTTP 402");
    assert!(error.expected_user_state);
}

#[test]
fn upstream_unavailable_is_transient_not_account_wide() {
    use tinymemory_api::Error as E;

    assert!(E::Off("disabled".into()).is_account_wide());
    assert!(E::Unauthorized("bad key".into()).is_account_wide());
    assert!(E::InsufficientCredits("top up".into()).is_account_wide());
    assert!(!E::Unavailable("timeout".into()).is_account_wide());
    assert!(E::Unavailable("timeout".into()).is_transient());
    assert!(!E::InvalidRequest("bad filter".into()).is_account_wide());
    assert!(!E::Conflict("duplicate".into()).is_account_wide());
    assert!(!E::Engine("corrupt index".into()).is_account_wide());
}
