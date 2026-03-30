use anyhow::Result;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::core::all;
use crate::core::jsonrpc::{default_state, invoke_method, parse_json_params};
use crate::core::{ControllerSchema, TypeSchema};

const CLI_BANNER: &str = r#"

 ▗▄▖ ▄▄▄▄  ▗▞▀▚▖▄▄▄▄  ▗▖ ▗▖█  ▐▌▄▄▄▄  ▗▞▀▜▌▄▄▄▄
▐▌ ▐▌█   █ ▐▛▀▀▘█   █ ▐▌ ▐▌▀▄▄▞▘█ █ █ ▝▚▄▟▌█   █
▐▌ ▐▌█▄▄▄▀ ▝▚▄▄▖█   █ ▐▛▀▜▌     █   █      █   █
▝▚▄▞▘█                ▐▌ ▐▌
     ▀

Contribute & Star us on GitHub: https://github.com/tinyhumansai/openhuman

"#;

pub fn run_from_cli_args(args: &[String]) -> Result<()> {
    eprint!("{CLI_BANNER}");

    let grouped = grouped_schemas();
    if args.is_empty() || is_help(&args[0]) {
        print_general_help(&grouped);
        return Ok(());
    }

    match args[0].as_str() {
        "run" | "serve" => run_server_command(&args[1..]),
        "call" => run_call_command(&args[1..]),
        namespace => run_namespace_command(namespace, &args[1..], &grouped),
    }
}

fn run_server_command(args: &[String]) -> Result<()> {
    let mut port: Option<u16> = None;
    let mut socketio_enabled = true;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --port"))?;
                port = Some(
                    raw.parse::<u16>()
                        .map_err(|e| anyhow::anyhow!("invalid --port: {e}"))?,
                );
                i += 2;
            }
            "--jsonrpc-only" => {
                socketio_enabled = false;
                i += 1;
            }
            "-h" | "--help" => {
                println!("Usage: openhuman run [--port <u16>] [--jsonrpc-only]");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown run arg: {other}")),
        }
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async { crate::core::jsonrpc::run_server(port, socketio_enabled).await })?;
    Ok(())
}

fn run_call_command(args: &[String]) -> Result<()> {
    let mut method: Option<String> = None;
    let mut params = "{}".to_string();

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--method" => {
                method = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --method"))?
                        .clone(),
                );
                i += 2;
            }
            "--params" => {
                params = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --params"))?
                    .clone();
                i += 2;
            }
            "-h" | "--help" => {
                println!("Usage: openhuman call --method <name> [--params '<json>']");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown call arg: {other}")),
        }
    }

    let method = method.ok_or_else(|| anyhow::anyhow!("--method is required"))?;
    let params = parse_json_params(&params).map_err(anyhow::Error::msg)?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let value = rt
        .block_on(async { invoke_method(default_state(), &method, params).await })
        .map_err(anyhow::Error::msg)?;

    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn run_namespace_command(
    namespace: &str,
    args: &[String],
    grouped: &BTreeMap<String, Vec<ControllerSchema>>,
) -> Result<()> {
    let Some(schemas) = grouped.get(namespace) else {
        return Err(anyhow::anyhow!(
            "unknown namespace '{namespace}'. Run `openhuman --help` to see available namespaces."
        ));
    };

    if args.is_empty() || is_help(&args[0]) {
        print_namespace_help(namespace, schemas);
        return Ok(());
    }

    let function = args[0].as_str();
    let Some(schema) = schemas.iter().find(|s| s.function == function).cloned() else {
        return Err(anyhow::anyhow!(
            "unknown function '{namespace} {function}'. Run `openhuman {namespace} --help`."
        ));
    };

    if args.len() > 1 && is_help(&args[1]) {
        print_function_help(namespace, &schema);
        return Ok(());
    }

    let params = parse_function_params(&schema, &args[1..]).map_err(anyhow::Error::msg)?;
    let method = all::rpc_method_from_parts(namespace, function)
        .ok_or_else(|| anyhow::anyhow!("unregistered controller '{namespace}.{function}'"))?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let value = rt
        .block_on(async { invoke_method(default_state(), &method, Value::Object(params)).await })
        .map_err(anyhow::Error::msg)?;

    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn parse_function_params(
    schema: &ControllerSchema,
    args: &[String],
) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    let mut i = 0usize;

    while i < args.len() {
        let raw = &args[i];
        if !raw.starts_with("--") {
            return Err(format!("invalid arg '{raw}', expected --<param> <value>"));
        }
        let key = raw.trim_start_matches("--").replace('-', "_");
        let Some(spec) = schema.inputs.iter().find(|input| input.name == key) else {
            return Err(format!(
                "unknown param '{key}' for {}.{}",
                schema.namespace, schema.function
            ));
        };
        let raw_value = args
            .get(i + 1)
            .ok_or_else(|| format!("missing value for --{key}"))?;
        let value = parse_input_value(&spec.ty, raw_value)?;
        out.insert(key, value);
        i += 2;
    }

    all::validate_params(schema, &out)?;
    Ok(out)
}

