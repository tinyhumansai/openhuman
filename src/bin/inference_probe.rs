//! Direct chat probe — exercise the full orchestrator harness end-to-end
//! with the live user config, run a single turn, and print whether the
//! harness actually emitted tool calls.
//!
//! Two modes:
//!
//! - `--mode harness` (default): build a real `Agent::from_config()`
//!   orchestrator and call `run_single("...")`. This is the production
//!   path — system prompt with tool catalog, Connected Integrations
//!   block, dispatcher selection, tool execution, all of it. Use this
//!   to verify whether tool calls fire for a given user prompt.
//!
//! - `--mode raw`: send a single hand-built request straight to the
//!   chat provider (no harness, no real tools). Useful to isolate
//!   "does the model itself follow P-format / native tool spec".
//!
//! # Usage
//!
//! ```sh
//! # Drive a real orchestrator turn — needs BACKEND_URL set so the
//! # integrations client can fetch the user's Connected Integrations.
//! BACKEND_URL=https://staging-api.tinyhumans.ai \
//!   OPENHUMAN_APP_ENV=staging \
//!   RUST_LOG=info,openhuman_core::openhuman::agent=debug,openhuman_core::openhuman::inference=debug \
//!   cargo run --bin inference-probe -- \
//!     --mode harness --prompt "hey list my top 5 emails"
//!
//! # Raw provider call (no harness):
//! cargo run --bin inference-probe -- --mode raw --raw-mode pformat
//! ```
use anyhow::{Context, Result};
use clap::Parser;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::inference::provider::create_chat_provider;
use openhuman_core::openhuman::inference::provider::traits::{ChatMessage, ChatRequest};
use openhuman_core::openhuman::tools::traits::ToolSpec;
use serde_json::json;

#[derive(Parser, Debug)]
#[command(name = "inference-probe")]
struct Args {
    /// "harness" — drive the real orchestrator harness end-to-end.
    /// "raw" — send a hand-built request directly to the chat provider.
    #[arg(long, default_value = "harness")]
    mode: String,

    /// Provider role for `--mode raw`. Ignored in harness mode.
    #[arg(long, default_value = "reasoning")]
    role: String,

    /// For `--mode raw`: "pformat" (no tools field, P-format protocol in
    /// system prompt) or "native" (structured `tools` + `tool_choice: auto`).
    #[arg(long, default_value = "pformat")]
    raw_mode: String,

    /// User prompt to send.
    #[arg(long, default_value = "hey list my top 5 emails")]
    prompt: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    let config = Config::load_or_init()
        .await
        .context("loading user config (Config::load_or_init)")?;

    eprintln!("[probe] config.default_model = {:?}", config.default_model);
    eprintln!(
        "[probe] config.agent.tool_dispatcher = {:?}",
        config.agent.tool_dispatcher
    );

    match args.mode.as_str() {
        "harness" => run_harness(&config, &args.prompt).await,
        "raw" => run_raw(&config, &args.role, &args.raw_mode, &args.prompt).await,
        other => anyhow::bail!("unknown --mode {other:?}; want 'harness' or 'raw'"),
    }
}

async fn run_harness(config: &Config, prompt: &str) -> Result<()> {
    eprintln!("[probe] mode = harness (orchestrator)");
    eprintln!("[probe] prompt = {prompt:?}");

    let mut agent = Agent::from_config(config).context("Agent::from_config failed")?;

    // Best-effort: refresh connected integrations so the orchestrator
    // tool catalog includes the user's live toolkits before the turn.
    eprintln!("[probe] fetching connected integrations...");
    agent.fetch_connected_integrations().await;
    let refreshed = agent.refresh_delegation_tools();
    let conn_count = agent.connected_integrations().len();
    eprintln!(
        "[probe] connected_integrations = {} (delegation tools refreshed = {})",
        conn_count, refreshed
    );
    eprintln!("[probe] visible tool count = {}", agent.tools().len());
    eprintln!("[probe] model = {}", agent.model_name());

    eprintln!("[probe] >>> agent.run_single() ...");
    let started = std::time::Instant::now();
    let response = agent
        .run_single(prompt)
        .await
        .context("agent.run_single failed")?;
    let elapsed = started.elapsed();
    eprintln!("[probe] <<< returned in {elapsed:?}");
    eprintln!();
    println!("=== FINAL ASSISTANT REPLY ===");
    println!("{response}");
    println!("=== /FINAL ASSISTANT REPLY ===");
    Ok(())
}

