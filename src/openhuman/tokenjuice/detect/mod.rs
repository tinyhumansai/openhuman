//! Content-kind detection + tool-name priors for the TokenJuice content router.

pub mod hint;
pub mod kind;

pub use hint::{extension_to_kind, mime_to_kind, prior_to_kind, tool_prior, ToolPrior};
pub use kind::{
    detect, detect_content_kind, looks_like_code, looks_like_diff, looks_like_html,
    looks_like_json, looks_like_json_array, parse_search_line,
};
