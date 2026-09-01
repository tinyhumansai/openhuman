//! Wire pins for the surrounding module.
//!
//! These assert **bytes**, not behaviour. The four taxonomy types were
//! `tinycortex::memory::health`'s until #5560 and are this host's now; every
//! literal below is what the engine's copy serialised to before the move,
//! captured from a running build rather than transcribed from its source, so
//! the comparison is against what the RPC actually emitted.
//!
//! # Why the table is exhaustive rather than a spot check
//!
//! `report_tests` already pins the two response envelopes, but it only ever
//! names four codes. A hand-written enum has eleven spellings, eleven
//! remediation keys and a class table, and a typo in any of the other seven
//! would ship: the frontend resolves an unknown `remediation_key` to nothing,
//! and a mis-spelled `code` makes
//! [`FailureCode::from_str`](super::FailureCode::from_str) drop a live cause on
//! the floor — a status panel that silently stops explaining itself, which is
//! the failure mode #5560 must not introduce while claiming to change nothing.
//!
//! [`CODES`] is therefore the whole vocabulary, and
//! [`every_variant_is_in_the_table`] is what stops it rotting: a new variant is
//! a compile error in its `match`, not a silently unpinned row.

use super::*;

/// `(variant, wire string, class, remediation key)` for every [`FailureCode`].
///
/// Order matches the enum's declaration order, which is also the serde
/// discriminant order — not that anything depends on it, since every
/// representation here is by name.
const CODES: &[(FailureCode, &str, FailureClass, &str)] = &[
    (
        FailureCode::BudgetExhausted,
        "budget_exhausted",
        FailureClass::Unrecoverable,
        "memory.health.remediation.budget_exhausted",
    ),
    (
        FailureCode::AuthMissing,
        "auth_missing",
        FailureClass::Unrecoverable,
        "memory.health.remediation.auth_missing",
    ),
    (
        FailureCode::AuthInvalid,
        "auth_invalid",
        FailureClass::Unrecoverable,
        "memory.health.remediation.auth_invalid",
    ),
    (
        FailureCode::EmbeddingsUnconfigured,
        "embeddings_unconfigured",
        FailureClass::Unrecoverable,
        "memory.health.remediation.embeddings_unconfigured",
    ),
    (
        FailureCode::EmbeddingDimMismatch,
        "embedding_dim_mismatch",
        FailureClass::Unrecoverable,
        "memory.health.remediation.embedding_dim_mismatch",
    ),
    (
        FailureCode::LocalModelUnavailable,
        "local_model_unavailable",
        FailureClass::Transient,
        "memory.health.remediation.local_model_unavailable",
    ),
    (
        FailureCode::ExtractionTimeout,
        "extraction_timeout",
        FailureClass::Transient,
        "memory.health.remediation.extraction_timeout",
    ),
    (
        FailureCode::SummarizerUnavailable,
        "summarizer_unavailable",
        FailureClass::Unrecoverable,
        "memory.health.remediation.summarizer_unavailable",
    ),
    (
        FailureCode::EmptyInputRefused,
        "empty_input_refused",
        FailureClass::Unrecoverable,
        "memory.health.remediation.empty_input_refused",
    ),
    (
        FailureCode::StorageUnavailable,
        "storage_unavailable",
        FailureClass::Unrecoverable,
        "memory.health.remediation.storage_unavailable",
    ),
    (
        FailureCode::Transient,
        "transient",
        FailureClass::Transient,
        "memory.health.remediation.transient",
    ),
];

/// A variant added to [`FailureCode`] must be added to [`CODES`] too, or its
/// spelling, class and remediation key go unpinned.
///
/// The `match` is what enforces it: adding a variant fails to compile here
/// first, and the length assertion then fails until the table catches up.
/// Without this, every other test in this file would keep passing over a
/// vocabulary it no longer covers.
#[test]
fn every_variant_is_in_the_table() {
    fn position(code: FailureCode) -> usize {
        match code {
            FailureCode::BudgetExhausted => 0,
            FailureCode::AuthMissing => 1,
            FailureCode::AuthInvalid => 2,
            FailureCode::EmbeddingsUnconfigured => 3,
            FailureCode::EmbeddingDimMismatch => 4,
            FailureCode::LocalModelUnavailable => 5,
            FailureCode::ExtractionTimeout => 6,
            FailureCode::SummarizerUnavailable => 7,
            FailureCode::EmptyInputRefused => 8,
            FailureCode::StorageUnavailable => 9,
            FailureCode::Transient => 10,
        }
    }

    assert_eq!(CODES.len(), 11, "the table must cover every variant");
    for (index, (code, ..)) in CODES.iter().enumerate() {
        assert_eq!(position(*code), index, "table out of order at {index}");
    }
}

