//! Storage connectivity and interrupted-layout checks.

use std::sync::Arc;

use async_trait::async_trait;
use tinystoragedrivers::StorageConfig;

use crate::doctor::contract::{DoctorCheck, DoctorCtx, DoctorTier, Finding};

pub(super) fn checks() -> Vec<Arc<dyn DoctorCheck>> {
    vec![Arc::new(StorageConnect), Arc::new(StorageLayout)]
}

struct StorageConnect;

#[async_trait]
impl DoctorCheck for StorageConnect {
    fn id(&self) -> &str {
        "storage/connect"
    }

    fn category(&self) -> &str {
        "storage"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let Some(url) = crate::storage::url_from(None, ctx.config()) else {
            return vec![Finding::skipped(
                self.id(),
                "Storage is not configured for this selected profile.",
                "Configure storage for the selected profile and rerun doctor.",
            )];
        };
        let parsed = match StorageConfig::parse(&url) {
            Ok(parsed) => parsed,
            Err(_) => {
                return vec![Finding::error(
                    self.id(),
                    "The selected storage configuration is invalid.",
                    "Review the storage URL in the selected configuration.",
                )];
            }
        };
        inspect_storage_config(self.id(), parsed, 0).await
    }
}

async fn inspect_storage_config(check_id: &str, config: StorageConfig, depth: u8) -> Vec<Finding> {
    if depth > 2 {
        return vec![Finding::skipped(
            check_id,
            "The selected composite storage configuration is nested beyond the inspection limit.",
            "Simplify the storage configuration and rerun doctor.",
        )];
    }
    match config {
        StorageConfig::Sqlite { path } => {
            match tokio::task::spawn_blocking(move || sqlite_probe(&path)).await {
                Ok(true) => vec![Finding::pass(
                    check_id,
                    "The selected SQLite storage database is readable.",
                )],
                Ok(false) | Err(_) => vec![Finding::error(
                    check_id,
                    "The selected SQLite storage database could not be read.",
                    "Check that the configured database exists, is readable, and is not busy.",
                )],
            }
        }
        StorageConfig::Memory => vec![Finding::pass(
            check_id,
            "The selected in-memory storage driver is available.",
        )],
        StorageConfig::File { dir } => {
            match tokio::task::spawn_blocking(move || file_probe(&dir)).await {
                Ok(true) => vec![Finding::pass(
                    check_id,
                    "The selected file storage directory exists and is readable.",
                )],
                Ok(false) | Err(_) => vec![Finding::error(
                    check_id,
                    "The selected file storage directory could not be read.",
                    "Check that the configured directory exists and is readable.",
                )],
            }
        }
        StorageConfig::MongoDb { uri, database } => {
            #[cfg(feature = "storage-mongodb")]
            {
                match tinystoragedrivers::MongoStorage::connect(&uri, &database).await {
                    Ok(_) => vec![Finding::pass(
                        check_id,
                        "The selected MongoDB server answered a read-only hello request.",
                    )],
                    Err(_) => vec![Finding::error(
                        check_id,
                        "The selected MongoDB server could not be reached.",
                        "Check the selected MongoDB endpoint and credentials.",
                    )],
                }
            }
            #[cfg(not(feature = "storage-mongodb"))]
            {
                let _ = (uri, database);
                vec![Finding::skipped(
                    check_id,
                    "MongoDB connectivity cannot be checked in this build.",
                    "Use a build with MongoDB storage support to probe this backend.",
                )]
            }
        }
        StorageConfig::Composite { records, blobs } => {
            let mut findings =
                Box::pin(inspect_storage_config(check_id, *records, depth + 1)).await;
            findings.extend(match *blobs {
                tinystoragedrivers::BlobConfig::Backend(config) => {
                    Box::pin(inspect_storage_config(check_id, config, depth + 1)).await
                }
                tinystoragedrivers::BlobConfig::S3 { .. } => vec![Finding::skipped(
                    check_id,
                    "The S3 blob backend has no bounded read-only connectivity probe.",
                    "Check the configured S3 endpoint with its provider's read-only diagnostics.",
                )],
            });
            findings
        }
        #[allow(unreachable_patterns)]
        _ => vec![Finding::skipped(
            check_id,
            "The selected storage driver has no safe read-only connectivity probe.",
            "Use a storage driver with a read-only doctor probe or run its own connectivity check.",
        )],
    }
}

fn sqlite_probe(path: &std::path::Path) -> bool {
    tinystoragedrivers::sqlite::SqliteStorage::probe_read_only(path).is_ok()
}

fn file_probe(path: &std::path::Path) -> bool {
    std::fs::read_dir(path).is_ok()
}

struct StorageLayout;

#[async_trait]
impl DoctorCheck for StorageLayout {
    fn id(&self) -> &str {
        "storage/layout"
    }

    fn category(&self) -> &str {
        "storage"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    fn is_blocking(&self) -> bool {
        true
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        self.detect_blocking(ctx).unwrap_or_default()
    }

    fn detect_blocking(&self, ctx: &DoctorCtx) -> Option<Vec<Finding>> {
        let Some(target) = crate::storage::layout::Target::from_config(ctx.config()) else {
            return Some(vec![Finding::pass(
                self.id(),
                "The selected configuration has no desktop layout migration state.",
            )]);
        };
        match crate::storage::layout::state::LayoutState::load(&target.profile_dir) {
            Ok(Some(_state)) => Some(vec![Finding::new(
                self.id(),
                crate::doctor::FindingSeverity::Warn,
                "A storage layout migration is incomplete.",
                "Stop OpenHuman and resume the storage layout migration from Settings or the CLI.",
            )]),
            Ok(None) => Some(vec![Finding::pass(
                self.id(),
                "No interrupted storage layout migration was found.",
            )]),
            Err(_) => Some(vec![Finding::error(
                self.id(),
                "The storage layout migration state could not be read.",
                "Check permissions for the selected profile directory.",
            )]),
        }
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
