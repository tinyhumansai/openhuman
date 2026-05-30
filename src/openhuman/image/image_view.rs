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
mod tests {
    use super::*;

    #[test]
    fn view_image_schema_requires_path_and_marks_model_visible_output() {
        let spec = image_view_spec();

        assert_eq!(spec.name, "view_image");
        assert!(spec.description.contains("model-visible image context"));
        assert_eq!(spec.permission, ImagePermission::ReadOnly);
        assert!(spec.model_visible_image_output);
        assert!(!spec.writes_files);
        assert_eq!(spec.parameters["required"], serde_json::json!(["path"]));
        assert_eq!(spec.parameters["properties"]["detail"]["default"], "auto");
    }

    #[test]
    fn detail_names_match_prompt_contract() {
        assert_eq!(ImageDetail::Auto.as_str(), "auto");
        assert_eq!(ImageDetail::High.as_str(), "high");
        assert_eq!(ImageDetail::Original.as_str(), "original");

        assert_eq!(
            serde_json::to_value(ImageDetail::Original).unwrap(),
            serde_json::json!("original")
        );
        assert_eq!(
            serde_json::from_value::<ImageDetail>(serde_json::json!("high")).unwrap(),
            ImageDetail::High
        );
    }

    #[test]
    fn view_image_schema_lists_supported_detail_levels() {
        let spec = image_view_spec();

        assert_eq!(
            spec.parameters["properties"]["detail"]["enum"],
            serde_json::json!(["auto", "high", "original"])
        );
        assert_eq!(spec.parameters["properties"]["path"]["type"], "string");
    }
}
