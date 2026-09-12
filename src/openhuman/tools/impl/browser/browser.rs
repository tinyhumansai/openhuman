//! Browser automation tool with pluggable backends.
//!
//! By default this uses Vercel's `agent-browser` tool for automation.
//! Playwright is also supported as a local Node-backed backend. Optionally, a
//! Rust-native backend can be enabled at build time via `--features
//! browser-native` and selected through config.
//! Computer-use (OS-level) actions are supported via an optional sidecar endpoint.

#[path = "action_parser.rs"]
mod action_parser;
#[cfg(feature = "browser-native")]
#[path = "native_backend.rs"]
mod native_backend;
#[path = "security.rs"]
mod security;
#[path = "types.rs"]
mod types;

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
include!("browser_part_01.rs");
include!("browser_part_02.rs");
include!("browser_part_03.rs");
