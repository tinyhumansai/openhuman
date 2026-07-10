//! Media generation domain — agent tools for image/video generation backed by
//! GMI via the OpenHuman backend's `media_generation` provider.
//!
//! The backend (`/agent-integrations/media-generation/*`) owns provider keys,
//! billing, and the standardized contract; these tools submit a request, block
//! with progress until it completes, download the resulting media into the
//! agent's `generated-media/` root, and return local file paths.

pub mod download;
pub mod tools;
pub mod types;

pub use tools::{
    build_media_tools, MediaGenerateImageTool, MediaGenerateVideoTool, MediaListModelsTool,
};