/// The whole vocabulary, one row at a time: the wire string, the serde
/// spelling, the derived class, the remediation key, and the round trip back.
///
/// `as_str` and the serde representation are asserted to agree — they are two
/// independent tables (an explicit `match` and `#[serde(rename_all)]`), read by
/// different consumers, and nothing but this makes them the same words.
#[test]
fn every_code_serialises_and_parses_to_its_pinned_spelling() {
    for (code, wire, class, remediation_key) in CODES {
        assert_eq!(code.as_str(), *wire, "as_str for {code:?}");
        assert_eq!(
            serde_json::to_value(code).unwrap(),
            serde_json::json!(wire),
            "serde spelling for {code:?}"
        );
        assert_eq!(
            serde_json::from_value::<FailureCode>(serde_json::json!(wire)).unwrap(),
            *code,
            "serde round trip for {code:?}"
        );
        assert_eq!(
            FailureCode::from_str(wire),
            Some(*code),
            "from_str for {code:?}"
        );
        assert_eq!(code.class(), *class, "class for {code:?}");
        assert_eq!(
            code.remediation_key(),
            *remediation_key,
            "remediation key for {code:?}"
        );
    }
}

/// The remediation keys are a frontend contract, so the *shape* is pinned too:
/// every key is `memory.health.remediation.<the code's own wire string>`.
///
/// Asserted as a relation rather than as eleven more literals, because that is
/// the property a locale file relies on — a key that drifts away from its code
/// resolves to nothing and the panel renders a blank remediation.
#[test]
fn every_remediation_key_is_derived_from_its_code() {
    for (code, wire, _, remediation_key) in CODES {
        assert_eq!(
            *remediation_key,
            format!("memory.health.remediation.{wire}"),
            "remediation key shape for {code:?}"
        );
    }
}

/// A code this build has no variant for is `None`, not a nearest guess.
#[test]
fn an_unknown_code_does_not_parse() {
    assert_eq!(
        FailureCode::from_str("a_cause_this_build_never_heard_of"),
        None
    );
    assert_eq!(FailureCode::from_str(""), None);
    // Casing is not normalised: the wire is snake_case and only snake_case.
    assert_eq!(FailureCode::from_str("Auth_Missing"), None);
    assert_eq!(FailureCode::from_str("AuthMissing"), None);
}

/// Both `FailureClass` spellings, in both directions.
#[test]
fn failure_class_serialises_to_its_pinned_spelling() {
    for (class, wire) in [
        (FailureClass::Transient, "transient"),
        (FailureClass::Unrecoverable, "unrecoverable"),
    ] {
        assert_eq!(class.as_str(), wire);
        assert_eq!(
            serde_json::to_value(class).unwrap(),
            serde_json::json!(wire)
        );
        assert_eq!(
            serde_json::from_value::<FailureClass>(serde_json::json!(wire)).unwrap(),
            class
        );
    }
}

/// `PipelineFailure`'s object shape: three fields always, `detail` only when
/// present. Field names are asserted as an exact object, so a rename cannot
/// pass by adding a field the assertion does not mention.
#[test]
fn pipeline_failure_object_shape_is_unchanged() {
    let bare = PipelineFailure::new(FailureCode::EmbeddingsUnconfigured);
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        serde_json::json!({
            "code": "embeddings_unconfigured",
            "class": "unrecoverable",
            "remediation_key": "memory.health.remediation.embeddings_unconfigured",
        }),
        "detail is omitted when None, never emitted as null"
    );

    let detailed = PipelineFailure::new(FailureCode::Transient).with_detail("boom");
    assert_eq!(
        serde_json::to_value(&detailed).unwrap(),
        serde_json::json!({
            "code": "transient",
            "class": "transient",
            "remediation_key": "memory.health.remediation.transient",
            "detail": "boom",
        })
    );
}

/// The deserialisation tolerance the wire has always had: `detail` may be
/// absent. Older payloads (and the driver's own, when it attaches none) omit
/// it, so `#[serde(default)]` is load-bearing rather than decoration.
#[test]
fn pipeline_failure_deserialises_without_a_detail() {
    let parsed: PipelineFailure = serde_json::from_str(
        r#"{"code":"auth_missing","class":"unrecoverable","remediation_key":"k"}"#,
    )
    .expect("detail is optional");
    assert_eq!(parsed.code, FailureCode::AuthMissing);
    assert_eq!(parsed.class, FailureClass::Unrecoverable);
    assert_eq!(parsed.remediation_key, "k");
    assert_eq!(parsed.detail, None);
}

