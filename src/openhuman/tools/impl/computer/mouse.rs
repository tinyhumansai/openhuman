//! Native mouse control tool using enigo.
//!
//! Provides absolute-coordinate mouse movement, clicking, double-clicking,
//! dragging, and scrolling via platform-native APIs (Core Graphics on macOS,
//! SendInput on Windows, X11/libxdo on Linux).

use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use enigo::{Button, Coordinate, Direction, Enigo, Mouse, Settings};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Coordinate safety bound — reject values outside this range.
const MAX_COORD: i64 = 32768;

pub struct MouseTool {
    security: Arc<SecurityPolicy>,
}

impl MouseTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

fn parse_button(args: &Value) -> anyhow::Result<Button> {
    match args.get("button") {
        None => Ok(Button::Left),
        Some(v) => match v.as_str() {
            Some("left") => Ok(Button::Left),
            Some("right") => Ok(Button::Right),
            Some("middle") => Ok(Button::Middle),
            Some(other) => {
                anyhow::bail!("Invalid mouse button '{other}'. Use: left, right, middle")
            }
            None => anyhow::bail!("'button' must be a string, got {v}"),
        },
    }
}

fn require_xy(args: &Value) -> anyhow::Result<(i32, i32)> {
    let x = args
        .get("x")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("Missing required 'x' parameter"))?;
    let y = args
        .get("y")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("Missing required 'y' parameter"))?;
    validate_coord("x", x)?;
    validate_coord("y", y)?;
    Ok((x as i32, y as i32))
}

fn validate_coord(name: &str, value: i64) -> anyhow::Result<()> {
    if value < 0 || value > MAX_COORD {
        anyhow::bail!("'{name}' coordinate {value} is out of range (0..{MAX_COORD})");
    }
    Ok(())
}

#[async_trait]
impl Tool for MouseTool {
    fn name(&self) -> &str {
        "mouse"
    }

    fn description(&self) -> &str {
        concat!(
            "Control the mouse cursor natively. Actions: move (reposition cursor), ",
            "click (move + click), double_click, drag (press at start, release at end), ",
            "scroll (vertical/horizontal). All coordinates are absolute screen pixels."
        )
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["move", "click", "double_click", "drag", "scroll"],
                    "description": "Mouse action to perform"
                },
                "x": {
                    "type": "integer",
                    "description": "Target X coordinate (absolute screen pixels). Required for move, click, double_click."
                },
                "y": {
                    "type": "integer",
                    "description": "Target Y coordinate (absolute screen pixels). Required for move, click, double_click."
                },
                "button": {
                    "type": "string",
                    "enum": ["left", "right", "middle"],
                    "description": "Mouse button for click/double_click/drag. Default: left."
                },
                "start_x": {
                    "type": "integer",
                    "description": "Drag start X coordinate (absolute). Required for drag."
                },
                "start_y": {
                    "type": "integer",
                    "description": "Drag start Y coordinate (absolute). Required for drag."
                },
                "scroll_x": {
                    "type": "integer",
                    "description": "Horizontal scroll amount (positive = right, negative = left). For scroll action."
                },
                "scroll_y": {
                    "type": "integer",
                    "description": "Vertical scroll amount (positive = down, negative = up). For scroll action."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        if !self.security.can_act() {
            debug!(tool = "mouse", "[computer] blocked: autonomy is read-only");
            return Ok(ToolResult::error("Action blocked: autonomy is read-only"));
        }
        if !self.security.record_action() {
            debug!(tool = "mouse", "[computer] blocked: rate limit exceeded");
            return Ok(ToolResult::error("Action blocked: rate limit exceeded"));
        }

        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        debug!(
            tool = "mouse",
            action = action,
            "[computer] mouse action requested"
        );

        match action {
            "move" => {
                let (x, y) = require_xy(&args)?;
                tokio::task::spawn_blocking(move || {
                    let mut enigo = Enigo::new(&Settings::default())
                        .map_err(|e| anyhow::anyhow!("Failed to create enigo instance: {e}"))?;
                    enigo
                        .move_mouse(x, y, Coordinate::Abs)
                        .map_err(|e| anyhow::anyhow!("move_mouse failed: {e}"))?;
                    info!(
                        tool = "mouse",
                        action = "move",
                        x = x,
                        y = y,
                        "[computer] cursor moved"
                    );
                    Ok(ToolResult::success(format!("Moved cursor to ({x}, {y})")))
                })
                .await?
            }

            "click" => {
                let (x, y) = require_xy(&args)?;
                let button = parse_button(&args)?;
                tokio::task::spawn_blocking(move || {
                    let mut enigo = Enigo::new(&Settings::default())
                        .map_err(|e| anyhow::anyhow!("Failed to create enigo instance: {e}"))?;
                    enigo
                        .move_mouse(x, y, Coordinate::Abs)
                        .map_err(|e| anyhow::anyhow!("move_mouse failed: {e}"))?;
                    enigo
                        .button(button, Direction::Click)
                        .map_err(|e| anyhow::anyhow!("button click failed: {e}"))?;
                    info!(
                        tool = "mouse", action = "click",
                        x = x, y = y, button = ?button,
                        "[computer] clicked"
                    );
                    Ok(ToolResult::success(format!(
                        "Clicked {button:?} at ({x}, {y})"
                    )))
                })
                .await?
            }

            "double_click" => {
                let (x, y) = require_xy(&args)?;
                let button = parse_button(&args)?;
                tokio::task::spawn_blocking(move || {
                    let mut enigo = Enigo::new(&Settings::default())
                        .map_err(|e| anyhow::anyhow!("Failed to create enigo instance: {e}"))?;
                    enigo
                        .move_mouse(x, y, Coordinate::Abs)
                        .map_err(|e| anyhow::anyhow!("move_mouse failed: {e}"))?;
                    enigo
                        .button(button, Direction::Click)
                        .map_err(|e| anyhow::anyhow!("button click failed: {e}"))?;
                    enigo
                        .button(button, Direction::Click)
                        .map_err(|e| anyhow::anyhow!("button click failed: {e}"))?;
                    info!(
                        tool = "mouse", action = "double_click",
                        x = x, y = y, button = ?button,
                        "[computer] double-clicked"
                    );
                    Ok(ToolResult::success(format!(
                        "Double-clicked {button:?} at ({x}, {y})"
                    )))
                })
                .await?
            }

            "drag" => {
                let start_x = args
                    .get("start_x")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow::anyhow!("Missing 'start_x' for drag"))?;
                let start_y = args
                    .get("start_y")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow::anyhow!("Missing 'start_y' for drag"))?;
                validate_coord("start_x", start_x)?;
                validate_coord("start_y", start_y)?;
                let (end_x, end_y) = require_xy(&args)?;
                let button = parse_button(&args)?;
                let sx = start_x as i32;
                let sy = start_y as i32;

                tokio::task::spawn_blocking(move || {
                    let mut enigo = Enigo::new(&Settings::default())
                        .map_err(|e| anyhow::anyhow!("Failed to create enigo instance: {e}"))?;
                    enigo
                        .move_mouse(sx, sy, Coordinate::Abs)
                        .map_err(|e| anyhow::anyhow!("move_mouse (start) failed: {e}"))?;
                    enigo
                        .button(button, Direction::Press)
                        .map_err(|e| anyhow::anyhow!("button press failed: {e}"))?;

                    // After press succeeds, guarantee release even on error.
                    let drag_result: Result<(), anyhow::Error> = (|| {
                        enigo
                            .move_mouse(end_x, end_y, Coordinate::Abs)
                            .map_err(|e| anyhow::anyhow!("move_mouse (end) failed: {e}"))?;
                        Ok(())
                    })();

                    // Always release — best-effort cleanup.
                    if let Err(e) = enigo.button(button, Direction::Release) {
                        warn!(
                            tool = "mouse",
                            button = ?button,
                            error = %e,
                            "[computer] best-effort button release failed during drag cleanup"
                        );
                    }

                    // Propagate the drag error if the move failed.
                    drag_result?;

                    info!(
                        tool = "mouse", action = "drag",
                        start_x = sx, start_y = sy,
                        end_x = end_x, end_y = end_y, button = ?button,
                        "[computer] dragged"
                    );
                    Ok(ToolResult::success(format!(
                        "Dragged {button:?} from ({sx}, {sy}) to ({end_x}, {end_y})"
                    )))
                })
                .await?
            }

