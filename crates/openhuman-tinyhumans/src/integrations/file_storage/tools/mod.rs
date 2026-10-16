//! Agent-facing file-storage tools backed by the OpenHuman backend's
//! `file_storage` provider (S3 under the hood).
//!
//! **Endpoints** (see the file-storage API contract):
//!   - `POST   /agent-integrations/file-storage/files` (multipart upload)
//!   - `GET    /agent-integrations/file-storage/files` (list)
//!   - `GET    /agent-integrations/file-storage/files/{id}/download` (302 → presigned S3)
//!   - `POST   /agent-integrations/file-storage/files/{id}/link` (presigned link)
//!   - `PATCH  /agent-integrations/file-storage/files/{id}` (visibility)
//!   - `DELETE /agent-integrations/file-storage/files/{id}`
//!
//! Billing: uploads are charged upfront for the whole TTL at S3 rates plus a
//! margin; downloads and link generation are charged as egress. Quota is
//! 1 GiB per user; TTL is 7 days on the free plan / up to 1 year on paid
//! plans. Public files get a stable public URL.

#[cfg(test)]
#[path = "../tools_tests.rs"]
mod tools_tests;

mod delete;
mod download;
pub(crate) mod helpers;
mod link;
mod list;
mod registry;
mod upload;
mod visibility;

pub use delete::StorageDeleteFileTool;
pub use download::StorageDownloadFileTool;
pub use link::StorageGetLinkTool;
pub use list::StorageListFilesTool;
pub use registry::build_file_storage_tools;
pub use upload::StorageUploadFileTool;
pub use visibility::StorageSetVisibilityTool;

// Test-only bridges: `tools_tests.rs` (kept as-is; not part of the unsplit
// batch) reaches these through `use super::{...}`, mirroring the flat scope
// it had when `include!` spliced everything into one file.
#[cfg(test)]
use helpers::{resolve_upload_path, sanitize_filename};
