use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use serde_json::json;

use crate::core_server::helpers::{
    parse_params, rpc_outcome_fut_to_cli_json, rpc_outcome_to_cli_json,
};
use crate::core_server::types::{
    command_response, BrowserSettingsUpdate, CommandResponse, ConfigSnapshot,
    GatewaySettingsUpdate, MemorySettingsUpdate, ModelSettingsUpdate, RuntimeSettingsUpdate,
};
use crate::core_server::{call_method, run_server, APP_SESSION_PROVIDER};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::{Config, TunnelConfig};
use crate::openhuman::credentials;
use crate::openhuman::heartbeat::engine::HeartbeatEngine;
use crate::openhuman::screen_intelligence::{
    AccessibilityStatus, CaptureImageRefResult, PermissionState,
};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::skills::init_skills_dir;
use crate::openhuman::tools::{ScreenshotTool, Tool};

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
    /// Initialize local OpenHuman workspace with baseline files
    Init(InitCliArgs),
    /// Run JSON-RPC server
    #[command(alias = "serve")]
    Run {
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
    /// Generate shell completion scripts
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Settings style commands mirroring app settings sections
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    /// Accessibility automation commands
    Accessibility {
        #[command(subcommand)]
        command: AccessibilityCommand,
    },
    /// Standalone inline autocomplete commands
    Autocomplete {
        #[command(subcommand)]
        command: AutocompleteCommand,
    },
    /// Tool wrappers for local CLI testing
    Tools {
        #[command(subcommand)]
        command: ToolsCommand,
    },
    /// Authentication and credential management commands
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Socket lifecycle and messaging commands
    Socket {
        #[command(subcommand)]
        command: SocketCommand,
    },
    /// Legacy config operations
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
enum SettingsCommand {
    Model {
        #[command(subcommand)]
        command: ModelSettingsCommand,
    },
    Memory {
        #[command(subcommand)]
        command: MemorySettingsCommand,
    },
    Gateway {
        #[command(subcommand)]
        command: GatewaySettingsCommand,
    },
    Tunnel {
        #[command(subcommand)]
        command: TunnelSettingsCommand,
    },
    Runtime {
        #[command(subcommand)]
        command: RuntimeSettingsCommand,
    },
    Browser {
        #[command(subcommand)]
        command: BrowserSettingsCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ModelSettingsCommand {
    Get,
    Set(ModelSetArgs),
}

#[derive(Debug, Args)]
struct ModelSetArgs {
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long)]
    api_url: Option<String>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    temperature: Option<f64>,
}

#[derive(Debug, Subcommand)]
enum MemorySettingsCommand {
    Get,
    Set(MemorySetArgs),
}

#[derive(Debug, Args)]
struct MemorySetArgs {
    #[arg(long)]
    backend: Option<String>,
    #[arg(long)]
    auto_save: Option<bool>,
    #[arg(long)]
    embedding_provider: Option<String>,
    #[arg(long)]
    embedding_model: Option<String>,
    #[arg(long)]
    embedding_dimensions: Option<usize>,
}

#[derive(Debug, Subcommand)]
enum GatewaySettingsCommand {
    Get,
    Set(GatewaySetArgs),
}

#[derive(Debug, Args)]
struct GatewaySetArgs {
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    port: Option<u16>,
    #[arg(long)]
    require_pairing: Option<bool>,
    #[arg(long)]
    allow_public_bind: Option<bool>,
}

#[derive(Debug, Subcommand)]
enum TunnelSettingsCommand {
    Get,
    /// Replace tunnel settings with full JSON payload
    Set(TunnelSetArgs),
}

#[derive(Debug, Args)]
struct TunnelSetArgs {
    #[arg(long)]
    json: String,
}

#[derive(Debug, Subcommand)]
enum RuntimeSettingsCommand {
    Get,
    Set(RuntimeSetArgs),
}

#[derive(Debug, Args)]
struct RuntimeSetArgs {
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    reasoning_enabled: Option<bool>,
}

#[derive(Debug, Subcommand)]
enum BrowserSettingsCommand {
    Get,
    Set(BrowserSetArgs),
}

#[derive(Debug, Subcommand)]
enum AccessibilityCommand {
    /// Read current accessibility automation status
    Status,
    /// Diagnose accessibility permission readiness with actionable fixes
    Doctor,
    /// Request all accessibility-related permissions
    RequestPermissions,
    /// Request a specific permission kind
    RequestPermission(RequestPermissionArgs),
    /// Start a bounded screen intelligence session
    StartSession(StartSessionCliArgs),
    /// Stop the active screen intelligence session
    StopSession(StopSessionCliArgs),
    /// Force an immediate capture sample
    CaptureNow,
    /// Directly trigger capture_screen_image_ref (no active session required)
    CaptureImageRef,
    /// Fetch recent vision summaries
    VisionRecent(VisionRecentCliArgs),
    /// Flush immediate vision summary from latest frame
    VisionFlush,
}

#[derive(Debug, Subcommand)]
enum AutocompleteCommand {
    Status,
    Start(AutocompleteStartCliArgs),
    Stop(AutocompleteStopCliArgs),
    Current(AutocompleteCurrentCliArgs),
    Accept(AutocompleteAcceptCliArgs),
    SetStyle(AutocompleteSetStyleCliArgs),
}

#[derive(Debug, Args)]
struct RequestPermissionArgs {
    /// One of: screen_recording, accessibility, input_monitoring
    #[arg(long)]
    permission: String,
}

#[derive(Debug, Args)]
struct StartSessionCliArgs {
    /// Explicit consent required to start
    #[arg(long, default_value_t = false)]
    consent: bool,
    /// Optional session TTL in seconds (bounded server-side)
    #[arg(long)]
    ttl_secs: Option<u64>,
    /// Optional override for screen monitoring
    #[arg(long)]
    screen_monitoring: Option<bool>,
    /// Optional override for device control
    #[arg(long)]
    device_control: Option<bool>,
    /// Optional override for predictive input
    #[arg(long)]
    predictive_input: Option<bool>,
}

#[derive(Debug, Args)]
struct StopSessionCliArgs {
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Args)]
struct VisionRecentCliArgs {
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Args)]
struct AutocompleteStartCliArgs {
    #[arg(long)]
    debounce_ms: Option<u64>,
}