/// `new` derives the class and the remediation key from the code, and
/// `is_unrecoverable` reads the class rather than re-deriving from the code —
/// which is what lets `report`/`rpc` override a class the driver stated
/// without the two disagreeing.
#[test]
fn new_derives_class_and_key_and_is_unrecoverable_reads_the_class() {
    for (code, _, class, remediation_key) in CODES {
        let failure = PipelineFailure::new(*code);
        assert_eq!(failure.class, *class);
        assert_eq!(failure.remediation_key, *remediation_key);
        assert_eq!(failure.detail, None);
        assert_eq!(
            failure.is_unrecoverable(),
            *class == FailureClass::Unrecoverable
        );
    }

    let mut overridden = PipelineFailure::new(FailureCode::BudgetExhausted);
    assert!(overridden.is_unrecoverable());
    overridden.class = FailureClass::Transient;
    assert!(!overridden.is_unrecoverable());
}

/// `with_detail` caps at 200 **characters** and marks the cut with `…`.
///
/// Characters, not bytes: a byte slice would panic on a multi-byte boundary,
/// and the detail can carry a provider's non-ASCII error body.
#[test]
fn with_detail_truncates_by_characters_and_marks_the_cut() {
    let short = PipelineFailure::new(FailureCode::Transient).with_detail("x");
    assert_eq!(short.detail.as_deref(), Some("x"));

    let exact = PipelineFailure::new(FailureCode::Transient).with_detail("a".repeat(200));
    assert_eq!(exact.detail.as_deref().unwrap().chars().count(), 200);
    assert!(!exact.detail.as_deref().unwrap().ends_with('…'));

    let long = PipelineFailure::new(FailureCode::Transient).with_detail("a".repeat(250));
    let detail = long.detail.as_deref().unwrap();
    assert_eq!(detail.chars().count(), 201, "200 kept plus the ellipsis");
    assert!(detail.ends_with('…'));

    // Multi-byte input must not be sliced mid-codepoint.
    let wide = PipelineFailure::new(FailureCode::Transient).with_detail("é".repeat(250));
    let detail = wide.detail.as_deref().unwrap();
    assert_eq!(detail.chars().count(), 201);
    assert!(detail.starts_with('é'));
}

/// The `Display` rendering that reaches logs: `code (class)`, with the detail
/// appended only when there is one.
#[test]
fn pipeline_failure_display_is_unchanged() {
    assert_eq!(
        PipelineFailure::new(FailureCode::AuthMissing).to_string(),
        "auth_missing (unrecoverable)"
    );
    assert_eq!(
        PipelineFailure::new(FailureCode::AuthMissing)
            .with_detail("boom")
            .to_string(),
        "auth_missing (unrecoverable): boom"
    );
}

/// `DegradedState`'s object shape: the three flags always present (`storage`
/// has `#[serde(default)]` and deliberately **no** `skip_serializing_if`),
/// `cause` only when known.
#[test]
fn degraded_state_object_shape_is_unchanged() {
    assert_eq!(
        serde_json::to_value(DegradedState::default()).unwrap(),
        serde_json::json!({
            "semantic_recall": false,
            "structure": false,
            "storage": false,
        }),
        "all three flags are emitted; cause is omitted when None"
    );

    assert_eq!(
        serde_json::to_value(DegradedState {
            semantic_recall: true,
            structure: true,
            storage: true,
            cause: Some(PipelineFailure::new(FailureCode::AuthInvalid).with_detail("d")),
        })
        .unwrap(),
        serde_json::json!({
            "semantic_recall": true,
            "structure": true,
            "storage": true,
            "cause": {
                "code": "auth_invalid",
                "class": "unrecoverable",
                "remediation_key": "memory.health.remediation.auth_invalid",
                "detail": "d",
            },
        })
    );
}

/// An older client's payload — no `storage`, no `cause` — still deserialises,
/// with `storage` defaulting to `false`. That backward compatibility is the
/// whole reason `storage` carries `#[serde(default)]`.
#[test]
fn degraded_state_deserialises_a_payload_without_storage_or_cause() {
    let parsed: DegradedState =
        serde_json::from_str(r#"{"semantic_recall":true,"structure":false}"#)
            .expect("storage and cause are both optional");
    assert_eq!(
        parsed,
        DegradedState {
            semantic_recall: true,
            structure: false,
            storage: false,
            cause: None,
        }
    );
}

/// `is_degraded` is the OR of the three flags and ignores `cause` — a cause
/// with no flag set is a report about something already cleared, not a
/// degradation.
#[test]
fn is_degraded_is_the_or_of_the_three_flags() {
    assert!(!DegradedState::default().is_degraded());
    for state in [
        DegradedState {
            semantic_recall: true,
            ..Default::default()
        },
        DegradedState {
            structure: true,
            ..Default::default()
        },
        DegradedState {
            storage: true,
            ..Default::default()
        },
    ] {
        assert!(state.is_degraded());
    }
    assert!(!DegradedState {
        cause: Some(PipelineFailure::new(FailureCode::Transient)),
        ..Default::default()
    }
    .is_degraded());
}
