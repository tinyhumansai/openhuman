use super::super::all;
use crate::core::ControllerSchema;
use std::collections::BTreeMap;

/// Prints the general help message listing available commands and namespaces.
pub(super) fn print_general_help(grouped: &BTreeMap<String, Vec<ControllerSchema>>) {
    println!("OpenHuman core CLI\n");
    println!("Usage:");
    println!("  openhuman [OPTIONS]                     (show this help)");
    println!("  openhuman run [--host <addr>] [--port <u16>] [--jsonrpc-only] [--verbose]");
    println!("  openhuman call --method <name> [--params '<json>' | --params-stdin]");
    println!("  openhuman mcp [-v|--verbose]              (stdio MCP server)");
    println!("  openhuman-tui [--thread <id>|--new]       (terminal UI; separate executable)");
    println!("  openhuman skills <subcommand> [options]   (skill development runtime)");
    println!("  openhuman agent <subcommand> [options]    (inspect agent definitions & prompts)");
    println!("  openhuman sentry-test [--message <text>] [--panic]  (verify Sentry wiring)");
    println!("  openhuman <namespace> <function> [--param value ...]\n");
    println!("Global options (place before the command):");
    println!("  -m, --model <id>       Override the model for this CLI session");
    println!("  -p, --provider <id>    Override the provider id or slug for this CLI session");
    println!("                        (aliases: --model-id, --provider-id)\n");
    println!("Available namespaces:");
    for namespace in grouped.keys() {
        let description = all::namespace_description(namespace.as_str())
            .unwrap_or("No namespace description available.");
        println!("  {namespace} - {description}");
    }
    println!("\nUse `openhuman <namespace> --help` to see functions.");
}

/// Prints help for a specific namespace, listing its functions.
pub(super) fn print_namespace_help(namespace: &str, schemas: &[ControllerSchema]) {
    println!("Namespace: {namespace}\n");
    if let Some(description) = all::namespace_description(namespace) {
        println!("{description}\n");
    }
    println!("Functions:");
    for schema in schemas {
        println!("  {} - {}", schema.function, schema.description);
    }
    println!("\nUse `openhuman {namespace} <function> --help` for parameters.");
}

/// Prints detailed help for a specific function, including its parameters and description.
pub(super) fn print_function_help(namespace: &str, schema: &ControllerSchema) {
    println!("{} {}\n", namespace, schema.function);
    println!("{}", schema.description);
    println!("\nParameters:");
    if schema.inputs.is_empty() {
        println!("  none");
    } else {
        for input in &schema.inputs {
            let required = if input.required {
                "required"
            } else {
                "optional"
            };
            println!("  --{} ({}) - {}", input.name, required, input.comment);
        }
    }
}

/// Checks if a string represents a help flag.
pub(super) fn is_help(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help")
}