#[derive(Debug, Args)]
struct AutocompleteStopCliArgs {
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Args)]
struct AutocompleteCurrentCliArgs {
    #[arg(long)]
    context: Option<String>,
}

#[derive(Debug, Args)]
struct AutocompleteAcceptCliArgs {
    #[arg(long)]
    suggestion: Option<String>,
}

#[derive(Debug, Args)]
struct AutocompleteSetStyleCliArgs {
    #[arg(long)]
    enabled: Option<bool>,
    #[arg(long)]
    debounce_ms: Option<u64>,
    #[arg(long)]
    max_chars: Option<usize>,
    #[arg(long)]
    style_preset: Option<String>,
    #[arg(long)]
    style_instructions: Option<String>,
    #[arg(long)]
    style_example: Vec<String>,
    #[arg(long)]
    disabled_app: Vec<String>,
    #[arg(long)]
    accept_with_tab: Option<bool>,
}

#[derive(Debug, Args)]
struct BrowserSetArgs {
    #[arg(long)]
    enabled: Option<bool>,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Get full config snapshot
    Get,
    /// Update model settings with a JSON object
    UpdateModel {
        #[arg(long)]
        json: String,
    },
    /// Update memory settings with a JSON object
    UpdateMemory {
        #[arg(long)]
        json: String,
    },
    /// Update gateway settings with a JSON object
    UpdateGateway {
        #[arg(long)]
        json: String,
    },
    /// Update runtime settings with a JSON object
    UpdateRuntime {
        #[arg(long)]
        json: String,
    },
    /// Update browser settings with a JSON object
    UpdateBrowser {
        #[arg(long)]
        json: String,
    },
    /// Replace tunnel settings with a JSON object
    UpdateTunnel {
        #[arg(long)]
        json: String,
    },
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// Store session or provider credentials
    Login(AuthLoginArgs),
    /// Remove stored session or provider credentials
    Logout(AuthLogoutArgs),
    /// Show auth/session state
    Status(AuthStatusArgs),
    /// List stored provider credentials (excluding app session)
    List(AuthListArgs),
}

#[derive(Debug, Args)]
struct AuthLoginArgs {
    /// Provider identifier (`app-session`, `google`, `discord`, etc.)
    #[arg(long, default_value = APP_SESSION_PROVIDER)]
    provider: String,
    /// Profile name (default: "default")
    #[arg(long)]
    profile: Option<String>,
    /// Main token/api-key field for this provider
    #[arg(long)]
    token: Option<String>,
    /// Optional user id for app session flows
    #[arg(long)]
    user_id: Option<String>,
    /// Optional user payload JSON for app session flows
    #[arg(long)]
    user_json: Option<String>,
    /// Additional credential fields (`key=value`, repeatable)
    #[arg(long = "field")]
    field: Vec<String>,
    /// Mark this profile as active
    #[arg(long, default_value_t = true)]
    set_active: bool,
}

#[derive(Debug, Args)]
struct AuthLogoutArgs {
    /// Provider identifier
    #[arg(long, default_value = APP_SESSION_PROVIDER)]
    provider: String,
    /// Profile name (default: "default")
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Debug, Args)]
struct AuthStatusArgs {
    /// Provider identifier (defaults to `app-session`)
    #[arg(long, default_value = APP_SESSION_PROVIDER)]
    provider: String,
    /// Optional profile override
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Debug, Args)]
struct AuthListArgs {
    /// Optional provider filter
    #[arg(long)]
    provider: Option<String>,
}

#[derive(Debug, Subcommand)]
enum SocketCommand {
    /// Connect to socket backend
    Connect(SocketConnectCliArgs),
    /// Disconnect socket backend
    Disconnect,
    /// Fetch current socket state
    Status,
    /// Emit a socket event
    Emit(SocketEmitCliArgs),
}

#[derive(Debug, Args)]
struct SocketConnectCliArgs {
    #[arg(long)]
    url: String,
    #[arg(long)]
    token: String,
}

