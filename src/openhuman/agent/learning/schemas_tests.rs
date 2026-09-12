use super::*;

#[test]
fn all_schemas_returns_eleven() {
    assert_eq!(all_learning_controller_schemas().len(), 11);
}

#[test]
fn all_controllers_returns_eleven() {
    assert_eq!(all_learning_registered_controllers().len(), 11);
}

#[test]
fn save_profile_schema_shape() {
    let s = learning_schemas("learning_save_profile");
    assert_eq!(s.namespace, "learning");
    assert_eq!(s.function, "save_profile");
    assert!(s.inputs.iter().any(|f| f.name == "markdown" && f.required));
}

#[test]
fn linkedin_enrichment_schema() {
    let s = learning_schemas("learning_linkedin_enrichment");
    assert_eq!(s.namespace, "learning");
    assert_eq!(s.function, "linkedin_enrichment");
    // Optional `profile_url` input: the frontend supplies one when it
    // has already discovered the URL via the webview-driven Gmail
    // helper, letting the pipeline skip its Composio-only stage 1.
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "profile_url");
    assert!(!s.inputs[0].required);
    assert!(!s.outputs.is_empty());
}

#[test]
fn unknown_function_returns_unknown() {
    let s = learning_schemas("nonexistent");
    assert_eq!(s.function, "unknown");
}

#[test]
fn facet_to_json_includes_cue_families_and_evidence_refs() {
    use std::collections::HashMap;
    use tinymemory_api::host::EvidenceRef;
    use tinymemory_api::provider::{FacetState, FacetType, ProfileFacet, UserState};

    let mut cue_families = HashMap::new();
    cue_families.insert("explicit".to_string(), 3u32);
    cue_families.insert("structural".to_string(), 1u32);

    let facet = ProfileFacet {
        facet_id: "f1".into(),
        facet_type: FacetType::Preference,
        key: "style/verbosity".into(),
        value: "terse".into(),
        confidence: 0.8,
        evidence_count: 4,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 1200.0,
        state: FacetState::Active,
        stability: 0.9,
        user_state: UserState::Auto,
        evidence_refs: vec![EvidenceRef::Episodic { episodic_id: 42 }],
        class: Some("style".into()),
        cue_families: Some(cue_families),
    };

    // Populated provenance round-trips through the serializer.
    let json = facet_to_json(&facet);
    assert_eq!(json["cue_families"]["explicit"].as_u64(), Some(3));
    assert_eq!(json["cue_families"]["structural"].as_u64(), Some(1));
    assert_eq!(json["evidence_refs"][0]["type"].as_str(), Some("episodic"));
    assert_eq!(json["evidence_refs"][0]["episodic_id"].as_i64(), Some(42));

    // Empty/None provenance serializes to []/null (present, not dropped).
    let bare = ProfileFacet {
        evidence_refs: vec![],
        cue_families: None,
        ..facet
    };
    let json = facet_to_json(&bare);
    assert_eq!(json["evidence_refs"].as_array().map(Vec::len), Some(0));
    assert!(json["cue_families"].is_null());
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_learning_controller_schemas();
    let c = all_learning_registered_controllers();
    assert_eq!(s[0].function, c[0].schema.function);
}

#[test]
fn list_facets_schema_shape() {
    let s = learning_schemas("learning_list_facets");
    assert_eq!(s.namespace, "learning");
    assert_eq!(s.function, "list_facets");
    assert!(s.inputs.iter().any(|f| f.name == "class" && !f.required));
    assert!(s.outputs.iter().any(|f| f.name == "facets"));
    assert!(s.outputs.iter().any(|f| f.name == "count"));
}

#[test]
fn get_facet_schema_shape() {
    let s = learning_schemas("learning_get_facet");
    assert_eq!(s.function, "get_facet");
    assert!(s.inputs.iter().any(|f| f.name == "class" && f.required));
    assert!(s.inputs.iter().any(|f| f.name == "key" && f.required));
}

#[test]
fn update_facet_schema_shape() {
    let s = learning_schemas("learning_update_facet");
    assert_eq!(s.function, "update_facet");
    assert!(s.inputs.iter().any(|f| f.name == "value" && f.required));
}

