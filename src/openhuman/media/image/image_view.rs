//! Contract for the hosted `view_image` tool.
//!
//! `view_image` bridges local image files into model-visible image content. It
//! is distinct from `image_info`: metadata extraction can stay textual, while
//! `view_image` asks the runtime to load pixels into the conversation context.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{ImagePermission, ImageToolSpec};

/// Stable model-facing tool name.
pub const IMAGE_VIEW_TOOL_NAME: &str = "view_image";

/// Requested image-detail level for model-visible inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageDetail {
    Auto,
    High,
    Original,
}

impl ImageDetail {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::High => "high",
            Self::Original => "original",
        }
    }
}

/// Build the hosted `view_image` model-facing contract.
pub fn image_view_spec() -> ImageToolSpec {
    ImageToolSpec {
        name: IMAGE_VIEW_TOOL_NAME.to_string(),
        description: "Load a local image file into model-visible image context for inspection, OCR, UI review, or visual reasoning.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Local image path, absolute or relative to the approved workspace."
                },
                "detail": {
                    "type": "string",
                    "enum": ["auto", "high", "original"],
                    "default": ImageDetail::Auto.as_str(),
                    "description": "Inspection detail. Use original only when full resolution is necessary."
                }
            },
            "required": ["path"]
        }),
        permission: ImagePermission::ReadOnly,
        model_visible_image_output: true,
        writes_files: false,
    }
}

#[cfg(test)]
#[path = "image_view_tests.rs"]
mod tests;