#[derive(Debug, Args)]
struct SocketEmitCliArgs {
    #[arg(long)]
    event: String,
    #[arg(long, default_value = "null")]
    data: String,
}

#[derive(Debug, Args)]
struct InitCliArgs {
    /// Overwrite existing bootstrap markdown files
    #[arg(long, default_value_t = false)]
    force: bool,
}

#[derive(Debug, Subcommand)]
enum ToolsCommand {
    /// List tool wrappers exposed by this CLI
    List,
    /// Capture a screenshot using the screenshot tool
    Screenshot(ToolsScreenshotArgs),
    /// Capture image ref directly from accessibility engine
    ScreenshotRef(ToolsScreenshotRefArgs),
    /// Generic wrapper for available tool commands
    Run(ToolsRunArgs),
}

#[derive(Debug, Args)]
struct ToolsScreenshotArgs {
    /// Optional filename saved under workspace
    #[arg(long)]
    filename: Option<String>,
    /// Optional region for macOS: selection | window
    #[arg(long)]
    region: Option<String>,
    /// Optional output file path (copies or writes PNG to this path)
    #[arg(long)]
    output: Option<PathBuf>,
    /// Include full data URL in JSON output
    #[arg(long, default_value_t = false)]
    print_data_url: bool,
}

#[derive(Debug, Args)]
struct ToolsScreenshotRefArgs {
    /// Optional output file path (writes PNG to this path)
    #[arg(long)]
    output: Option<PathBuf>,
    /// Include full data URL in JSON output
    #[arg(long, default_value_t = false)]
    print_data_url: bool,
}

#[derive(Debug, Args)]
struct ToolsRunArgs {
    /// Tool wrapper name: screenshot | screenshot-ref
    #[arg(long)]
    name: String,
    /// JSON arguments payload for selected wrapper
    #[arg(long, default_value = "{}")]
    args: String,
}

fn parse_json_arg(raw: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("invalid JSON for --json/--params: {e}"))
}

fn parse_key_value_flags(entries: &[String]) -> Result<serde_json::Value, String> {
    let mut fields = serde_json::Map::new();
    for entry in entries {
        let Some((raw_key, raw_value)) = entry.split_once('=') else {
            return Err(format!(
                "invalid --field value '{entry}', expected key=value format"
            ));
        };
        let key = raw_key.trim();
        if key.is_empty() {
            return Err("invalid --field value with empty key".to_string());
        }
        fields.insert(
            key.to_string(),
            serde_json::Value::String(raw_value.to_string()),
        );
    }
    Ok(serde_json::Value::Object(fields))
}

fn ensure_non_empty_payload(payload: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    if payload.is_empty() {
        return Err(anyhow::anyhow!("no fields provided for set operation"));
    }
    Ok(())
}

fn extract_data_url(raw: &str) -> Option<String> {
    raw.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .starts_with("data:image/")
            .then(|| trimmed.to_string())
    })
}

fn extract_saved_path(raw: &str) -> Option<PathBuf> {
    const PREFIX: &str = "Screenshot saved to: ";
    raw.lines()
        .find_map(|line| line.strip_prefix(PREFIX).map(PathBuf::from))
}

fn decode_data_url_bytes(data_url: &str) -> Result<Vec<u8>, String> {
    let (meta, payload) = data_url
        .split_once(',')
        .ok_or_else(|| "invalid data URL: missing comma separator".to_string())?;
    if !meta.starts_with("data:image/") || !meta.ends_with(";base64") {
        return Err("invalid data URL: expected data:image/*;base64,...".to_string());
    }
    BASE64_STANDARD
        .decode(payload)
        .map_err(|e| format!("failed to decode base64 image payload: {e}"))
}

fn write_bytes_to_path(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create output directory: {e}"))?;
        }
    }
    std::fs::write(path, bytes).map_err(|e| format!("failed to write output file: {e}"))
}

const BOOTSTRAP_FILES: [(&str, &str); 7] = [
    ("AGENTS.md", include_str!("../ai/prompts/AGENTS.md")),
    ("SOUL.md", include_str!("../ai/prompts/SOUL.md")),
    ("TOOLS.md", include_str!("../ai/prompts/TOOLS.md")),
    ("IDENTITY.md", include_str!("../ai/prompts/IDENTITY.md")),
    ("USER.md", include_str!("../ai/prompts/USER.md")),
    ("BOOTSTRAP.md", include_str!("../ai/prompts/BOOTSTRAP.md")),
    ("MEMORY.md", include_str!("../ai/prompts/MEMORY.md")),
];

fn ensure_workspace_file(
    workspace_dir: &Path,
    filename: &str,
    contents: &str,
    force: bool,
) -> Result<&'static str, String> {
    let path = workspace_dir.join(filename);
    if path.exists() && !force {
        return Ok("existing");
    }
    std::fs::write(&path, contents)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(if force { "overwritten" } else { "created" })
}

