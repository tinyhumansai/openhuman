use super::{
    apply_max_result_chars, format_deterministic_memory_hits, parse_memory_fast_path_enabled,
    MEMORY_FAST_PATH_LIMIT,
};
use crate::openhuman::memory::api::provider::retrieval::{
    RetrievalHit, RetrievalNodeKind, RetrievalResponse,
};
use chrono::Utc;

// The contract's hit, not the engine's. `tree_kind` is an open `String`
// here where the engine's is a `TreeKind` enum; the module serialises that
// enum with `rename_all = "snake_case"`, so `"source"` is the same value
// the engine's `TreeKind::Source` encodes to — no fixture drift, and one
// fewer `tinycortex::` import in the tree (#5560).
fn hit(content: &str, scope: &str, score: f32) -> RetrievalHit {
    RetrievalHit {
        node_id: "n1".into(),
        node_kind: RetrievalNodeKind::Summary,
        tree_id: "t1".into(),
        tree_kind: Some("source".into()),
        tree_scope: scope.into(),
        level: 1,
        content: content.into(),
        entities: vec![],
        topics: vec![],
        time_range_start: Utc::now(),
        time_range_end: Utc::now(),
        score,
        child_ids: vec![],
        source_ref: None,
    }
}

#[test]
fn fast_path_enabled_by_default_and_opt_out_parsing() {
    // Unset / unrecognised → enabled (fast path on by default).
    assert!(parse_memory_fast_path_enabled(None));
    assert!(parse_memory_fast_path_enabled(Some("1")));
    assert!(parse_memory_fast_path_enabled(Some("yes")));
    // Explicit falsy values → disabled (fall back to the model walk).
    for off in ["0", "false", "no", "off", " OFF ", "False"] {
        assert!(
            !parse_memory_fast_path_enabled(Some(off)),
            "{off:?} must disable the fast path"
        );
    }
}

#[test]
fn format_returns_none_for_no_hits() {
    // Empty response → None so the caller falls back to the full sub-agent
    // (the empty/degraded case is #4655's territory, not this fast path).
    let resp = RetrievalResponse::default();
    assert!(format_deterministic_memory_hits(&resp).is_none());
}

#[test]
fn format_renders_hits_with_scope_content_and_score() {
    let resp = RetrievalResponse {
        hits: vec![
            hit("Q3 OKR is to ship memory v2", "notes", 0.91),
            hit("prefers concise replies", "profile", 0.80),
        ],
        total: 2,
        truncated: false,
    };
    let out = format_deterministic_memory_hits(&resp).expect("hits present → Some");
    assert!(out.contains("Retrieved 2 relevant memories"), "{out}");
    assert!(out.contains("[notes] Q3 OKR is to ship memory v2"), "{out}");
    assert!(out.contains("[profile] prefers concise replies"), "{out}");
    assert!(out.contains("(relevance 0.91)"), "{out}");
    // Numbered list, no LLM synthesis needed.
    assert!(out.contains("1. ") && out.contains("2. "), "{out}");
}

#[test]
fn format_singular_wording_for_one_hit() {
    let resp = RetrievalResponse {
        hits: vec![hit("only one", "memory", 0.5)],
        total: 1,
        truncated: false,
    };
    let out = format_deterministic_memory_hits(&resp).unwrap();
    assert!(out.contains("Retrieved 1 relevant memory"), "{out}");
}

#[test]
fn format_truncates_oversized_hit_body() {
    let big = "x".repeat(5_000);
    let resp = RetrievalResponse {
        hits: vec![hit(&big, "memory", 0.42)],
        total: 1,
        truncated: false,
    };
    let out = format_deterministic_memory_hits(&resp).unwrap();
    assert!(
        out.contains(" …"),
        "oversized body must be ellipsised: {out}"
    );
    assert!(
        out.chars().count() < big.chars().count(),
        "output must be shorter than the raw 5k-char body"
    );
}

#[test]
fn fast_path_limit_is_small_single_digit() {
    // Guards against an accidental blow-up of the deterministic fan-out.
    assert!(
        (1..=16).contains(&MEMORY_FAST_PATH_LIMIT),
        "fast-path limit should stay small: {MEMORY_FAST_PATH_LIMIT}"
    );
}

#[test]
fn max_result_chars_none_is_noop() {
    // No cap → output untouched (the `agent_memory` default).
    let mut out = "hello world".to_string();
    apply_max_result_chars(&mut out, None, "agent_memory");
    assert_eq!(out, "hello world");
}

#[test]
fn max_result_chars_under_cap_is_noop() {
    let mut out = "short".to_string();
    apply_max_result_chars(&mut out, Some(100), "agent_memory");
    assert_eq!(out, "short");
}

#[test]
fn max_result_chars_over_cap_truncates_with_marker() {
    let mut out = "x".repeat(50);
    apply_max_result_chars(&mut out, Some(10), "agent_memory");
    assert!(out.starts_with(&"x".repeat(10)), "{out}");
    assert!(out.ends_with("[...truncated]"), "{out}");
    // 10 kept chars + the marker, and shorter than the 50-char original.
    assert!(out.chars().count() < 50, "{out}");
}

#[test]
fn max_result_chars_truncates_on_char_boundary_for_multibyte() {
    // Cap lands mid-run of multi-byte chars; must not panic or split a char.
    let mut out = "é".repeat(20); // each 'é' is 2 bytes
    apply_max_result_chars(&mut out, Some(5), "agent_memory");
    assert!(out.starts_with(&"é".repeat(5)), "{out}");
    assert!(out.ends_with("[...truncated]"), "{out}");
}
