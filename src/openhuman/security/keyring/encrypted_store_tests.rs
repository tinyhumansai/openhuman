use super::*;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[path = "encrypted_store_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "encrypted_store_tests_part_02_tests.rs"]
mod part_02_tests;
