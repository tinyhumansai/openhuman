use super::*;
use serde_json::json;

fn map(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .cloned()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

#[test]
fn read_required_returns_value_when_present_and_well_typed() {
    let params = map(&[("name", json!("alice"))]);
    let value: String = read_required(&params, "name").expect("present");
    assert_eq!(value, "alice");
}

#[test]
fn read_required_errors_when_missing() {
    let params = map(&[]);
    let err = read_required::<String>(&params, "name").expect_err("missing");
    assert!(err.contains("missing required param 'name'"), "{err}");
}

#[test]
fn read_required_errors_when_wrong_shape() {
    let params = map(&[("name", json!(42))]);
    let err = read_required::<String>(&params, "name").expect_err("wrong shape");
    assert!(err.contains("invalid 'name'"), "{err}");
}

#[test]
fn read_optional_treats_missing_and_null_as_none() {
    let missing = map(&[]);
    assert_eq!(read_optional::<String>(&missing, "name").unwrap(), None);

    let null = map(&[("name", Value::Null)]);
    assert_eq!(read_optional::<String>(&null, "name").unwrap(), None);
}

#[test]
fn read_optional_returns_some_when_present() {
    let params = map(&[("count", json!(3))]);
    assert_eq!(read_optional::<u32>(&params, "count").unwrap(), Some(3));
}

// ── read_optional_u32 ────────────────────────────────────────────────────────

#[test]
fn read_optional_u32_treats_missing_and_null_as_none() {
    assert_eq!(read_optional_u32(&map(&[]), "page").unwrap(), None);
    let null = map(&[("page", Value::Null)]);
    assert_eq!(read_optional_u32(&null, "page").unwrap(), None);
}

#[test]
fn read_optional_u32_returns_some_for_a_u32() {
    let params = map(&[("page", json!(2))]);
    assert_eq!(read_optional_u32(&params, "page").unwrap(), Some(2));
}

#[test]
fn read_optional_u32_rejects_out_of_range_numbers_with_exact_copy() {
    for value in [json!(-1), json!(1.5), json!(u64::from(u32::MAX) + 1)] {
        let params = map(&[("page", value.clone())]);
        assert_eq!(
            read_optional_u32(&params, "page").unwrap_err(),
            "invalid 'page': expected u32",
            "value={value}"
        );
    }
}

#[test]
fn read_optional_u32_names_the_wrong_json_type() {
    let params = map(&[("page", json!("2"))]);
    assert_eq!(
        read_optional_u32(&params, "page").unwrap_err(),
        "invalid 'page': expected number, got string"
    );
}

// ── type_name ────────────────────────────────────────────────────────────────

#[test]
fn type_name_covers_every_json_variant() {
    assert_eq!(type_name(&Value::Null), "null");
    assert_eq!(type_name(&json!(true)), "bool");
    assert_eq!(type_name(&json!(1)), "number");
    assert_eq!(type_name(&json!("s")), "string");
    assert_eq!(type_name(&json!([])), "array");
    assert_eq!(type_name(&json!({})), "object");
}

// ── opt_trimmed_str_arg ──────────────────────────────────────────────────────

#[test]
fn opt_trimmed_str_arg_trims_and_drops_blank_or_non_string() {
    let args = json!({"a": "  x  ", "blank": "   ", "n": 3});
    assert_eq!(opt_trimmed_str_arg(&args, "a").as_deref(), Some("x"));
    assert_eq!(opt_trimmed_str_arg(&args, "blank"), None);
    assert_eq!(opt_trimmed_str_arg(&args, "n"), None);
    assert_eq!(opt_trimmed_str_arg(&args, "missing"), None);
}

// ── Outcome conversion ───────────────────────────────────────────────────────

#[test]
fn to_json_is_bare_without_logs_and_wrapped_with_logs() {
    assert_eq!(
        to_json(Outcome::new(json!({"k": 1}), vec![])).unwrap(),
        json!({"k": 1})
    );
    assert_eq!(
        to_json(Outcome::single_log(json!(1), "did it")).unwrap(),
        json!({"result": 1, "logs": ["did it"]})
    );
}

#[test]
fn value_to_json_never_adds_a_log_envelope() {
    assert_eq!(value_to_json(json!([1, 2])).unwrap(), json!([1, 2]));
}

#[test]
fn outcome_value_to_json_drops_logs() {
    let outcome = Outcome::single_log(json!({"k": 1}), "ignored");
    assert_eq!(outcome_value_to_json(outcome, "").unwrap(), json!({"k": 1}));
}

/// Serializes as an error so the serde-failure copy can be asserted.
struct FailsToSerialize;

impl serde::Serialize for FailsToSerialize {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("boom"))
    }
}

#[test]
fn serialize_failures_keep_each_callers_error_copy() {
    assert_eq!(
        to_json(Outcome::new(FailsToSerialize, vec![])).unwrap_err(),
        "boom"
    );
    assert_eq!(value_to_json(FailsToSerialize).unwrap_err(), "boom");
    assert_eq!(
        outcome_value_to_json(Outcome::new(FailsToSerialize, vec![]), "").unwrap_err(),
        "boom"
    );
    assert_eq!(
        outcome_value_to_json(Outcome::new(FailsToSerialize, vec![]), "serialize error: ")
            .unwrap_err(),
        "serialize error: boom"
    );
}

#[test]
fn new_correlation_id_is_eight_hex_chars() {
    let id = new_correlation_id();
    assert_eq!(id.len(), 8);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
    assert_ne!(id, new_correlation_id());
}

// ── FieldSchema builders ─────────────────────────────────────────────────────

#[test]
fn field_schema_builders_set_type_and_requiredness() {
    let json = json_output("result", "c");
    assert_eq!(
        (json.name, json.comment, json.required),
        ("result", "c", true)
    );
    assert!(matches!(json.ty, TypeSchema::Json));

    let req = required_string("id", "c");
    assert!(req.required);
    assert!(matches!(req.ty, TypeSchema::String));

    let opt = optional_string("q", "c");
    assert!(!opt.required);
    assert!(
        matches!(opt.ty, TypeSchema::Option(ref inner) if matches!(**inner, TypeSchema::String))
    );
}