async fn execute_init_command(args: InitCliArgs) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let workspace_dir = config.workspace_dir.clone();

    let mut created_dirs = Vec::new();
    let mut existing_dirs = Vec::new();
    for rel in ["memory", "sessions", "state", "cron"] {
        let dir = workspace_dir.join(rel);
        if dir.exists() {
            existing_dirs.push(dir.display().to_string());
        } else {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("failed to create directory {}: {e}", dir.display()))?;
            created_dirs.push(dir.display().to_string());
        }
    }

    let mut created_files = Vec::new();
    let mut overwritten_files = Vec::new();
    let mut existing_files = Vec::new();
    for (filename, contents) in BOOTSTRAP_FILES {
        match ensure_workspace_file(&workspace_dir, filename, contents, args.force)? {
            "created" => created_files.push(workspace_dir.join(filename).display().to_string()),
            "overwritten" => {
                overwritten_files.push(workspace_dir.join(filename).display().to_string())
            }
            _ => existing_files.push(workspace_dir.join(filename).display().to_string()),
        }
    }

    let skills_readme = workspace_dir.join("skills").join("README.md");
    let had_skills_readme = skills_readme.exists();
    let heartbeat = workspace_dir.join("HEARTBEAT.md");
    let had_heartbeat = heartbeat.exists();
    init_skills_dir(&workspace_dir).map_err(|e| format!("failed to initialize skills dir: {e}"))?;
    HeartbeatEngine::ensure_heartbeat_file(&workspace_dir)
        .await
        .map_err(|e| format!("failed to initialize HEARTBEAT.md: {e}"))?;

    if had_skills_readme {
        existing_files.push(skills_readme.display().to_string());
    } else {
        created_files.push(skills_readme.display().to_string());
    }

    if had_heartbeat {
        existing_files.push(heartbeat.display().to_string());
    } else {
        created_files.push(heartbeat.display().to_string());
    }

    Ok(json!({
        "result": {
            "workspace_dir": workspace_dir.display().to_string(),
            "config_path": config.config_path.display().to_string(),
            "directories": {
                "created": created_dirs,
                "existing": existing_dirs
            },
            "files": {
                "created": created_files,
                "overwritten": overwritten_files,
                "existing": existing_files
            }
        },
        "logs": [
            "workspace initialization completed"
        ]
    }))
}

async fn execute_tools_screenshot(args: ToolsScreenshotArgs) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    let tool = ScreenshotTool::new(security);

    let mut payload = serde_json::Map::new();
    if let Some(filename) = args.filename {
        payload.insert("filename".to_string(), json!(filename));
    }
    if let Some(region) = args.region {
        payload.insert("region".to_string(), json!(region));
    }

    let tool_result = tool
        .execute(serde_json::Value::Object(payload))
        .await
        .map_err(|e| format!("screenshot tool failed to execute: {e}"))?;

    let mut logs = vec!["tools.screenshot executed".to_string()];

    if let Some(output_path) = args.output.as_ref() {
        if let Some(saved_path) = extract_saved_path(&tool_result.output) {
            std::fs::copy(&saved_path, output_path).map_err(|e| {
                format!(
                    "failed to copy screenshot from {} to {}: {e}",
                    saved_path.display(),
                    output_path.display()
                )
            })?;
            logs.push(format!("copied screenshot to {}", output_path.display()));
        } else if let Some(data_url) = extract_data_url(&tool_result.output) {
            let bytes = decode_data_url_bytes(&data_url)?;
            write_bytes_to_path(output_path, &bytes)?;
            logs.push(format!(
                "decoded data URL and wrote {} bytes to {}",
                bytes.len(),
                output_path.display()
            ));
        } else {
            return Err(
                "screenshot tool response did not contain a saved path or image data URL"
                    .to_string(),
            );
        }
    }

    let data_url = extract_data_url(&tool_result.output);
    let response = json!({
        "result": {
            "success": tool_result.success,
            "error": tool_result.error,
            "output_path": args.output.as_ref().map(|p| p.display().to_string()),
            "tool_output": tool_result.output,
            "data_url": if args.print_data_url { data_url } else { None::<String> },
        },
        "logs": logs
    });

    Ok(response)
}

async fn execute_tools_screenshot_ref(
    args: ToolsScreenshotRefArgs,
) -> Result<serde_json::Value, String> {
    let raw = rpc_outcome_fut_to_cli_json(
        crate::openhuman::screen_intelligence::rpc::accessibility_capture_image_ref(),
    )
    .await?;
    let payload: CommandResponse<CaptureImageRefResult> =
        serde_json::from_value(raw).map_err(|e| {
            format!("failed to decode screen intelligence capture_image_ref response: {e}")
        })?;

    let mut logs = payload.logs;
    logs.push("tools.screenshot-ref executed".to_string());

    if let Some(output_path) = args.output.as_ref() {
        if let Some(data_url) = payload.result.image_ref.as_deref() {
            let bytes = decode_data_url_bytes(data_url)?;
            write_bytes_to_path(output_path, &bytes)?;
            logs.push(format!(
                "decoded image_ref and wrote {} bytes to {}",
                bytes.len(),
                output_path.display()
            ));
        } else {
            return Err(
                "screen intelligence capture_image_ref did not return image_ref".to_string(),
            );
        }
    }

    Ok(json!({
        "result": {
            "ok": payload.result.ok,
            "mime_type": payload.result.mime_type,
            "bytes_estimate": payload.result.bytes_estimate,
            "message": payload.result.message,
            "output_path": args.output.as_ref().map(|p| p.display().to_string()),
            "image_ref": if args.print_data_url { payload.result.image_ref } else { None::<String> },
        },
        "logs": logs
    }))
}

