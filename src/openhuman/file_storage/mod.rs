//! File storage domain — agent tools for managed cloud file storage backed by
//! the OpenHuman backend's `file_storage` provider
//! (`/agent-integrations/file-storage/*`, S3 under the hood).
//!
//! The backend owns the bucket, billing (S3 rates + margin, charged via the
//! standard integration billing flow), per-user quota (1 GiB), and TTL
//! enforcement (7 days free / up to 1 year paid); these tools upload from and
//! download into the agent's action dir and expose list / link / visibility /
//! delete operations.

pub mod tools;
pub mod types;

pub use tools::{
    build_file_storage_tools, StorageDeleteFileTool, StorageDownloadFileTool, StorageGetLinkTool,
    StorageListFilesTool, StorageSetVisibilityTool, StorageUploadFileTool,
};
