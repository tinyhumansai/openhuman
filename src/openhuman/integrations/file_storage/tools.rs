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
#[path = "tools_tests.rs"]
mod tools_tests;
include!("tools_part_01.rs");
include!("tools_part_02.rs");
