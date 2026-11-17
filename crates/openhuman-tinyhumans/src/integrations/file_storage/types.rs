//! Shared types for the `file_storage` agent tools.
//!
//! These mirror the backend's `file_storage` integration contract
//! (`/agent-integrations/file-storage/*`) — see the file-storage API contract.
//! Every response arrives inside the standard `{ success, data, error }`
//! envelope handled by `IntegrationClient`; these structs are the `data`
//! payloads.

use serde::Deserialize;
use serde_json::json;

/// Metadata for a stored file as returned by list / metadata / visibility
/// endpoints.
#[derive(Debug, Clone, Deserialize)]
pub struct FileMeta {
    #[serde(rename = "fileId")]
    pub file_id: String,
    pub filename: String,
    #[serde(default)]
    pub size: u64,
    #[serde(rename = "contentType", default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub visibility: String,
    #[serde(rename = "expiresAt", default)]
    pub expires_at: Option<String>,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
    /// Stable public URL — present only when `visibility == "public"`.
    #[serde(rename = "publicUrl", default)]
    pub public_url: Option<String>,
}

impl FileMeta {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "file_id": self.file_id,
            "filename": self.filename,
            "size": self.size,
            "content_type": self.content_type,
            "visibility": self.visibility,
            "expires_at": self.expires_at,
            "created_at": self.created_at,
            "public_url": self.public_url,
        })
    }
}

/// `POST /files` (multipart upload) response.
#[derive(Debug, Clone, Deserialize)]
pub struct UploadResponse {
    #[serde(rename = "fileId")]
    pub file_id: String,
    pub filename: String,
    #[serde(default)]
    pub size: u64,
    #[serde(rename = "contentType", default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub visibility: String,
    #[serde(rename = "expiresAt", default)]
    pub expires_at: Option<String>,
    #[serde(rename = "publicUrl", default)]
    pub public_url: Option<String>,
    #[serde(rename = "costUsd", default)]
    pub cost_usd: f64,
}

/// Storage usage summary embedded in the list response.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct StorageUsage {
    #[serde(rename = "usedBytes", default)]
    pub used_bytes: u64,
    #[serde(rename = "limitBytes", default)]
    pub limit_bytes: u64,
}

/// `GET /files` response.
#[derive(Debug, Clone, Deserialize)]
pub struct ListFilesResponse {
    #[serde(default)]
    pub files: Vec<FileMeta>,
    #[serde(rename = "nextCursor", default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub usage: StorageUsage,
}

/// `POST /files/:fileId/link` response — a short-lived presigned GET URL.
#[derive(Debug, Clone, Deserialize)]
pub struct LinkResponse {
    pub url: String,
    #[serde(rename = "expiresAt", default)]
    pub expires_at: Option<String>,
    #[serde(rename = "costUsd", default)]
    pub cost_usd: f64,
}

/// `DELETE /files/:fileId` response.
#[derive(Debug, Clone, Deserialize)]
pub struct DeleteResponse {
    #[serde(default)]
    pub deleted: bool,
}
