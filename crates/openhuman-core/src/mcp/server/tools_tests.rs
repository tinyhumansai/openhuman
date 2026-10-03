use super::specs::tool_specs_for_loaded_config;
use super::*;

/// The `tools/list` result this config advertises, rendered through the same
/// `tinymcp` conversion the served catalog uses.
fn list_tools_result_for_config(config: &crate::config::Config) -> Value {
    let tools = tool_specs_for_loaded_config(config)
        .iter()
        .map(|spec| server_tool_spec(spec).to_json())
        .collect::<Vec<_>>();
    json!({ "tools": tools })
}

#[path = "tools_tests_dispatch_tests.rs"]
mod dispatch_tests;
#[path = "tools_tests_params_and_specs_tests.rs"]
mod params_and_specs_tests;