async fn run_raw(config: &Config, role: &str, raw_mode: &str, prompt: &str) -> Result<()> {
    let (provider, model_name) =
        create_chat_provider(role, config).context("create_chat_provider failed")?;
    eprintln!("[probe] mode = raw");
    eprintln!("[probe] role = {role}");
    eprintln!("[probe] resolved model = {model_name}");
    eprintln!(
        "[probe] provider.supports_native_tools() = {}",
        provider.supports_native_tools()
    );
    eprintln!("[probe] raw_mode = {raw_mode}");

    let pformat_system = r#"You are a helpful assistant with access to tools.

## Tools

### get_weather
Get the current weather for a city.
Call as: get_weather[city|unit]

### list_files
List files in a directory.
Call as: list_files[path]

## Tool Use Protocol

Emit tool calls as `<tool_call>name[arg1|arg2]</tool_call>` blocks.
"#;

    let native_system =
        "You are a helpful assistant. Use the provided tools when the user asks something \
         a tool can answer.";

    let (system, tools_for_request): (&str, Option<Vec<ToolSpec>>) = match raw_mode {
        "native" => (
            native_system,
            Some(vec![
                ToolSpec {
                    name: "get_weather".into(),
                    description: "Get the current weather for a city.".into(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "city": {"type": "string"},
                            "unit": {"type": "string", "enum": ["metric", "imperial"]}
                        },
                        "required": ["city"]
                    }),
                },
                ToolSpec {
                    name: "list_files".into(),
                    description: "List files in a directory.".into(),
                    parameters: json!({
                        "type": "object",
                        "properties": {"path": {"type": "string"}},
                        "required": ["path"]
                    }),
                },
            ]),
        ),
        "pformat" => (pformat_system, None),
        other => anyhow::bail!("unknown --raw-mode {other:?}"),
    };

    let messages = vec![
        ChatMessage::system(system),
        ChatMessage::user(prompt.to_string()),
    ];

    let request = ChatRequest {
        messages: &messages,
        tools: tools_for_request.as_deref(),
        stream: None,
    };

    eprintln!("[probe] >>> raw provider.chat()...");
    let started = std::time::Instant::now();
    let response = provider
        .chat(request, &model_name, 0.4)
        .await
        .context("provider.chat() failed")?;
    let elapsed = started.elapsed();
    eprintln!("[probe] <<< {elapsed:?}");

    println!();
    println!("=== RAW RESPONSE TEXT ===");
    println!("{}", response.text_or_empty());
    println!("=== /RAW RESPONSE TEXT ===");

    let text = response.text_or_empty();
    let saw_pformat = text.contains("<tool_call>") || text.contains("<toolcall>");
    let saw_native = !response.tool_calls.is_empty();
    eprintln!(
        "[probe] response.tool_calls.len() = {}",
        response.tool_calls.len()
    );
    eprintln!(
        "[probe] response contains <tool_call> tag = {}",
        saw_pformat
    );
    if saw_native {
        for tc in &response.tool_calls {
            // Print tool name + arg byte-count only — raw arguments may
            // contain user content / secrets and must not land in logs.
            eprintln!(
                "[probe]   native tool_call name={} arg_bytes={}",
                tc.name,
                tc.arguments.len()
            );
        }
    }
    if !saw_pformat && !saw_native {
        eprintln!("[probe] VERDICT: ZERO tool calls.");
    }
    Ok(())
}
