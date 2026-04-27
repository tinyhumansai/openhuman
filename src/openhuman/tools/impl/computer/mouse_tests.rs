use super::*;

fn make_tool() -> MouseTool {
    MouseTool::new(Arc::new(SecurityPolicy::default()))
}

#[test]
fn schema_has_required_action() {
    let tool = make_tool();
    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["action"]));
}

#[test]
fn schema_enumerates_actions() {
    let tool = make_tool();
    let schema = tool.parameters_schema();
    let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
    let names: Vec<&str> = actions.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(names.contains(&"move"));
    assert!(names.contains(&"click"));
    assert!(names.contains(&"double_click"));
    assert!(names.contains(&"drag"));
    assert!(names.contains(&"scroll"));
}

#[test]
fn permission_is_dangerous() {
    let tool = make_tool();
    assert_eq!(tool.permission_level(), PermissionLevel::Dangerous);
}

#[test]
fn name_is_mouse() {
    assert_eq!(make_tool().name(), "mouse");
}

#[test]
fn coord_validation_rejects_negative() {
    assert!(validate_coord("x", -1).is_err());
}

#[test]
fn coord_validation_rejects_overflow() {
    assert!(validate_coord("x", MAX_COORD + 1).is_err());
}

#[test]
fn coord_validation_accepts_zero() {
    assert!(validate_coord("x", 0).is_ok());
}

#[test]
fn coord_validation_accepts_max() {
    assert!(validate_coord("x", MAX_COORD).is_ok());
}

#[test]
fn parse_button_defaults_to_left() {
    assert_eq!(parse_button(&json!({})).unwrap(), Button::Left);
    assert_eq!(
        parse_button(&json!({"button": "left"})).unwrap(),
        Button::Left
    );
}

#[test]
fn parse_button_right() {
    assert_eq!(
        parse_button(&json!({"button": "right"})).unwrap(),
        Button::Right
    );
}

#[test]
fn parse_button_middle() {
    assert_eq!(
        parse_button(&json!({"button": "middle"})).unwrap(),
        Button::Middle
    );
}

#[test]
fn parse_button_unknown_returns_error() {
    assert!(parse_button(&json!({"button": "laser"})).is_err());
}

#[test]
fn parse_button_non_string_returns_error() {
    assert!(parse_button(&json!({"button": 42})).is_err());
}

#[tokio::test]
async fn missing_action_returns_error() {
    let tool = make_tool();
    let result = tool.execute(json!({})).await;
    assert!(result.is_err() || result.unwrap().is_error);
}

#[tokio::test]
async fn unknown_action_returns_error() {
    let tool = make_tool();
    let result = tool.execute(json!({"action": "teleport"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Unknown mouse action"));
}

#[tokio::test]
async fn click_missing_coords_returns_error() {
    let tool = make_tool();
    let result = tool.execute(json!({"action": "click"})).await;
    // Should fail with missing x/y
    assert!(result.is_err() || result.unwrap().is_error);
}

#[tokio::test]
async fn scroll_zero_both_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "scroll", "scroll_x": 0, "scroll_y": 0}))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn drag_missing_start_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "drag", "x": 100, "y": 100}))
        .await;
    assert!(result.is_err() || result.unwrap().is_error);
}

// ── require_xy: individually missing parameters ───────────────────────────

#[test]
fn require_xy_missing_x_returns_error() {
    let result = require_xy(&json!({"y": 100}));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("'x'"));
}

#[test]
fn require_xy_missing_y_returns_error() {
    let result = require_xy(&json!({"x": 100}));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("'y'"));
}

#[test]
fn require_xy_out_of_range_x_returns_error() {
    let result = require_xy(&json!({"x": -1, "y": 0}));
    assert!(result.is_err());
}

#[test]
fn require_xy_out_of_range_y_returns_error() {
    let result = require_xy(&json!({"x": 0, "y": MAX_COORD + 1}));
    assert!(result.is_err());
}

#[test]
fn require_xy_valid_returns_tuple() {
    let (x, y) = require_xy(&json!({"x": 100, "y": 200})).unwrap();
    assert_eq!(x, 100);
    assert_eq!(y, 200);
}

// ── security: read-only autonomy blocks all actions ───────────────────────

#[tokio::test]
async fn blocked_in_read_only_mode() {
    use crate::openhuman::security::AutonomyLevel;
    let readonly = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = MouseTool::new(readonly);
    let result = tool
        .execute(json!({"action": "move", "x": 10, "y": 10}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only"));
}

// ── security: rate limit exceeded blocks action ───────────────────────────

#[tokio::test]
async fn blocked_when_rate_limited() {
    let limited = Arc::new(SecurityPolicy {
        max_actions_per_hour: 0,
        ..SecurityPolicy::default()
    });
    let tool = MouseTool::new(limited);
    let result = tool
        .execute(json!({"action": "move", "x": 10, "y": 10}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("rate limit"));
}

// ── scroll with only one axis ──────────────────────────────────────────────

#[tokio::test]
async fn scroll_only_x_is_valid_input() {
    let tool = make_tool();
    // Should bypass the zero-check and attempt hardware access. Whether
    // hardware access succeeds is environment-dependent, but neither
    // branch may surface the "non-zero" validation error.
    let result = tool
        .execute(json!({"action": "scroll", "scroll_x": 3, "scroll_y": 0}))
        .await;
    match result {
        Ok(r) => assert!(
            !r.output().contains("non-zero"),
            "single-axis scroll should not trigger zero guard (got: {})",
            r.output()
        ),
        Err(e) => assert!(
            !e.to_string().contains("non-zero"),
            "single-axis scroll should not trigger zero guard (got Err: {e})"
        ),
    }
}

#[tokio::test]
async fn scroll_only_y_is_valid_input() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "scroll", "scroll_x": 0, "scroll_y": -5}))
        .await;
    match result {
        Ok(r) => assert!(
            !r.output().contains("non-zero"),
            "single-axis scroll should not trigger zero guard (got: {})",
            r.output()
        ),
        Err(e) => assert!(
            !e.to_string().contains("non-zero"),
            "single-axis scroll should not trigger zero guard (got Err: {e})"
        ),
    }
}

// ── drag: missing end coords error ───────────────────────────────────────

#[tokio::test]
async fn drag_missing_end_coords_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "drag", "start_x": 10, "start_y": 20}))
        .await;
    assert!(result.is_err() || result.unwrap().is_error);
}

// ── drag: out-of-range start coord ────────────────────────────────────────

#[tokio::test]
async fn drag_out_of_range_start_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({
            "action": "drag",
            "start_x": -1,
            "start_y": 0,
            "x": 100,
            "y": 100
        }))
        .await;
    assert!(result.is_err() || result.unwrap().is_error);
}

// ── tool description ──────────────────────────────────────────────────────

#[test]
fn description_is_non_empty() {
    let tool = make_tool();
    assert!(!tool.description().is_empty());
    assert!(tool.description().contains("mouse"));
}

// ── tool spec ─────────────────────────────────────────────────────────────

#[test]
fn spec_roundtrip() {
    let tool = make_tool();
    let spec = tool.spec();
    assert_eq!(spec.name, "mouse");
    assert!(spec.parameters.is_object());
}