async fn get_config_snapshot() -> Result<CommandResponse<ConfigSnapshot>, String> {
    let value = rpc_outcome_fut_to_cli_json(config_rpc::load_and_get_config_snapshot()).await?;
    serde_json::from_value::<CommandResponse<ConfigSnapshot>>(value)
        .map_err(|e| format!("failed to decode config snapshot: {e}"))
}

fn settings_view_response(
    section: &'static str,
    snapshot: CommandResponse<ConfigSnapshot>,
) -> CommandResponse<serde_json::Value> {
    let cfg = &snapshot.result.config;
    let settings = match section {
        "model" => json!({
            "api_key": cfg.get("api_key"),
            "api_url": cfg.get("api_url"),
            "default_provider": cfg.get("default_provider"),
            "default_model": cfg.get("default_model"),
            "default_temperature": cfg.get("default_temperature"),
        }),
        "memory" => cfg
            .get("memory")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "gateway" => cfg
            .get("gateway")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "tunnel" => cfg
            .get("tunnel")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "runtime" => cfg
            .get("runtime")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "browser" => cfg
            .get("browser")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        _ => serde_json::Value::Null,
    };

    command_response(
        json!({
            "section": section,
            "settings": settings,
            "workspace_dir": snapshot.result.workspace_dir,
            "config_path": snapshot.result.config_path,
        }),
        snapshot.logs,
    )
}

