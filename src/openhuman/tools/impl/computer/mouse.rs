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
    if !(0..=MAX_COORD).contains(&value) {
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
#[path = "mouse_tests.rs"]
mod tests;