#[test]
fn pin_facet_schema_shape() {
    let s = learning_schemas("learning_pin_facet");
    assert_eq!(s.function, "pin_facet");
}

#[test]
fn unpin_facet_schema_shape() {
    let s = learning_schemas("learning_unpin_facet");
    assert_eq!(s.function, "unpin_facet");
}

#[test]
fn forget_facet_schema_shape() {
    let s = learning_schemas("learning_forget_facet");
    assert_eq!(s.function, "forget_facet");
}

#[test]
fn reset_cache_schema_shape() {
    let s = learning_schemas("learning_reset_cache");
    assert_eq!(s.function, "reset_cache");
    assert!(s.outputs.iter().any(|f| f.name == "deleted"));
    assert!(s.outputs.iter().any(|f| f.name == "pinned_preserved"));
}

// ── strict class validation (#6077) ───────────────────────────────────────────
//
// The class filter/argument is validated before the handler touches the store,
// so an unknown class is rejected without a bound memory guard — which is what
// lets these run as plain unit tests. On the old code `list_facets` with an
// unknown class returned `{facets:[],count:0}`; it must now be an error.

fn params_with_class(class: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("class".to_string(), Value::String(class.to_string()));
    m
}

#[tokio::test]
async fn list_facets_rejects_unknown_class() {
    let err = handle_list_facets(params_with_class("nonsense"))
        .await
        .expect_err("unknown class must be an error, not an empty result");
    assert!(err.contains("invalid class `nonsense`"), "got: {err}");
}

#[tokio::test]
async fn get_facet_rejects_unknown_class() {
    let mut params = params_with_class("nonsense");
    params.insert("key".to_string(), Value::String("verbosity".to_string()));
    let err = handle_get_facet(params)
        .await
        .expect_err("unknown class must be rejected before store access");
    assert!(err.contains("invalid class `nonsense`"), "got: {err}");
}

#[tokio::test]
async fn forget_facet_rejects_unknown_class() {
    let mut params = params_with_class("nonsense");
    params.insert("key".to_string(), Value::String("verbosity".to_string()));
    let err = handle_forget_facet(params)
        .await
        .expect_err("unknown class must be rejected before store access");
    assert!(err.contains("invalid class `nonsense`"), "got: {err}");
}

#[tokio::test]
async fn class_taking_handlers_keep_presence_check_before_value_check() {
    // A missing class is still the presence error, not the invalid-class error —
    // the new validation does not weaken the existing "missing required" contract.
    let err = handle_get_facet(Map::new())
        .await
        .expect_err("missing class must error");
    assert!(err.contains("missing required `class`"), "got: {err}");
}

// ── #6108: the forget_facet log must describe what actually happened ─────────

/// The line was previously built unconditionally, before the read that decides
/// whether there is anything to drop. A typo'd key therefore produced a log
/// asserting `state=dropped user_state=forgotten` for a row that was never
/// touched. The RPC contract itself is unchanged and deliberately idempotent —
/// `{"facet": null}`, no error — so the log is the only thing that can tell the
/// two outcomes apart.
#[test]
fn forget_facet_log_claims_a_drop_only_when_a_row_was_written() {
    let dropped = forget_facet_log("style/observed_key", true);
    assert_eq!(dropped.len(), 1);
    assert!(
        dropped[0].contains("state=dropped") && dropped[0].contains("user_state=forgotten"),
        "a real drop must still record the state change: {dropped:?}"
    );
    assert!(
        dropped[0].contains("style/observed_key"),
        "the log must name the key it dropped: {dropped:?}"
    );
}

#[test]
fn forget_facet_log_does_not_claim_a_drop_for_an_absent_key() {
    let absent = forget_facet_log("style/never_observed", false);
    assert_eq!(absent.len(), 1);
    assert!(
        !absent[0].contains("state=dropped"),
        "an absent key must not be logged as a state change — this is the #6108 \
         defect, where the claim was made before the row was read: {absent:?}"
    );
    assert!(
        !absent[0].contains("user_state=forgotten"),
        "nor may it claim the user forgot something that was never there: {absent:?}"
    );
    assert!(
        absent[0].contains("style/never_observed") && absent[0].contains("not present"),
        "it must still name the key and say plainly that nothing changed: {absent:?}"
    );
}
