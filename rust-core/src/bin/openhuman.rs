use clap::{Args, Parser, Subcommand};
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "openhuman")]
#[command(about = "OpenHuman core CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run JSON-RPC server
    Serve {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Check core health
    Ping,
    /// Print core version
    Version,
    /// Get health snapshot
    Health,
    /// Get runtime flags
    RuntimeFlags,
    /// Get security policy info
    SecurityPolicy,
    /// Generic JSON-RPC style method call
    Call {
        #[arg(long)]
        method: String,
        #[arg(long, default_value = "{}")]
        params: String,
    },
    /// Config operations
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Service operations
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Doctor operations
    Doctor {
        #[command(subcommand)]
        command: DoctorCommand,
    },
    /// Integrations operations
    Integrations {
        #[command(subcommand)]
        command: IntegrationsCommand,
    },
    /// Send one-shot agent message
    AgentChat(AgentChatArgs),
    /// Hardware operations
    Hardware {
        #[command(subcommand)]
        command: HardwareCommand,
    },
    /// Encrypt a secret
    Encrypt {
        plaintext: String,
    },
    /// Decrypt a secret
    Decrypt {
        ciphertext: String,
    },
    /// Toggle browser allow-all runtime flag
    BrowserAllowAll {
        #[arg(long)]
        enabled: bool,
    },
    /// Refresh model catalog
    ModelsRefresh {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// Migrate OpenClaw memory
    MigrateOpenclaw {
        #[arg(long)]
        source_workspace: Option<String>,
        #[arg(long, default_value_t = true)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Get full config snapshot
    Get,
    /// Update model settings with a JSON object
    UpdateModel { #[arg(long)] json: String },
    /// Update memory settings with a JSON object
    UpdateMemory { #[arg(long)] json: String },
    /// Update gateway settings with a JSON object
    UpdateGateway { #[arg(long)] json: String },
    /// Update runtime settings with a JSON object
    UpdateRuntime { #[arg(long)] json: String },
    /// Update browser settings with a JSON object
    UpdateBrowser { #[arg(long)] json: String },
    /// Replace tunnel settings with a JSON object
    UpdateTunnel { #[arg(long)] json: String },
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    Install,
    Start,
    Stop,
    Status,
    Reinstall,
    Uninstall,
}

#[derive(Debug, Subcommand)]
enum DoctorCommand {
    /// Run doctor checks
    Report,
    /// Probe model catalog
    Models {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long, default_value_t = true)]
        use_cache: bool,
    },
}

#[derive(Debug, Subcommand)]
enum IntegrationsCommand {
    /// List integrations
    List,
    /// Get one integration info
    Info {
        #[arg(long)]
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum HardwareCommand {
    /// Discover connected hardware
    Discover,
    /// Introspect one device path
    Introspect {
        #[arg(long)]
        path: String,
    },
}

#[derive(Debug, Args)]
struct AgentChatArgs {
    message: String,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    temperature: Option<f64>,
}

fn parse_json_arg(raw: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("invalid JSON for --json/--params: {e}"))
}

async fn call_local(method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    openhuman_core::core_server::call_method(method, params).await
}

async fn execute(cli: Cli) -> Result<serde_json::Value, String> {
    match cli.command {
        Command::Serve { port } => openhuman_core::core_server::run_server(port)
            .await
            .map(|_| serde_json::Value::Null)
            .map_err(|e| format!("serve failed: {e}")),
        Command::Ping => call_local("core.ping", json!({})).await,
        Command::Version => call_local("core.version", json!({})).await,
        Command::Health => call_local("openhuman.health_snapshot", json!({})).await,
        Command::RuntimeFlags => call_local("openhuman.get_runtime_flags", json!({})).await,
        Command::SecurityPolicy => call_local("openhuman.security_policy_info", json!({})).await,
        Command::Call { method, params } => call_local(&method, parse_json_arg(&params)?).await,
        Command::Config { command } => match command {
            ConfigCommand::Get => call_local("openhuman.get_config", json!({})).await,
            ConfigCommand::UpdateModel { json } => {
                call_local("openhuman.update_model_settings", parse_json_arg(&json)?).await
            }
            ConfigCommand::UpdateMemory { json } => {
                call_local("openhuman.update_memory_settings", parse_json_arg(&json)?).await
            }
            ConfigCommand::UpdateGateway { json } => {
                call_local("openhuman.update_gateway_settings", parse_json_arg(&json)?).await
            }
            ConfigCommand::UpdateRuntime { json } => {
                call_local("openhuman.update_runtime_settings", parse_json_arg(&json)?).await
            }
            ConfigCommand::UpdateBrowser { json } => {
                call_local("openhuman.update_browser_settings", parse_json_arg(&json)?).await
            }
            ConfigCommand::UpdateTunnel { json } => {
                call_local("openhuman.update_tunnel_settings", parse_json_arg(&json)?).await
            }
        },
        Command::Service { command } => match command {
            ServiceCommand::Install => call_local("openhuman.service_install", json!({})).await,
            ServiceCommand::Start => call_local("openhuman.service_start", json!({})).await,
            ServiceCommand::Stop => call_local("openhuman.service_stop", json!({})).await,
            ServiceCommand::Status => call_local("openhuman.service_status", json!({})).await,
            ServiceCommand::Reinstall => {
                call_local("openhuman.service_uninstall", json!({})).await?;
                call_local("openhuman.service_install", json!({})).await
            }
            ServiceCommand::Uninstall => {
                call_local("openhuman.service_uninstall", json!({})).await
            }
        },
        Command::Doctor { command } => match command {
            DoctorCommand::Report => call_local("openhuman.doctor_report", json!({})).await,
            DoctorCommand::Models {
                provider,
                use_cache,
            } => {
                call_local(
                    "openhuman.doctor_models",
                    json!({
                        "provider_override": provider,
                        "use_cache": use_cache,
                    }),
                )
                .await
            }
        },
        Command::Integrations { command } => match command {
            IntegrationsCommand::List => call_local("openhuman.list_integrations", json!({})).await,
            IntegrationsCommand::Info { name } => {
                call_local("openhuman.get_integration_info", json!({ "name": name })).await
            }
        },
        Command::AgentChat(args) => {
            call_local(
                "openhuman.agent_chat",
                json!({
                    "message": args.message,
                    "provider_override": args.provider,
                    "model_override": args.model,
                    "temperature": args.temperature,
                }),
            )
            .await
        }
        Command::Hardware { command } => match command {
            HardwareCommand::Discover => call_local("openhuman.hardware_discover", json!({})).await,
            HardwareCommand::Introspect { path } => {
                call_local("openhuman.hardware_introspect", json!({ "path": path })).await
            }
        },
        Command::Encrypt { plaintext } => {
            call_local("openhuman.encrypt_secret", json!({ "plaintext": plaintext })).await
        }
        Command::Decrypt { ciphertext } => {
            call_local("openhuman.decrypt_secret", json!({ "ciphertext": ciphertext })).await
        }
        Command::BrowserAllowAll { enabled } => {
            call_local("openhuman.set_browser_allow_all", json!({ "enabled": enabled })).await
        }
        Command::ModelsRefresh { provider, force } => {
            call_local(
                "openhuman.models_refresh",
                json!({
                    "provider_override": provider,
                    "force": force,
                }),
            )
            .await
        }
        Command::MigrateOpenclaw {
            source_workspace,
            dry_run,
        } => {
            call_local(
                "openhuman.migrate_openclaw",
                json!({
                    "source_workspace": source_workspace,
                    "dry_run": dry_run,
                }),
            )
            .await
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match execute(cli).await {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| "null".to_string())
            );
        }
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}