fn parse_input_value(ty: &TypeSchema, raw: &str) -> Result<Value, String> {
    match ty {
        TypeSchema::String => Ok(Value::String(raw.to_string())),
        TypeSchema::Bool => raw
            .parse::<bool>()
            .map(Value::Bool)
            .map_err(|e| format!("expected bool, got '{raw}': {e}")),
        TypeSchema::I64 => raw
            .parse::<i64>()
            .map(|n| Value::Number(n.into()))
            .map_err(|e| format!("expected i64, got '{raw}': {e}")),
        TypeSchema::U64 => raw
            .parse::<u64>()
            .map(|n| Value::Number(n.into()))
            .map_err(|e| format!("expected u64, got '{raw}': {e}")),
        TypeSchema::F64 => {
            let n = raw
                .parse::<f64>()
                .map_err(|e| format!("expected f64, got '{raw}': {e}"))?;
            serde_json::Number::from_f64(n)
                .map(Value::Number)
                .ok_or_else(|| format!("invalid f64 '{raw}'"))
        }
        TypeSchema::Option(inner) => parse_input_value(inner, raw),
        TypeSchema::Enum { .. } => Ok(Value::String(raw.to_string())),
        TypeSchema::Json
        | TypeSchema::Array(_)
        | TypeSchema::Map(_)
        | TypeSchema::Object { .. }
        | TypeSchema::Ref(_)
        | TypeSchema::Bytes => parse_json_params(raw),
    }
}

fn grouped_schemas() -> BTreeMap<String, Vec<ControllerSchema>> {
    let mut grouped: BTreeMap<String, Vec<ControllerSchema>> = BTreeMap::new();
    for schema in all::all_controller_schemas() {
        grouped
            .entry(schema.namespace.to_string())
            .or_default()
            .push(schema);
    }
    for schemas in grouped.values_mut() {
        schemas.sort_by_key(|s| s.function);
    }
    grouped
}

fn print_general_help(grouped: &BTreeMap<String, Vec<ControllerSchema>>) {
    println!("OpenHuman core CLI\n");
    println!("Usage:");
    println!("  openhuman run [--port <u16>]");
    println!("  openhuman call --method <name> [--params '<json>']");
    println!("  openhuman <namespace> <function> [--param value ...]\n");
    println!("Available namespaces:");
    for namespace in grouped.keys() {
        let description = all::namespace_description(namespace.as_str())
            .unwrap_or("No namespace description available.");
        println!("  {namespace} - {description}");
    }
    println!("\nUse `openhuman <namespace> --help` to see functions.");
}

fn print_namespace_help(namespace: &str, schemas: &[ControllerSchema]) {
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

fn print_function_help(namespace: &str, schema: &ControllerSchema) {
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

fn is_help(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help")
}

#[cfg(test)]
mod tests {
    use super::grouped_schemas;

    #[test]
    fn grouped_schemas_contains_migrated_namespaces() {
        let grouped = grouped_schemas();
        assert!(grouped.contains_key("health"));
        assert!(grouped.contains_key("doctor"));
        assert!(grouped.contains_key("encrypt"));
        assert!(grouped.contains_key("decrypt"));
        assert!(grouped.contains_key("autocomplete"));
        assert!(grouped.contains_key("config"));
        assert!(grouped.contains_key("auth"));
        assert!(grouped.contains_key("service"));
        assert!(grouped.contains_key("migrate"));
        assert!(grouped.contains_key("local_ai"));
    }
}