async fn execute_core_cli(cli: CoreCli) -> Result<serde_json::Value, String> {
    match cli.command {
        CoreCommand::Init(args) => execute_init_command(args).await,
        CoreCommand::Run { port } => run_server(port)
            .await
            .map(|_| serde_json::Value::Null)
            .map_err(|e| format!("run failed: {e}")),
        CoreCommand::Ping => call_method("core.ping", json!({})).await,
        CoreCommand::Version => call_method("core.version", json!({})).await,
        CoreCommand::Health => {
            rpc_outcome_to_cli_json(crate::openhuman::health::rpc::health_snapshot())
        }
        CoreCommand::RuntimeFlags => rpc_outcome_to_cli_json(config_rpc::get_runtime_flags()),
        CoreCommand::SecurityPolicy => {
            rpc_outcome_to_cli_json(crate::openhuman::security::rpc::security_policy_info())
        }
        CoreCommand::Call { method, params } => {
            call_method(&method, parse_json_arg(&params)?).await
        }
        CoreCommand::Completions { shell } => {
            let mut cmd = CoreCli::command();
            let bin_name = cmd.get_name().to_string();
            generate(shell, &mut cmd, bin_name, &mut io::stdout());
            Ok(serde_json::Value::Null)
        }
        CoreCommand::Settings { command } => match command {
            SettingsCommand::Model { command } => match command {
                ModelSettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("model", snapshot))
                        .map_err(|e| e.to_string())
                }
                ModelSettingsCommand::Set(args) => {
                    let mut payload = serde_json::Map::new();
                    if let Some(v) = args.api_key {
                        payload.insert("api_key".to_string(), json!(v));
                    }
                    if let Some(v) = args.api_url {
                        payload.insert("api_url".to_string(), json!(v));
                    }
                    if let Some(v) = args.provider {
                        payload.insert("default_provider".to_string(), json!(v));
                    }
                    if let Some(v) = args.model {
                        payload.insert("default_model".to_string(), json!(v));
                    }
                    if let Some(v) = args.temperature {
                        payload.insert("default_temperature".to_string(), json!(v));
                    }
                    ensure_non_empty_payload(&payload).map_err(|e| e.to_string())?;
                    let update: ModelSettingsUpdate =
                        parse_params(serde_json::Value::Object(payload))?;
                    rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_model_settings(
                        update.into(),
                    ))
                    .await
                }
            },
            SettingsCommand::Memory { command } => match command {
                MemorySettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("memory", snapshot))
                        .map_err(|e| e.to_string())
                }
                MemorySettingsCommand::Set(args) => {
                    let mut payload = serde_json::Map::new();
                    if let Some(v) = args.backend {
                        payload.insert("backend".to_string(), json!(v));
                    }
                    if let Some(v) = args.auto_save {
                        payload.insert("auto_save".to_string(), json!(v));
                    }
                    if let Some(v) = args.embedding_provider {
                        payload.insert("embedding_provider".to_string(), json!(v));
                    }
                    if let Some(v) = args.embedding_model {
                        payload.insert("embedding_model".to_string(), json!(v));
                    }
                    if let Some(v) = args.embedding_dimensions {
                        payload.insert("embedding_dimensions".to_string(), json!(v));
                    }
                    ensure_non_empty_payload(&payload).map_err(|e| e.to_string())?;
                    let update: MemorySettingsUpdate =
                        parse_params(serde_json::Value::Object(payload))?;
                    rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_memory_settings(
                        update.into(),
                    ))
                    .await
                }
            },
            SettingsCommand::Gateway { command } => match command {
                GatewaySettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("gateway", snapshot))
                        .map_err(|e| e.to_string())
                }
                GatewaySettingsCommand::Set(args) => {
                    let mut payload = serde_json::Map::new();
                    if let Some(v) = args.host {
                        payload.insert("host".to_string(), json!(v));
                    }
                    if let Some(v) = args.port {
                        payload.insert("port".to_string(), json!(v));
                    }
                    if let Some(v) = args.require_pairing {
                        payload.insert("require_pairing".to_string(), json!(v));
                    }
                    if let Some(v) = args.allow_public_bind {
                        payload.insert("allow_public_bind".to_string(), json!(v));
                    }
                    ensure_non_empty_payload(&payload).map_err(|e| e.to_string())?;
                    let update: GatewaySettingsUpdate =
                        parse_params(serde_json::Value::Object(payload))?;
                    rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_gateway_settings(
                        update.into(),
                    ))
                    .await
                }
            },
            SettingsCommand::Tunnel { command } => match command {
                TunnelSettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("tunnel", snapshot))
                        .map_err(|e| e.to_string())
                }
                TunnelSettingsCommand::Set(args) => {
                    let tunnel: TunnelConfig = parse_params(parse_json_arg(&args.json)?)?;
                    rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_tunnel_settings(tunnel))
                        .await
                }
            },
            SettingsCommand::Runtime { command } => match command {
                RuntimeSettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("runtime", snapshot))
                        .map_err(|e| e.to_string())
                }
                RuntimeSettingsCommand::Set(args) => {
                    let mut payload = serde_json::Map::new();
                    if let Some(v) = args.kind {
                        payload.insert("kind".to_string(), json!(v));
                    }
                    if let Some(v) = args.reasoning_enabled {
                        payload.insert("reasoning_enabled".to_string(), json!(v));
                    }
                    ensure_non_empty_payload(&payload).map_err(|e| e.to_string())?;
                    let update: RuntimeSettingsUpdate =
                        parse_params(serde_json::Value::Object(payload))?;
                    rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_runtime_settings(
                        update.into(),
                    ))
                    .await
                }
            },
            SettingsCommand::Browser { command } => match command {
                BrowserSettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("browser", snapshot))
                        .map_err(|e| e.to_string())
                }
                BrowserSettingsCommand::Set(args) => {
                    let mut payload = serde_json::Map::new();
                    if let Some(v) = args.enabled {
                        payload.insert("enabled".to_string(), json!(v));
                    }
                    ensure_non_empty_payload(&payload).map_err(|e| e.to_string())?;
                    let update: BrowserSettingsUpdate =
                        parse_params(serde_json::Value::Object(payload))?;
                    rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_browser_settings(
                        update.into(),
                    ))
                    .await
                }
            },
        },
        CoreCommand::Accessibility { command } => match command {
            AccessibilityCommand::Status => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_status(),
                )
                .await
            }
            AccessibilityCommand::Doctor => {
                let raw = rpc_outcome_fut_to_cli_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_status(),
                )
                .await?;
                let payload: CommandResponse<AccessibilityStatus> = serde_json::from_value(raw)
                    .map_err(|e| format!("failed to decode screen intelligence status: {e}"))?;
                let permissions = &payload.result.permissions;

                let screen_ready = permissions.screen_recording == PermissionState::Granted;
                let control_ready = permissions.accessibility == PermissionState::Granted;
                let monitoring_ready = permissions.input_monitoring == PermissionState::Granted;
                let overall_ready =
                    payload.result.platform_supported && screen_ready && control_ready;

                let mut recommendations: Vec<String> = Vec::new();
                if !payload.result.platform_supported {
                    recommendations.push(
                        "Accessibility automation is macOS-only in this build/runtime.".to_string(),
                    );
                }
                if permissions.screen_recording != PermissionState::Granted {
                    recommendations.push(
                        "Grant Screen Recording in System Settings -> Privacy & Security -> Screen Recording."
                            .to_string(),
                    );
                }
                if permissions.accessibility != PermissionState::Granted {
                    recommendations.push(
                        "Grant Accessibility in System Settings -> Privacy & Security -> Accessibility."
                            .to_string(),
                    );
                }
                if permissions.input_monitoring != PermissionState::Granted {
                    recommendations.push(
                        "Grant Input Monitoring in System Settings -> Privacy & Security -> Input Monitoring (optional but recommended)."
                            .to_string(),
                    );
                }
                if recommendations.is_empty() {
                    recommendations
                        .push("No action required. Accessibility automation is ready.".to_string());
                }

                Ok(json!({
                    "result": {
                        "summary": {
                            "overall_ready": overall_ready,
                            "platform_supported": payload.result.platform_supported,
                            "session_active": payload.result.session.active,
                            "screen_capture_ready": screen_ready,
                            "device_control_ready": control_ready,
                            "input_monitoring_ready": monitoring_ready
                        },
                        "permissions": permissions,
                        "features": payload.result.features,
                        "recommendations": recommendations
                    },
                    "logs": payload.logs
                }))
            }
            AccessibilityCommand::RequestPermissions => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_request_permissions(),
                )
                .await
            }
            AccessibilityCommand::RequestPermission(args) => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_request_permission(
                        parse_params(json!({ "permission": args.permission }))?,
                    ),
                )
                .await
            }
            AccessibilityCommand::StartSession(args) => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_start_session(
                        parse_params(json!({
                            "consent": args.consent,
                            "ttl_secs": args.ttl_secs,
                            "screen_monitoring": args.screen_monitoring,
                            "device_control": args.device_control,
                            "predictive_input": args.predictive_input,
                        }))?,
                    ),
                )
                .await
            }
            AccessibilityCommand::StopSession(args) => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_stop_session(
                        parse_params(json!({ "reason": args.reason }))?,
                    ),
                )
                .await
            }
            AccessibilityCommand::CaptureNow => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_capture_now(),
                )
                .await
            }
            AccessibilityCommand::CaptureImageRef => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_capture_image_ref(),
                )
                .await
            }
            AccessibilityCommand::VisionRecent(args) => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_vision_recent(
                        args.limit,
                    ),
                )
                .await
            }
            AccessibilityCommand::VisionFlush => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_vision_flush(),
                )
                .await
            }
        },
        CoreCommand::Autocomplete { command } => match command {
            AutocompleteCommand::Status => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::autocomplete::rpc::autocomplete_status(),
                )
                .await
            }
            AutocompleteCommand::Start(args) => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::autocomplete::rpc::autocomplete_start(parse_params(
                        json!({ "debounce_ms": args.debounce_ms }),
                    )?),
                )
                .await
            }
            AutocompleteCommand::Stop(args) => {
                rpc_outcome_fut_to_cli_json(crate::openhuman::autocomplete::rpc::autocomplete_stop(
                    Some(parse_params(json!({ "reason": args.reason }))?),
                ))
                .await
            }
            AutocompleteCommand::Current(args) => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::autocomplete::rpc::autocomplete_current(Some(parse_params(
                        json!({ "context": args.context }),
                    )?)),
                )
                .await
            }
            AutocompleteCommand::Accept(args) => {
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::autocomplete::rpc::autocomplete_accept(parse_params(
                        json!({ "suggestion": args.suggestion }),
                    )?),
                )
                .await
            }
            AutocompleteCommand::SetStyle(args) => {
                let style_examples = (!args.style_example.is_empty()).then_some(args.style_example);
                let disabled_apps = (!args.disabled_app.is_empty()).then_some(args.disabled_app);
                rpc_outcome_fut_to_cli_json(
                    crate::openhuman::autocomplete::rpc::autocomplete_set_style(parse_params(
                        json!({
                            "enabled": args.enabled,
                            "debounce_ms": args.debounce_ms,
                            "max_chars": args.max_chars,
                            "style_preset": args.style_preset,
                            "style_instructions": args.style_instructions,
                            "style_examples": style_examples,
                            "disabled_apps": disabled_apps,
                            "accept_with_tab": args.accept_with_tab,
                        }),
                    )?),
                )
                .await
            }
        },
        CoreCommand::Auth { command } => match command {
            AuthCommand::Login(args) => {
                let provider = args.provider.trim().to_string();
                let token = args.token.clone().unwrap_or_default();
                let fields = parse_key_value_flags(&args.field)?;
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;

                if provider == APP_SESSION_PROVIDER {
                    let user = match args.user_json {
                        Some(raw) => Some(parse_json_arg(&raw)?),
                        None => None,
                    };
                    rpc_outcome_fut_to_cli_json(credentials::rpc::store_session(
                        &config,
                        &token,
                        args.user_id.clone(),
                        user,
                    ))
                    .await
                } else {
                    let fields_opt = match &fields {
                        serde_json::Value::Object(map) if map.is_empty() => None,
                        _ => Some(fields),
                    };
                    rpc_outcome_fut_to_cli_json(credentials::rpc::store_provider_credentials(
                        &config,
                        &provider,
                        args.profile.as_deref(),
                        args.token.clone(),
                        fields_opt,
                        Some(args.set_active),
                    ))
                    .await
                }
            }
            AuthCommand::Logout(args) => {
                let provider = args.provider.trim().to_string();
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
                if provider == APP_SESSION_PROVIDER {
                    rpc_outcome_fut_to_cli_json(credentials::rpc::clear_session(&config)).await
                } else {
                    rpc_outcome_fut_to_cli_json(credentials::rpc::remove_provider_credentials(
                        &config,
                        &provider,
                        args.profile.as_deref(),
                    ))
                    .await
                }
            }
            AuthCommand::Status(args) => {
                let provider = args.provider.trim().to_string();
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
                if provider == APP_SESSION_PROVIDER {
                    rpc_outcome_fut_to_cli_json(credentials::rpc::auth_get_state(&config)).await
                } else {
                    rpc_outcome_fut_to_cli_json(credentials::rpc::list_provider_credentials(
                        &config,
                        Some(provider),
                    ))
                    .await
                }
            }
            AuthCommand::List(args) => {
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
                let filter = args
                    .provider
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                rpc_outcome_fut_to_cli_json(credentials::rpc::list_provider_credentials(
                    &config, filter,
                ))
                .await
            }
        },
        CoreCommand::Socket { command } => match command {
            SocketCommand::Connect(args) => {
                call_method(
                    "openhuman.socket.connect",
                    json!({
                        "url": args.url,
                        "token": args.token
                    }),
                )
                .await
            }
            SocketCommand::Disconnect => {
                call_method("openhuman.socket.disconnect", json!({})).await
            }
            SocketCommand::Status => call_method("openhuman.socket.state", json!({})).await,
            SocketCommand::Emit(args) => {
                call_method(
                    "openhuman.socket.emit",
                    json!({
                        "event": args.event,
                        "data": parse_json_arg(&args.data)?
                    }),
                )
                .await
            }
        },
        CoreCommand::Tools { command } => match command {
            ToolsCommand::List => Ok(json!({
                "result": {
                    "wrappers": [
                        {
                            "name": "screenshot",
                            "description": "Capture a screenshot with screenshot tool wrapper."
                        },
                        {
                            "name": "screenshot-ref",
                            "description": "Capture data URL from screen intelligence capture_image_ref."
                        }
                    ]
                },
                "logs": ["tools wrappers listed"]
            })),
            ToolsCommand::Screenshot(args) => execute_tools_screenshot(args).await,
            ToolsCommand::ScreenshotRef(args) => execute_tools_screenshot_ref(args).await,
            ToolsCommand::Run(args) => {
                let parsed = parse_json_arg(&args.args)?;
                match args.name.as_str() {
                    "screenshot" => {
                        let payload = parsed.as_object().cloned().unwrap_or_default();
                        let wrapped = ToolsScreenshotArgs {
                            filename: payload
                                .get("filename")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string),
                            region: payload
                                .get("region")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string),
                            output: payload
                                .get("output")
                                .and_then(serde_json::Value::as_str)
                                .map(PathBuf::from),
                            print_data_url: payload
                                .get("print_data_url")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false),
                        };
                        execute_tools_screenshot(wrapped).await
                    }
                    "screenshot-ref" | "screenshot_ref" => {
                        let payload = parsed.as_object().cloned().unwrap_or_default();
                        let wrapped = ToolsScreenshotRefArgs {
                            output: payload
                                .get("output")
                                .and_then(serde_json::Value::as_str)
                                .map(PathBuf::from),
                            print_data_url: payload
                                .get("print_data_url")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false),
                        };
                        execute_tools_screenshot_ref(wrapped).await
                    }
                    other => Err(format!(
                        "unsupported tool wrapper '{other}'. available: screenshot, screenshot-ref"
                    )),
                }
            }
        },
        CoreCommand::Config { command } => match command {
            ConfigCommand::Get => {
                rpc_outcome_fut_to_cli_json(config_rpc::load_and_get_config_snapshot()).await
            }
            ConfigCommand::UpdateModel { json } => {
                let update: ModelSettingsUpdate = parse_params(parse_json_arg(&json)?)?;
                rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_model_settings(
                    update.into(),
                ))
                .await
            }
            ConfigCommand::UpdateMemory { json } => {
                let update: MemorySettingsUpdate = parse_params(parse_json_arg(&json)?)?;
                rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_memory_settings(
                    update.into(),
                ))
                .await
            }
            ConfigCommand::UpdateGateway { json } => {
                let update: GatewaySettingsUpdate = parse_params(parse_json_arg(&json)?)?;
                rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_gateway_settings(
                    update.into(),
                ))
                .await
            }
            ConfigCommand::UpdateRuntime { json } => {
                let update: RuntimeSettingsUpdate = parse_params(parse_json_arg(&json)?)?;
                rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_runtime_settings(
                    update.into(),
                ))
                .await
            }
            ConfigCommand::UpdateBrowser { json } => {
                let update: BrowserSettingsUpdate = parse_params(parse_json_arg(&json)?)?;
                rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_browser_settings(
                    update.into(),
                ))
                .await
            }
            ConfigCommand::UpdateTunnel { json } => {
                let tunnel: TunnelConfig = parse_params(parse_json_arg(&json)?)?;
                rpc_outcome_fut_to_cli_json(config_rpc::load_and_apply_tunnel_settings(tunnel))
                    .await
            }
        },
    }
}

pub fn run_from_cli_args(args: &[String]) -> Result<()> {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push("openhuman".to_string());
    argv.extend(args.iter().cloned());
    let cli = CoreCli::try_parse_from(argv).map_err(|e| anyhow::anyhow!(e.render().to_string()))?;

    let thread_stack_size = std::env::var("OPENHUMAN_CORE_THREAD_STACK_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8 * 1024 * 1024);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(thread_stack_size)
        .enable_all()
        .build()?;
    let output = runtime
        .block_on(execute_core_cli(cli))
        .map_err(anyhow::Error::msg)?;
    if !output.is_null() {
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_else(|_| "null".to_string())
        );
    }
    Ok(())
}
