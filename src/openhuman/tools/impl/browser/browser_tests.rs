use super::*;

static BROWSER_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[path = "browser_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "browser_tests_part_02_tests.rs"]
mod part_02_tests;
