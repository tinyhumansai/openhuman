use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
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
    load_repl_dotenv();
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
    let mut session_id = start_repl_session(&mut rpc).await?;
    println!("Agent session: {session_id}");

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
            match handle_command(line, &mut mode, &mut transcript, &mut rpc, &session_id).await {
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
                let result = match rpc
                    .call(
                        "openhuman.agent_repl_session_chat",
                        json!({ "session_id": session_id.as_str(), "message": line }),
                    )
                    .await
                {
                    Ok(result) => result,
                    Err(err) => {
                        eprintln!("{err}");
                        eprintln!("[repl] attempting to recover by starting a new agent session");
                        match start_repl_session(&mut rpc).await {
                            Ok(new_session_id) => {
                                session_id = new_session_id;
                                println!("[repl] switched to new agent session: {session_id}");
                                match rpc
                                    .call(
                                        "openhuman.agent_repl_session_chat",
                                        json!({ "session_id": session_id.as_str(), "message": line }),
                                    )
                                    .await
                                {
                                    Ok(result) => result,
                                    Err(retry_err) => {
                                        eprintln!("{retry_err}");
                                        continue;
                                    }
                                }
                            }
                            Err(start_err) => {
                                eprintln!("{start_err}");
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

    let _ = rpc
        .call(
            "openhuman.agent_repl_session_end",
            json!({ "session_id": session_id.as_str() }),
        )
        .await;

    if let Some(handle) = server_task {
        handle.abort();
    }

    Ok(())
}

fn parse_dotenv_value(raw: &str) -> String {
    let raw = raw.trim();
    let unquoted = if raw.len() >= 2
        && ((raw.starts_with('"') && raw.ends_with('"'))
            || (raw.starts_with('\'') && raw.ends_with('\'')))
    {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };
    unquoted.split_once(" #").map_or_else(
        || unquoted.trim().to_string(),
        |(value, _)| value.trim().to_string(),
    )
}

fn resolve_dotenv_path() -> PathBuf {
    if let Ok(path) = std::env::var("OPENHUMAN_REPL_DOTENV") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    PathBuf::from(".env")
}

fn load_repl_dotenv() {
    let env_path = resolve_dotenv_path();
    if !env_path.exists() {
        log::info!(
            "[repl] dotenv not found at {} (set OPENHUMAN_REPL_DOTENV to override)",
            env_path.display()
        );
        return;
    }

    let content = match std::fs::read_to_string(&env_path) {
        Ok(content) => content,
        Err(err) => {
            log::warn!(
                "[repl] failed to read dotenv {}: {}",
                env_path.display(),
                err
            );
            return;
        }
    };

    let mut loaded = 0usize;
    let mut skipped = 0usize;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        if key.is_empty() {
            continue;
        }
        if std::env::var_os(key).is_some() {
            skipped += 1;
            continue;
        }
        let value = parse_dotenv_value(raw_value);
        std::env::set_var(key, value);
        loaded += 1;
    }

    log::info!(
        "[repl] dotenv loaded path={} loaded={} skipped_existing={}",
        env_path.display(),
        loaded,
        skipped
    );
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
    session_id: &str,
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
        let result = rpc
            .call(
                "openhuman.agent_repl_session_reset",
                json!({ "session_id": session_id }),
            )
            .await?;
        let reset = result
            .get("result")
            .and_then(|v| v.get("reset"))
            .or_else(|| result.get("reset"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if reset {
            println!("message transcript and agent session history cleared");
        } else {
            println!("message transcript cleared (agent session was not found)");
        }
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
    println!("  message: plain text sends openhuman.agent_repl_session_chat");
    println!("  settings: plain text is '<method> [json]'");
}

async fn start_repl_session(rpc: &mut RpcClient) -> Result<String, String> {
    let result = rpc
        .call("openhuman.agent_repl_session_start", json!({}))
        .await?;
    result
        .get("result")
        .and_then(|v| v.get("session_id"))
        .or_else(|| result.get("session_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "openhuman.agent_repl_session_start returned no session_id".to_string())
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
    fn normalize_rpc_url_adds_rpc_path() {
        let normalized = normalize_rpc_url("http://127.0.0.1:7788").expect("url");
        assert_eq!(normalized, "http://127.0.0.1:7788/rpc");
    }
}
