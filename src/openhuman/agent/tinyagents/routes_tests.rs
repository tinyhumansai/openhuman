use super::*;

/// The fallback chain for every tier must lead with the primary and carry the
/// single same-family alternate the legacy static table encoded — the crate
/// `ModelRouter` projection is exactly behavior-neutral.
#[test]
fn route_fallback_policy_matches_legacy_chains() {
    let cases: &[(&str, Option<&[&str]>)] = &[
        (MODEL_CHAT_V1, Some(&[MODEL_CHAT_V1, MODEL_BURST_V1])),
        (MODEL_BURST_V1, Some(&[MODEL_BURST_V1, MODEL_CHAT_V1])),
        (
            MODEL_REASONING_V1,
            Some(&[MODEL_REASONING_V1, MODEL_AGENTIC_V1]),
        ),
        (
            MODEL_AGENTIC_V1,
            Some(&[MODEL_AGENTIC_V1, MODEL_REASONING_V1]),
        ),
        (MODEL_CODING_V1, Some(&[MODEL_CODING_V1, MODEL_AGENTIC_V1])),
        (
            MODEL_SUMMARIZATION_V1,
            Some(&[MODEL_SUMMARIZATION_V1, MODEL_CHAT_V1]),
        ),
        // Vision is primary-only (an image_in gate no text tier can satisfy).
        (MODEL_VISION_V1, None),
        ("hint:vision", None),
        // A raw non-tier model installs no chain.
        ("gpt-4o", None),
    ];
    for (model, expected) in cases {
        let got = route_fallback_policy(model).map(|p| p.models);
        let want = expected.map(|chain| chain.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(got, want, "fallback chain mismatch for {model}");
    }
}

/// Only the vision tier (and its hint form) imposes an `image_in` gate; the
/// common text turn stays ungated.
#[test]
fn turn_required_capabilities_gates_only_vision() {
    let vision = turn_required_capabilities(MODEL_VISION_V1).expect("vision is gated");
    assert!(vision.image_in);
    let hint = turn_required_capabilities("hint:vision").expect("hint:vision is gated");
    assert!(hint.image_in);
    for model in [
        MODEL_CHAT_V1,
        MODEL_REASONING_V1,
        MODEL_AGENTIC_V1,
        MODEL_CODING_V1,
        MODEL_BURST_V1,
        MODEL_SUMMARIZATION_V1,
        "gpt-4o",
    ] {
        assert!(
            turn_required_capabilities(model).is_none(),
            "{model} must not be capability-gated"
        );
    }
}

/// The router covers exactly the projected tier inventory (plus the hint:vision
/// gate alias), so the fallback/capability source of truth stays aligned with
/// `WORKLOAD_ROUTE_TIERS`.
#[test]
fn router_covers_the_workload_tier_inventory() {
    for tier in WORKLOAD_ROUTE_TIERS {
        assert!(
            OH_WORKLOAD_ROUTER.route(tier).is_some(),
            "router missing tier {tier}"
        );
    }
}
