//! POINT tag parser and multi-monitor coordinate mapping.
//!
//! The companion LLM embeds `[POINT:x,y:label:screenN]` tags in its
//! response text (Clicky convention). This module extracts those tags,
//! maps screen-relative coordinates to absolute desktop coordinates
//! using monitor geometry, and strips the tags from the display text.

use log::debug;
use serde::{Deserialize, Serialize};

const LOG_PREFIX: &str = "[companion_pointing]";

/// A parsed point target from the LLM response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PointTarget {
    /// Screen-relative X coordinate (as emitted by the LLM).
    pub x: f64,
    /// Screen-relative Y coordinate.
    pub y: f64,
    /// Human-readable label for the target element.
    pub label: String,
    /// Zero-based screen index.
    pub screen_index: usize,
    /// Absolute desktop X after multi-monitor mapping.
    pub absolute_x: f64,
    /// Absolute desktop Y after multi-monitor mapping.
    pub absolute_y: f64,
}

/// Monitor geometry used for coordinate mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenGeometry {
    /// Zero-based index.
    pub index: usize,
    /// Left edge in absolute desktop coordinates.
    pub x: f64,
    /// Top edge in absolute desktop coordinates.
    pub y: f64,
    /// Width in points.
    pub width: f64,
    /// Height in points.
    pub height: f64,
}

/// Result of parsing POINT tags from an LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointingParseResult {
    /// Extracted point targets with mapped coordinates.
    pub targets: Vec<PointTarget>,
    /// The response text with POINT tags stripped out.
    pub clean_text: String,
}

/// Lazily compiled POINT-tag regex.
fn point_tag_regex() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"\[POINT:(-?\d+(?:\.\d+)?),(-?\d+(?:\.\d+)?):([^:\]]+):screen(\d+)\]")
            .expect("companion POINT tag regex is static and valid")
    })
}

/// Parse `[POINT:x,y:label:screenN]` tags from LLM response text and map
/// coordinates to absolute desktop positions using the given screen geometry.
pub fn parse_and_map(text: &str, screens: &[ScreenGeometry]) -> PointingParseResult {
    let re = point_tag_regex();

    let mut targets = Vec::new();
    let clean_text = re
        .replace_all(text, |caps: &regex::Captures| {
            let x: f64 = caps[1].parse().unwrap_or(0.0);
            let y: f64 = caps[2].parse().unwrap_or(0.0);
            let label = caps[3].trim().to_string();
            let screen_index: usize = caps[4].parse().unwrap_or(0);

            let (abs_x, abs_y) = map_to_absolute(x, y, screen_index, screens);

            debug!(
                "{LOG_PREFIX} parsed target: ({x},{y}) label=\"{label}\" screen{screen_index} -> abs({abs_x},{abs_y})"
            );

            targets.push(PointTarget {
                x,
                y,
                label,
                screen_index,
                absolute_x: abs_x,
                absolute_y: abs_y,
            });

            // Replace the tag with empty string in the display text.
            String::new()
        })
        .to_string();

    PointingParseResult {
        targets,
        clean_text: clean_text.trim().to_string(),
    }
}

/// Map screen-relative coordinates to absolute desktop coordinates.
///
/// If the screen index is out of range, falls back to screen 0 (primary).
/// Coordinates are clamped to screen bounds.
fn map_to_absolute(x: f64, y: f64, screen_index: usize, screens: &[ScreenGeometry]) -> (f64, f64) {
    if screens.is_empty() {
        return (x, y);
    }

    let screen = screens
        .iter()
        .find(|s| s.index == screen_index)
        .or_else(|| screens.first())
        .unwrap();

    let clamped_x = x.clamp(0.0, screen.width);
    let clamped_y = y.clamp(0.0, screen.height);

    (screen.x + clamped_x, screen.y + clamped_y)
}

#[cfg(test)]
#[path = "pointing_tests.rs"]
mod tests;