            "scroll" => {
                let raw_x = args.get("scroll_x").and_then(Value::as_i64).unwrap_or(0);
                let raw_y = args.get("scroll_y").and_then(Value::as_i64).unwrap_or(0);

                let scroll_x = i32::try_from(raw_x).map_err(|_| {
                    anyhow::anyhow!(
                        "'scroll_x' value {raw_x} is out of i32 range ({min}..={max})",
                        min = i32::MIN,
                        max = i32::MAX
                    )
                })?;
                let scroll_y = i32::try_from(raw_y).map_err(|_| {
                    anyhow::anyhow!(
                        "'scroll_y' value {raw_y} is out of i32 range ({min}..={max})",
                        min = i32::MIN,
                        max = i32::MAX
                    )
                })?;

                if scroll_x == 0 && scroll_y == 0 {
                    return Ok(ToolResult::error(
                        "At least one of 'scroll_x' or 'scroll_y' must be non-zero",
                    ));
                }

                tokio::task::spawn_blocking(move || {
                    let mut enigo = Enigo::new(&Settings::default())
                        .map_err(|e| anyhow::anyhow!("Failed to create enigo instance: {e}"))?;
                    if scroll_y != 0 {
                        enigo
                            .scroll(scroll_y, enigo::Axis::Vertical)
                            .map_err(|e| anyhow::anyhow!("vertical scroll failed: {e}"))?;
                    }
                    if scroll_x != 0 {
                        enigo
                            .scroll(scroll_x, enigo::Axis::Horizontal)
                            .map_err(|e| anyhow::anyhow!("horizontal scroll failed: {e}"))?;
                    }
                    info!(
                        tool = "mouse",
                        action = "scroll",
                        scroll_x = scroll_x,
                        scroll_y = scroll_y,
                        "[computer] scrolled"
                    );
                    Ok(ToolResult::success(format!(
                        "Scrolled (x={scroll_x}, y={scroll_y})"
                    )))
                })
                .await?
            }

            other => Ok(ToolResult::error(format!(
                "Unknown mouse action '{other}'. Use: move, click, double_click, drag, scroll"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
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
}
