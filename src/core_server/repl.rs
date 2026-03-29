use std::io::{self, Write};
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;
use tokio::task::JoinHandle;
use url::Url;

const DEFAULT_HISTORY_TURNS: usize = 8;
const DEFAULT_REPL_PORT: u16 = 7788;

#[derive(Debug, Clone)]
pub struct ReplOptions {
    pub port: Option<u16>,
    pub rpc_url: Option<String>,
    pub history_turns: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplMode {
    Message,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TranscriptTurn {
    user: String,
    assistant: String,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    result: Option<serde_json::Value>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    #[allow(dead_code)]
    code: i64,
    message: String,
    data: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct RpcClient {
    client: reqwest::Client,
    endpoint: String,
    next_id: u64,
}

impl RpcClient {
    fn new(endpoint: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
            next_id: 1,
        }
    }

    async fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let request_id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let started = std::time::Instant::now();

        let body = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params
        });
        log::info!("[repl] rpc -> method={method} id={request_id}");

        let response = self
            .client
            .post(self.endpoint.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("rpc transport failed: {e}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| format!("rpc response read failed: {e}"))?;
        if !status.is_success() {
            return Err(format!("rpc http error {status}: {text}"));
        }

        let payload: RpcResponse =
            serde_json::from_str(&text).map_err(|e| format!("invalid rpc json: {e}"))?;
        if let Some(error) = payload.error {
            let suffix = error.data.map(|d| format!(" ({d})")).unwrap_or_default();
            return Err(format!("rpc error: {}{}", error.message, suffix));
        }

        let elapsed = started.elapsed().as_millis();
        log::info!("[repl] rpc <- method={method} id={request_id} elapsed_ms={elapsed}");

        payload
            .result
            .ok_or_else(|| "rpc response missing result".to_string())
    }
}

pub async fn run_repl(options: ReplOptions) -> Result<(), String> {
    let endpoint = resolve_rpc_endpoint(options.rpc_url)?;
    let history_turns = options
        .history_turns
        .unwrap_or(DEFAULT_HISTORY_TURNS)
        .clamp(1, 128);

    let mut rpc = RpcClient::new(endpoint.clone());
    let server_task = ensure_core_ready(&mut rpc, &endpoint, options.port).await?;

    println!("OpenHuman REPL");
    println!("Connected RPC endpoint: {endpoint}");
    println!("Type /help for commands.");

    let mut mode = ReplMode::Message;
    let mut transcript: Vec<TranscriptTurn> = Vec::new();

    loop {
        print_prompt(&mode)?;
        let Some(input) = read_line()? else {
            println!();
            break;
        };
        let line = input.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('/') {
            match handle_command(line, &mut mode, &mut transcript, &mut rpc).await {
                Ok(true) => break,
                Ok(false) => {}
                Err(err) => {
                    eprintln!("{err}");
                }
            }
            continue;
        }

        match mode {
            ReplMode::Message => {
                let outgoing = compose_message_payload(&transcript, line);
                let result = match rpc
                    .call("openhuman.agent_chat", json!({ "message": outgoing }))
                    .await
                {
                    Ok(result) => result,
                    Err(err) => {
                        eprintln!("{err}");
                        eprintln!(
                            "[repl] falling back to openhuman.agent_chat_simple (no tool loop)"
                        );
                        match rpc
                            .call(
                                "openhuman.agent_chat_simple",
                                json!({ "message": outgoing }),
                            )
                            .await
                        {
                            Ok(result) => result,
                            Err(fallback_err) => {
                                eprintln!("{fallback_err}");
                                eprintln!("[repl] falling back to direct backend curl transport");
                                match backend_chat_via_curl(&mut rpc, outgoing.as_str()).await {
                                    Ok(text) => {
                                        println!("{text}");
                                        transcript.push(TranscriptTurn {
                                            user: line.to_string(),
                                            assistant: text,
                                        });
                                        while transcript.len() > history_turns {
                                            transcript.remove(0);
                                        }
                                    }
                                    Err(curl_err) => eprintln!("{curl_err}"),
                                }
                                continue;
                            }
                        }
                    }
                };
                print_result(&result);

                if let Some(reply) = extract_agent_reply(&result) {
                    transcript.push(TranscriptTurn {
                        user: line.to_string(),
                        assistant: reply,
                    });
                    while transcript.len() > history_turns {
                        transcript.remove(0);
                    }
                }
            }
            ReplMode::Settings => {
                let (method, params) = parse_method_and_params(line)?;
                let result = match rpc.call(&method, params).await {
                    Ok(result) => result,
                    Err(err) => {
                        eprintln!("{err}");
                        continue;
                    }
                };
                print_result(&result);
            }
        }
    }

    if let Some(handle) = server_task {
        handle.abort();
    }

    Ok(())
}

fn resolve_rpc_endpoint(raw: Option<String>) -> Result<String, String> {
    let initial = raw.unwrap_or_else(|| {
        std::env::var("OPENHUMAN_CORE_RPC_URL")
            .unwrap_or_else(|_| crate::core_server::DEFAULT_CORE_RPC_URL.to_string())
    });
    normalize_rpc_url(&initial)
}

fn normalize_rpc_url(raw: &str) -> Result<String, String> {
    let mut url = Url::parse(raw).map_err(|e| format!("invalid rpc url '{raw}': {e}"))?;
    if url.path().is_empty() || url.path() == "/" {
        url.set_path("/rpc");
    }
    Ok(url.to_string())
}

fn resolve_repl_port(options_port: Option<u16>, endpoint: &Url) -> u16 {
    if let Some(port) = options_port {
        return port;
    }
    if let Some(port) = endpoint.port() {
        return port;
    }
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(DEFAULT_REPL_PORT)
}

fn is_localhost(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("::1")
    )
}

