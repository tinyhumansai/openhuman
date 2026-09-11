//! Channel startup wiring.

#[cfg(any(test, debug_assertions))]
#[path = "startup_test_support_tests.rs"]
pub mod test_support;

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "startup_yuanbao_secret_tests_tests.rs"]
mod yuanbao_secret_tests;

#[cfg(test)]
#[path = "startup_email_secret_tests_tests.rs"]
mod email_secret_tests;
include!("startup_part_01.rs");
include!("startup_part_02.rs");
