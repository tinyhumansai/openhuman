use super::*;
use serde_json::json;

#[test]
fn recall_memories_request_accepts_compatibility_noop_params() {
    let request: RecallMemoriesRequest = serde_json::from_value(json!({
        "namespace": "team",
        "top_k": 7,
        "min_retention": 0.8,
        "as_of": 1700000000.0
    }))
    .expect("compatibility params should deserialize");

    assert_eq!(request.namespace, "team");
    assert_eq!(request.top_k, Some(7));
    assert_eq!(request.min_retention, Some(0.8));
    assert_eq!(request.as_of, Some(1_700_000_000.0));
}

#[test]
fn recall_memories_request_limit_resolution_ignores_compatibility_noop_params() {
    let request: RecallMemoriesRequest = serde_json::from_value(json!({
        "namespace": "team",
        "limit": 3,
        "min_retention": 0.5,
        "as_of": 1700000000.0
    }))
    .expect("request should deserialize");

    assert_eq!(request.resolved_limit(), 3);
}

// ── resolved_limit priorities ─────────────────────────────────

#[test]
fn recall_memories_resolved_limit_prefers_top_k_over_max_chunks_and_limit() {
    let req = RecallMemoriesRequest {
        namespace: "n".into(),
        min_retention: None,
        as_of: None,
        limit: Some(5),
        max_chunks: Some(7),
        top_k: Some(9),
    };
    assert_eq!(req.resolved_limit(), 9);
}

#[test]
fn recall_memories_resolved_limit_falls_back_to_max_chunks_then_limit_then_default() {
    let without_top_k = RecallMemoriesRequest {
        namespace: "n".into(),
        min_retention: None,
        as_of: None,
        limit: Some(5),
        max_chunks: Some(7),
        top_k: None,
    };
    assert_eq!(without_top_k.resolved_limit(), 7);

    let limit_only = RecallMemoriesRequest {
        namespace: "n".into(),
        min_retention: None,
        as_of: None,
        limit: Some(5),
        max_chunks: None,
        top_k: None,
    };
    assert_eq!(limit_only.resolved_limit(), 5);

    let none = RecallMemoriesRequest {
        namespace: "n".into(),
        min_retention: None,
        as_of: None,
        limit: None,
        max_chunks: None,
        top_k: None,
    };
    assert_eq!(none.resolved_limit(), 10);
}

#[test]
fn query_namespace_resolved_limit_prefers_max_chunks_then_limit_then_default() {
    let req = QueryNamespaceRequest {
        namespace: "n".into(),
        query: "q".into(),
        include_references: None,
        document_ids: None,
        limit: Some(3),
        max_chunks: Some(9),
    };
    assert_eq!(req.resolved_limit(), 9);

    let req_limit_only = QueryNamespaceRequest {
        namespace: "n".into(),
        query: "q".into(),
        include_references: None,
        document_ids: None,
        limit: Some(3),
        max_chunks: None,
    };
    assert_eq!(req_limit_only.resolved_limit(), 3);

    let req_none = QueryNamespaceRequest {
        namespace: "n".into(),
        query: "q".into(),
        include_references: None,
        document_ids: None,
        limit: None,
        max_chunks: None,
    };
    assert_eq!(req_none.resolved_limit(), 10);
}

#[test]
fn recall_context_resolved_limit_prefers_max_chunks_then_limit_then_default() {
    let req = RecallContextRequest {
        namespace: "n".into(),
        include_references: None,
        limit: Some(3),
        max_chunks: Some(9),
    };
    assert_eq!(req.resolved_limit(), 9);

    let req_limit_only = RecallContextRequest {
        namespace: "n".into(),
        include_references: None,
        limit: Some(3),
        max_chunks: None,
    };
    assert_eq!(req_limit_only.resolved_limit(), 3);

    let req_none = RecallContextRequest {
        namespace: "n".into(),
        include_references: None,
        limit: None,
        max_chunks: None,
    };
    assert_eq!(req_none.resolved_limit(), 10);
}

// ── deny_unknown_fields enforcement ───────────────────────────

#[test]
fn query_namespace_request_rejects_unknown_fields() {
    let err = serde_json::from_value::<QueryNamespaceRequest>(json!({
        "namespace": "n",
        "query": "q",
        "bogus": 1
    }))
    .unwrap_err();
    assert!(err.to_string().contains("bogus"));
}

#[test]
fn recall_context_request_rejects_unknown_fields() {
    let err = serde_json::from_value::<RecallContextRequest>(json!({
        "namespace": "n",
        "bogus": true
    }))
    .unwrap_err();
    assert!(err.to_string().contains("bogus"));
}

#[test]
fn empty_request_rejects_any_field() {
    let err = serde_json::from_value::<EmptyRequest>(json!({"x": 1})).unwrap_err();
    assert!(err.to_string().contains("x"));
    serde_json::from_value::<EmptyRequest>(json!({})).unwrap();
}

// ── MemoryInitRequest tolerates backwards-compatible jwt_token ────

#[test]
fn memory_init_request_jwt_token_is_optional_and_ignored() {
    let without: MemoryInitRequest = serde_json::from_value(json!({})).unwrap();
    assert_eq!(without.jwt_token, None);
    let with: MemoryInitRequest = serde_json::from_value(json!({"jwt_token": "abc"})).unwrap();
    assert_eq!(with.jwt_token.as_deref(), Some("abc"));
}

// ── ApiError / ApiMeta / ApiEnvelope round-trip ──────────────

#[test]
fn api_error_round_trips_with_optional_details() {
    let err = ApiError {
        code: "E".into(),
        message: "boom".into(),
        details: Some(json!({"why": "reason"})),
    };
    let s = serde_json::to_string(&err).unwrap();
    let back: ApiError = serde_json::from_str(&s).unwrap();
    assert_eq!(back.code, "E");
    assert_eq!(back.message, "boom");
    assert!(back.details.is_some());
}

#[test]
fn api_error_without_details_omits_field_when_serialized() {
    let err = ApiError {
        code: "E".into(),
        message: "boom".into(),
        details: None,
    };
    let s = serde_json::to_string(&err).unwrap();
    assert!(!s.contains("details"), "got: {s}");
}

#[test]
fn api_envelope_round_trip_preserves_data_and_meta() {
    let env = ApiEnvelope::<u32> {
        data: Some(42),
        error: None,
        meta: ApiMeta {
            request_id: "r1".into(),
            latency_seconds: Some(0.5),
            cached: Some(false),
            counts: None,
            pagination: Some(PaginationMeta {
                limit: 10,
                offset: 0,
                count: 1,
            }),
        },
    };
    let s = serde_json::to_string(&env).unwrap();
    let back: ApiEnvelope<u32> = serde_json::from_str(&s).unwrap();
    assert_eq!(back.data, Some(42));
    assert!(back.error.is_none());
    assert_eq!(back.meta.pagination.unwrap().count, 1);
}

#[test]
fn default_memory_relative_dir_is_memory() {
    assert_eq!(default_memory_relative_dir(), "memory");
}
