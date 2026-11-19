use super::*;
use serde_json::json;

#[test]
fn required_mcp_identifier_preserves_text_for_module_normalization() {
    let args = json!({ "server": " `docs`,", "tool": "**search!**" });

    assert_eq!(required_string_arg(&args, "server").unwrap(), "`docs`,");
    assert_eq!(required_string_arg(&args, "tool").unwrap(), "**search!**");
}

#[test]
fn required_mcp_identifier_rejects_empty_values() {
    for value in ["", "   "] {
        let args = json!({ "server": value });
        assert!(required_string_arg(&args, "server").is_err());
    }
}

#[test]
fn module_failures_keep_transport_and_unavailability_categories() {
    use tinybus::module::ModuleCallError;

    assert_eq!(
        input_module_error_code(ModuleCallError::Unavailable),
        tinymcp_bus::errors::BUS
    );
    assert_eq!(
        input_module_error_code(ModuleCallError::IncompatibleContract),
        tinymcp_bus::errors::BUS
    );
    assert_eq!(
        input_module_error_code(ModuleCallError::TransportFailed),
        tinymcp_bus::errors::TRANSPORT
    );
    assert_eq!(
        input_module_error_code(ModuleCallError::ModuleFault),
        tinymcp_bus::errors::BUS
    );
    assert_eq!(
        input_module_error_message(ModuleCallError::Unavailable),
        "TinyMCP module is unavailable"
    );
    assert_eq!(
        input_module_error_message(ModuleCallError::IncompatibleContract),
        "TinyMCP contract is incompatible"
    );
    assert_eq!(
        input_module_error_message(ModuleCallError::TransportFailed),
        "TinyMCP transport failed"
    );
    assert_eq!(
        input_module_error_message(ModuleCallError::ModuleFault),
        "TinyMCP execution failed"
    );
}
