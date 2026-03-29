use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::{Map, Value};

use crate::core::all;
use crate::core::jsonrpc::{default_state, invoke_method, parse_json_params};

const CLI_BANNER: &str = r#"

 ▗▄▖ ▄▄▄▄  ▗▞▀▚▖▄▄▄▄  ▗▖ ▗▖█  ▐▌▄▄▄▄  ▗▞▀▜▌▄▄▄▄
▐▌ ▐▌█   █ ▐▛▀▀▘█   █ ▐▌ ▐▌▀▄▄▞▘█ █ █ ▝▚▄▟▌█   █
▐▌ ▐▌█▄▄▄▀ ▝▚▄▄▖█   █ ▐▛▀▜▌     █   █      █   █
▝▚▄▞▘█                ▐▌ ▐▌
     ▀

Contribute & Star us on GitHub: https://github.com/tinyhumansai/openhuman

"#;

fn print_cli_banner() {
    eprint!("{CLI_BANNER}");
}

#[derive(Debug, Parser)]
#[command(name = "openhuman")]
#[command(about = "OpenHuman core CLI")]
#[command(arg_required_else_help = true)]
struct CoreCli {
    #[command(subcommand)]
    command: CoreCommand,
}

#[derive(Debug, Subcommand)]
enum CoreCommand {
    /// Run JSON-RPC server
    #[command(alias = "serve")]
    Run {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Generic JSON-RPC style method call
    Call {
        #[arg(long)]
        method: String,
        #[arg(long, default_value = "{}")]
        params: String,
    },
    /// Invoke a registered controller by namespace/function using schema validation.
    Controller {
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        function: String,
        /// Repeatable param entry in `key=json_value` form.
        #[arg(long = "param")]
        params: Vec<String>,
    },
    /// List registered controller schemas.
    Controllers,
}

pub fn run_from_cli_args(args: &[String]) -> Result<()> {
    print_cli_banner();

    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push("openhuman".to_string());
    argv.extend_from_slice(args);
    let cli = CoreCli::try_parse_from(argv).map_err(|e| anyhow::anyhow!(e.render().to_string()))?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let value = rt
        .block_on(async move {
            match cli.command {
                CoreCommand::Run { port } => {
                    crate::core_server::run_server(port)
                        .await
                        .map(|_| serde_json::json!({ "ok": true }))
                        .map_err(|e| e.to_string())
                }
                CoreCommand::Call { method, params } => {
                    let params = parse_json_params(&params)?;
                    invoke_method(default_state(), &method, params).await
                }
                CoreCommand::Controller {
                    namespace,
                    function,
                    params,
                } => run_controller_command(&namespace, &function, params).await,
                CoreCommand::Controllers => Ok(serde_json::json!({
                    "controllers": all::all_controller_schemas()
                        .into_iter()
                        .map(|schema| serde_json::json!({
                            "namespace": schema.namespace,
                            "function": schema.function,
                            "rpc_method": all::rpc_method_name(&schema),
                            "description": schema.description,
                            "inputs": schema.inputs.iter().map(|i| serde_json::json!({
                                "name": i.name,
                                "required": i.required,
                                "comment": i.comment,
                            })).collect::<Vec<_>>(),
                        }))
                        .collect::<Vec<_>>(),
                })),
            }
        })
        .map_err(anyhow::Error::msg)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| anyhow::anyhow!(e.to_string()))?
    );
    Ok(())
}

async fn run_controller_command(
    namespace: &str,
    function: &str,
    param_entries: Vec<String>,
) -> Result<Value, String> {
    let method = all::rpc_method_from_parts(namespace, function).ok_or_else(|| {
        format!("unknown controller '{}.{}' (see `openhuman controllers`)", namespace, function)
    })?;
    let params = parse_param_entries(param_entries)?;
    invoke_method(default_state(), &method, Value::Object(params)).await
}

fn parse_param_entries(entries: Vec<String>) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for entry in entries {
        let (key, raw_value) = entry
            .split_once('=')
            .ok_or_else(|| format!("invalid --param '{entry}': expected key=json_value"))?;
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("invalid --param '{entry}': key must not be empty"));
        }
        let value = serde_json::from_str(raw_value.trim())
            .map_err(|e| format!("invalid JSON value for --param '{key}': {e}"))?;
        out.insert(key.to_string(), value);
    }
    Ok(out)
}
