use super::*;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[path = "encrypted_store_crypto_migration_tests.rs"]
mod crypto_migration_tests;
#[path = "encrypted_store_key_management_tests.rs"]
mod key_management_tests;