async fn ensure_core_ready(
    rpc: &mut RpcClient,
    endpoint: &str,
    options_port: Option<u16>,
) -> Result<Option<JoinHandle<()>>, String> {
    if ping_rpc(rpc).await {
        return Ok(None);
    }

    let parsed = Url::parse(endpoint).map_err(|e| format!("invalid rpc endpoint: {e}"))?;
    if !is_localhost(&parsed) {
        return Err("core rpc endpoint is not reachable and host is not local; refusing to spawn local server".to_string());
    }

    let port = resolve_repl_port(options_port, &parsed);
    log::info!("[repl] starting in-process core rpc server on port {port}");
    let task = tokio::spawn(async move {
        if let Err(err) = crate::core_server::run_server(Some(port)).await {
            log::error!("[repl] in-process core server exited with error: {err}");
        }
    });

    for _ in 0..80 {
        if ping_rpc(rpc).await {
            return Ok(Some(task));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    task.abort();
    Err("core rpc did not become ready in time".to_string())
}

async fn ping_rpc(rpc: &mut RpcClient) -> bool {
    let Ok(result) = rpc.call("core.ping", json!({})).await else {
        return false;
    };
    result
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn print_prompt(mode: &ReplMode) -> Result<(), String> {
    let prefix = match mode {
        ReplMode::Message => "message",
        ReplMode::Settings => "settings",
    };
    print!("{prefix}> ");
    io::stdout()
        .flush()
        .map_err(|e| format!("failed to flush prompt: {e}"))
}

fn read_line() -> Result<Option<String>, String> {
    let mut input = String::new();
    let bytes = io::stdin()
        .read_line(&mut input)
        .map_err(|e| format!("failed reading stdin: {e}"))?;
    if bytes == 0 {
        return Ok(None);
    }
    Ok(Some(input))
}

fn compose_message_payload(history: &[TranscriptTurn], user_message: &str) -> String {
    if history.is_empty() {
        return user_message.to_string();
    }

    let mut out = String::from(
        "Use this transcript as context for continuity. Keep continuity but answer the latest user message directly.\n\n[Transcript]\n",
    );
    for turn in history {
        out.push_str("User: ");
        out.push_str(turn.user.as_str());
        out.push('\n');
        out.push_str("Assistant: ");
        out.push_str(turn.assistant.as_str());
        out.push('\n');
    }
    out.push_str("\n[Latest User Message]\n");
    out.push_str(user_message);
    out
}

fn extract_agent_reply(result: &serde_json::Value) -> Option<String> {
    if let Some(raw) = result.as_str() {
        return Some(raw.to_string());
    }
    result
        .get("result")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn parse_method_and_params(input: &str) -> Result<(String, serde_json::Value), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("missing method".to_string());
    }

    let mut split = trimmed.splitn(2, char::is_whitespace);
    let method = split.next().unwrap_or_default().trim().to_string();
    if method.is_empty() {
        return Err("missing method".to_string());
    }
    let params_raw = split.next().map(str::trim).unwrap_or("{}");
    let params = serde_json::from_str(params_raw)
        .map_err(|e| format!("invalid params JSON for '{method}': {e}"))?;
    Ok((method, params))
}

async fn handle_command(
    line: &str,
    mode: &mut ReplMode,
    transcript: &mut Vec<TranscriptTurn>,
    rpc: &mut RpcClient,
) -> Result<bool, String> {
    if matches!(line, "/exit" | "/quit") {
        return Ok(true);
    }

    if line == "/help" {
        print_help();
        return Ok(false);
    }

    if line == "/history" {
        if transcript.is_empty() {
            println!("(no transcript)");
        } else {
            for (idx, turn) in transcript.iter().enumerate() {
                println!("{}. user: {}", idx + 1, turn.user);
                println!("   assistant: {}", turn.assistant);
            }
        }
        return Ok(false);
    }

    if line == "/reset" {
        transcript.clear();
        println!("message transcript cleared");
        return Ok(false);
    }

    if let Some(arg) = line.strip_prefix("/mode ") {
        *mode = match arg.trim() {
            "message" => ReplMode::Message,
            "settings" => ReplMode::Settings,
            other => {
                return Err(format!(
                    "unknown mode '{other}', expected: message | settings"
                ))
            }
        };
        println!("mode switched to {}", arg.trim());
        return Ok(false);
    }

    if let Some(raw) = line.strip_prefix("/rpc ") {
        let (method, params) = parse_method_and_params(raw)?;
        let result = rpc.call(&method, params).await?;
        print_result(&result);
        return Ok(false);
    }

    Err("unknown command, use /help".to_string())
}

fn print_result(result: &serde_json::Value) {
    if let Some(logs) = result.get("logs").and_then(serde_json::Value::as_array) {
        for entry in logs {
            if let Some(text) = entry.as_str() {
                println!("[log] {text}");
            }
        }
    }

    if let Some(text) = extract_agent_reply(result) {
        println!("{text}");
        return;
    }

    let pretty = serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string());
    println!("{pretty}");
}

fn print_help() {
    println!("Commands:");
    println!("  /help                       show this help");
    println!("  /mode message|settings      switch input mode");
    println!("  /rpc <method> [json]        execute raw JSON-RPC call");
    println!("  /history                    show local message transcript");
    println!("  /reset                      clear local message transcript");
    println!("  /exit                       exit REPL");
    println!();
    println!("Modes:");
    println!("  message: plain text sends openhuman.agent_chat");
    println!("  settings: plain text is '<method> [json]'");
}

async fn backend_chat_via_curl(rpc: &mut RpcClient, message: &str) -> Result<String, String> {
    let config = rpc.call("openhuman.get_config", json!({})).await?;
    let config_body = config
        .get("result")
        .and_then(|v| v.get("config"))
        .or_else(|| config.get("config"))
        .ok_or_else(|| "unable to read config from openhuman.get_config".to_string())?;

    let api_url = config_body
        .get("api_url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("https://staging-api.alphahuman.xyz");
    let model = config_body
        .get("default_model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("neocortex-mk1");
    let token = resolve_bearer_token(rpc, config_body).await?;
    let endpoint = format!(
        "{}/openai/v1/chat/completions",
        api_url.trim_end_matches('/')
    );
    let payload = json!({
        "model": model,
        "messages": [{"role": "user", "content": message}],
    })
    .to_string();

    let output = Command::new("curl")
        .arg("-sS")
        .arg("--connect-timeout")
        .arg("10")
        .arg("-X")
        .arg("POST")
        .arg(endpoint)
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg(format!("Authorization: Bearer {token}"))
        .arg("-d")
        .arg(payload)
        .output()
        .await
        .map_err(|e| format!("failed to spawn curl fallback: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(format!("curl fallback failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).map_err(|e| format!("invalid curl JSON: {e}"))?;
    value
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "curl fallback response missing choices[0].message.content".to_string())
}

async fn resolve_bearer_token(
    rpc: &mut RpcClient,
    config_body: &serde_json::Value,
) -> Result<String, String> {
    if let Ok(token_response) = rpc
        .call("openhuman.auth.get_session_token", json!({}))
        .await
    {
        if let Some(token) = token_response
            .get("result")
            .and_then(|v| v.get("token"))
            .or_else(|| token_response.get("token"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Ok(token.to_string());
        }
    }

    if let Some(token) = config_body
        .get("api_key")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(token.to_string());
    }

    Err("no bearer token found in auth session or config.api_key".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_method_defaults_to_empty_object() {
        let (method, params) = parse_method_and_params("core.ping").expect("parse");
        assert_eq!(method, "core.ping");
        assert_eq!(params, json!({}));
    }

    #[test]
    fn parse_method_with_json_payload() {
        let (method, params) =
            parse_method_and_params("openhuman.socket.emit {\"event\":\"x\"}").expect("parse");
        assert_eq!(method, "openhuman.socket.emit");
        assert_eq!(params, json!({ "event": "x" }));
    }

    #[test]
    fn compose_message_includes_transcript() {
        let turns = vec![TranscriptTurn {
            user: "hi".to_string(),
            assistant: "hello".to_string(),
        }];
        let payload = compose_message_payload(&turns, "next");
        assert!(payload.contains("User: hi"));
        assert!(payload.contains("Assistant: hello"));
        assert!(payload.contains("[Latest User Message]"));
    }

    #[test]
    fn normalize_rpc_url_adds_rpc_path() {
        let normalized = normalize_rpc_url("http://127.0.0.1:7788").expect("url");
        assert_eq!(normalized, "http://127.0.0.1:7788/rpc");
    }
}
