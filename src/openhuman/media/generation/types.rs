//! Shared types for the `media_generation` agent tools.
//!
//! These mirror the backend's standardized `media_generation` contract
//! (`/agent-integrations/media-generation/*`) — see
//! `backend/docs/media-generation.md`. The backend normalizes GMI's per-model
//! payload/outcome shapes; the core only depends on this stable envelope.

use serde::Deserialize;

/// A single generated artifact as returned by the backend. The `url` is an
/// expiring signed URL — the core downloads + persists it locally.
#[derive(Debug, Clone, Deserialize)]
pub struct MediaItem {
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(rename = "thumbnailUrl", default)]
    pub thumbnail_url: Option<String>,
}

/// Standardized media-generation response envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct MediaResponse {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub status: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub media: Vec<MediaItem>,
    #[serde(rename = "costUsd", default)]
    pub cost_usd: f64,
}

impl MediaResponse {
    pub fn is_success(&self) -> bool {
        self.status.eq_ignore_ascii_case("success")
    }

    pub fn is_failed(&self) -> bool {
        self.status.eq_ignore_ascii_case("failed")
    }

    pub fn is_terminal(&self) -> bool {
        self.is_success() || self.is_failed()
    }
}
