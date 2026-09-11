//! Unit tests for the MCP clients RPC handlers.
//!
//! What is testable without a running service is the prompt the configuration
//! assistant builds and the blank-identifier guards. The operations themselves
//! are covered in `tinymcp`; what this layer adds is the RPC shape, which the
//! end-to-end suites exercise against a live process.

use super::*;

#[test]
fn the_assistant_prompt_lists_the_credentials_the_server_needs() {
    let prompt = build_config_assist_system_prompt(
        "Test Server",
        "@test/server",
        &["API_KEY".to_string(), "SECRET".to_string()],
    );

    assert!(prompt.contains("API_KEY"));
    assert!(prompt.contains("SECRET"));
    assert!(prompt.contains("Test Server"));
    assert!(prompt.contains("@test/server"));
}

#[test]
fn the_assistant_prompt_says_so_when_a_server_needs_nothing() {
    let prompt = build_config_assist_system_prompt("My Server", "@my/server", &[]);
    assert!(prompt.contains("none detected"));
}

#[test]
fn a_blank_identifier_is_refused_with_the_field_name() {
    // The frontend surfaces this text, so it has to name what was missing.
    let error = require("   ", "server_id").expect_err("a blank identifier");
    assert_eq!(error, "server_id must not be empty");
}

#[test]
fn an_identifier_is_trimmed_before_it_is_used() {
    assert_eq!(require("  srv-1  ", "server_id").unwrap(), "srv-1");
}
