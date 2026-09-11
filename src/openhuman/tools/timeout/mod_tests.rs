use super::*;

#[test]
fn default_when_env_missing() {
    assert_eq!(parse_tool_timeout_secs(None), DEFAULT_TIMEOUT_SECS);
}

#[test]
fn default_when_value_not_numeric() {
    assert_eq!(
        parse_tool_timeout_secs(Some("not-a-number")),
        DEFAULT_TIMEOUT_SECS
    );
    assert_eq!(parse_tool_timeout_secs(Some("")), DEFAULT_TIMEOUT_SECS);
    assert_eq!(parse_tool_timeout_secs(Some("12x")), DEFAULT_TIMEOUT_SECS);
}

#[test]
fn default_when_value_zero() {
    // 0 seconds would disable the timeout — reject and fall back.
    assert_eq!(parse_tool_timeout_secs(Some("0")), DEFAULT_TIMEOUT_SECS);
}

#[test]
fn default_when_value_above_max() {
    assert_eq!(parse_tool_timeout_secs(Some("3601")), DEFAULT_TIMEOUT_SECS);
    assert_eq!(
        parse_tool_timeout_secs(Some("99999999999")),
        DEFAULT_TIMEOUT_SECS
    );
}

#[test]
fn default_when_value_negative_or_signed() {
    // Negative values fail u64 parse and fall back to default.
    assert_eq!(parse_tool_timeout_secs(Some("-5")), DEFAULT_TIMEOUT_SECS);
}

#[test]
fn accepts_valid_values_at_boundaries() {
    assert_eq!(parse_tool_timeout_secs(Some("1")), MIN_TIMEOUT_SECS);
    assert_eq!(parse_tool_timeout_secs(Some("3600")), MAX_TIMEOUT_SECS);
}

#[test]
fn accepts_valid_midrange_value() {
    assert_eq!(parse_tool_timeout_secs(Some("300")), 300);
}

#[test]
fn env_override_takes_precedence_over_config() {
    // When the env var holds a valid value it wins over the config value.
    assert_eq!(resolve_effective(300, Some("600")), 600);
}

#[test]
fn config_value_used_when_env_absent_or_invalid() {
    // No env → config drives the effective value (bounded).
    assert_eq!(resolve_effective(300, None), 300);
    // Present-but-invalid env (non-numeric / 0 / out of range) is ignored,
    // so the config value still applies.
    assert_eq!(resolve_effective(300, Some("nonsense")), 300);
    assert_eq!(resolve_effective(300, Some("0")), 300);
    assert_eq!(resolve_effective(300, Some("4000")), 300);
}

#[test]
fn config_value_is_bounded() {
    // An out-of-range config value falls back to the default rather than
    // being applied verbatim.
    assert_eq!(resolve_effective(0, None), DEFAULT_TIMEOUT_SECS);
    assert_eq!(resolve_effective(99_999, None), DEFAULT_TIMEOUT_SECS);
}

#[test]
fn explicit_call_timeout_unbounded_when_absent_or_disabled() {
    // No request, or an explicit 0, means "run unbounded" (None).
    assert_eq!(explicit_call_timeout_secs(None, MAX_TIMEOUT_SECS), None);
    assert_eq!(explicit_call_timeout_secs(Some(0), MAX_TIMEOUT_SECS), None);
}

#[test]
fn explicit_call_timeout_enforces_and_clamps_request() {
    // An in-range request is enforced verbatim.
    assert_eq!(
        explicit_call_timeout_secs(Some(900), MAX_TIMEOUT_SECS),
        Some(900)
    );
    // Below the floor clamps up to MIN.
    assert_eq!(
        explicit_call_timeout_secs(Some(0), MAX_TIMEOUT_SECS),
        None,
        "0 disables rather than clamping to MIN"
    );
    // Above the cap clamps down to the cap.
    assert_eq!(
        explicit_call_timeout_secs(Some(99_999), MAX_TIMEOUT_SECS),
        Some(MAX_TIMEOUT_SECS)
    );
    // A tool with a tighter own ceiling (node/npm at 1800) clamps to it.
    assert_eq!(explicit_call_timeout_secs(Some(99_999), 1800), Some(1800));
    assert_eq!(explicit_call_timeout_secs(Some(600), 1800), Some(600));
}

#[test]
fn explicit_call_timeout_duration_matches_secs() {
    assert_eq!(
        explicit_call_timeout_duration(Some(900), MAX_TIMEOUT_SECS),
        Some(Duration::from_secs(900))
    );
    assert_eq!(explicit_call_timeout_duration(None, MAX_TIMEOUT_SECS), None);
}

#[test]
fn env_override_from_rejects_invalid() {
    assert_eq!(env_override_from(None), None);
    assert_eq!(env_override_from(Some("")), None);
    assert_eq!(env_override_from(Some("0")), None);
    assert_eq!(env_override_from(Some("abc")), None);
    assert_eq!(env_override_from(Some("3601")), None);
    assert_eq!(env_override_from(Some("120")), Some(120));
}
