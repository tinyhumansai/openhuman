//! Subprocess lifecycle for the Claude Agent SDK provider.

use anyhow::Context;
use async_trait::async_trait;
use tinyagents::error::TinyAgentsError;
use tinyagents::harness::message::Message;
use tinyagents::harness::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinyagents::harness::tool::{coalesce_prompt_tool_results, with_prompt_tool_instructions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::protocol::SdkMessage;
use crate::openhuman::config::schema::claude_agent_sdk::ClaudeAgentSdkConfig;

pub struct ClaudeAgentSdkProvider {
    pub(super) config: ClaudeAgentSdkConfig,
    profile: ModelProfile,
}

struct ClaudeInvocation {
    args: Vec<String>,
    stdin: String,
}

fn build_invocation(
    system_prompt: Option<&str>,
    message: &str,
    model: &str,
    max_budget_usd: Option<f64>,
) -> ClaudeInvocation {
    let stdin = match system_prompt {
        Some(system) if !system.trim().is_empty() => {
            format!("[SYSTEM]\n{system}\n[/SYSTEM]\n\n{message}")
        }
        _ => message.to_string(),
    };
    let mut args = vec![
        "-p".to_string(),
        "--model".to_string(),
        model.to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--no-color".to_string(),
    ];
    if let Some(budget) = max_budget_usd {
        args.push("--max-turns".to_string());
        args.push("10".to_string());
        args.push("--budget".to_string());
        args.push(format!("{budget:.4}"));
    }
    ClaudeInvocation { args, stdin }
}

fn spawn_error(binary: &str, source: std::io::Error) -> anyhow::Error {
    let message = format!("failed to spawn claude binary '{binary}': {source}");
    anyhow::Error::new(source).context(message)
}

impl ClaudeAgentSdkProvider {
    pub fn new(config: ClaudeAgentSdkConfig) -> Self {
        let model = config.default_model.clone();
        Self::for_model(config, model)
    }

    pub fn for_model(config: ClaudeAgentSdkConfig, model: impl Into<String>) -> Self {
        Self {
            config,
            profile: ModelProfile {
                provider: Some("claude-agent-sdk".to_string()),
                model: Some(model.into()),
                ..Default::default()
            },
        }
    }

    async fn invoke_cli(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
    ) -> anyhow::Result<String> {
        let model = if model.is_empty() {
            &self.config.default_model
        } else {
            model
        };

        // `claude -p` reads stdin in non-interactive mode. Keep the full
        // request out of argv so large harness prompts can spawn on Windows.
        let invocation =
            build_invocation(system_prompt, message, model, self.config.max_budget_usd);

        let mut cmd = Command::new(&self.config.binary);
        cmd.args(&invocation.args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::piped())
            .kill_on_drop(true);

        tracing::debug!(
            "[claude_agent_sdk] spawning claude binary={} model={} message_len={}",
            self.config.binary,
            model,
            invocation.stdin.len()
        );

        let mut child = cmd.spawn().map_err(|source| {
            tracing::warn!(
                error = %source,
                binary = %self.config.binary,
                "[claude_agent_sdk] failed to spawn claude binary"
            );
            spawn_error(&self.config.binary, source)
        })?;

        let mut stdin = child
            .stdin
            .take()
            .context("claude subprocess has no stdin")?;
        stdin
            .write_all(invocation.stdin.as_bytes())
            .await
            .context("failed to write claude request to stdin")?;
        stdin
            .shutdown()
            .await
            .context("failed to close claude subprocess stdin")?;
        drop(stdin);

        let stdout = child
            .stdout
            .take()
            .context("claude subprocess has no stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("claude subprocess has no stderr")?;

        // Drain stderr concurrently to prevent pipe-buffer stalls and capture failure context.
        let stderr_task = tokio::spawn(async move {
            let mut err_lines = BufReader::new(stderr).lines();
            let mut buf = String::new();
            while let Ok(Some(line)) = err_lines.next_line().await {
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&line);
            }
            buf
        });

        let mut lines = BufReader::new(stdout).lines();
        let mut text_parts: Vec<String> = Vec::new();
        let mut result_text: Option<String> = None;
        let mut error_message: Option<String> = None;

        let read_result = timeout(Duration::from_secs(120), async {
            while let Some(line) = lines.next_line().await? {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                tracing::trace!(
                    "[claude_agent_sdk] ndjson line received line_len={}",
                    line.len()
                );
                match serde_json::from_str::<SdkMessage>(&line) {
                    Ok(SdkMessage::Text { text }) => {
                        text_parts.push(text);
                    }
                    Ok(SdkMessage::Result {
                        result,
                        is_error,
                        total_cost_usd,
                    }) => {
                        if let Some(cost) = total_cost_usd {
                            tracing::debug!(
                                "[claude_agent_sdk] request completed total_cost_usd={:.6}",
                                cost
                            );
                        }
                        if is_error {
                            error_message = Some(result.unwrap_or_else(|| {
                                "claude subprocess returned an error".to_string()
                            }));
                        } else {
                            result_text = result;
                        }
                    }
                    Ok(SdkMessage::Error { error }) => {
                        error_message = Some(error.message);
                    }
                    Ok(SdkMessage::Unknown) => {
                        tracing::trace!("[claude_agent_sdk] unknown ndjson message type, skipping");
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            line_len = line.len(),
                            "[claude_agent_sdk] failed to parse ndjson line"
                        );
                    }
                }
            }
            anyhow::Ok(())
        })
        .await;

        match read_result {
            Ok(inner) => inner?,
            Err(_) => {
                let _ = child.kill().await;
                anyhow::bail!("[claude_agent_sdk] subprocess timed out while reading output");
            }
        }

        let status = timeout(Duration::from_secs(30), child.wait())
            .await
            .map_err(|_| {
                anyhow::anyhow!("[claude_agent_sdk] subprocess timed out while waiting for exit")
            })??;
        let stderr_output = stderr_task.await.unwrap_or_default();
        tracing::debug!("[claude_agent_sdk] subprocess exited status={}", status);

        if let Some(err) = error_message {
            anyhow::bail!("[claude_agent_sdk] error from claude CLI: {err}");
        }

        // Use the final result message if present; otherwise join streaming text parts.
        let output = result_text
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| text_parts.join(""));

        if !status.success() && output.is_empty() {
            anyhow::bail!(
                "[claude_agent_sdk] claude subprocess exited with non-zero status {} and no output; stderr={}",
                status,
                stderr_output
            );
        }

        tracing::debug!(
            "[claude_agent_sdk] response collected output_len={}",
            output.len()
        );

        Ok(output)
    }
}

#[async_trait]
impl ChatModel<()> for ClaudeAgentSdkProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        let messages = coalesce_prompt_tool_results(&request.messages);
        let messages = with_prompt_tool_instructions(&messages, &request.tools);
        let system = messages.iter().find_map(|message| match message {
            Message::System(_) => Some(message.text()),
            _ => None,
        });
        let last_user = messages
            .iter()
            .rev()
            .find_map(|message| match message {
                Message::User(_) => Some(message.text()),
                _ => None,
            })
            .unwrap_or_default();
        let model = request
            .model
            .as_deref()
            .or(self.profile.model.as_deref())
            .unwrap_or(&self.config.default_model);
        let output = self
            .invoke_cli(system.as_deref(), &last_user, model)
            .await
            .map_err(|error| TinyAgentsError::Model(error.to_string()))?;

        Ok(
            crate::openhuman::agent::tinyagents::model::prompt_guided_text_response(
                output, &request,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::schema::claude_agent_sdk::ClaudeAgentSdkConfig;

    #[test]
    fn provider_constructs_with_default_config() {
        let config = ClaudeAgentSdkConfig::default();
        let provider = ClaudeAgentSdkProvider::new(config);
        assert_eq!(provider.config.binary, "claude");
        assert_eq!(provider.config.default_model, "claude-sonnet-4-6");
    }

    #[test]
    fn config_default_disabled() {
        let config = ClaudeAgentSdkConfig::default();
        assert!(!config.enabled);
        assert!(config.max_budget_usd.is_none());
    }

    #[test]
    fn large_request_is_delivered_over_stdin_instead_of_argv() {
        let system_prompt = "system instruction\n".repeat(2_500);
        assert!(system_prompt.len() > 32_767);

        let invocation = build_invocation(Some(&system_prompt), "hello", "claude-sonnet-4-6", None);

        assert_eq!(
            invocation.args,
            [
                "-p",
                "--model",
                "claude-sonnet-4-6",
                "--output-format",
                "stream-json",
                "--no-color"
            ]
        );
        assert!(!invocation
            .args
            .iter()
            .any(|arg| arg.contains(&system_prompt)));
        assert_eq!(
            invocation.stdin,
            format!("[SYSTEM]\n{system_prompt}\n[/SYSTEM]\n\nhello")
        );
    }

    #[test]
    fn invocation_preserves_plain_message_and_budget_flags() {
        let invocation = build_invocation(None, "hello", "claude-opus-4-6", Some(1.25));

        assert_eq!(invocation.stdin, "hello");
        assert_eq!(
            &invocation.args[6..],
            ["--max-turns", "10", "--budget", "1.2500"]
        );
    }

    #[test]
    fn spawn_error_message_includes_the_os_source() {
        let source = std::io::Error::from_raw_os_error(206);
        let error = spawn_error(r"C:\Users\test\.local\bin\claude.exe", source);

        assert!(error.to_string().contains("os error 206"));
        assert_eq!(
            error.chain().count(),
            2,
            "io::Error source must be preserved"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn provider_pipes_large_request_to_cli_stdin() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("claude");
        std::fs::write(
            &script,
            r#"#!/bin/sh
cat > "$0.stdin"
printf '%s\n' '{"type":"result","result":"captured","is_error":false}'
"#,
        )
        .expect("write fake claude");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("make fake claude executable");

        let mut config = ClaudeAgentSdkConfig::default();
        config.binary = script.display().to_string();
        let provider = ClaudeAgentSdkProvider::new(config);
        let system_prompt = "system instruction\n".repeat(2_500);

        let output = provider
            .invoke(
                &(),
                ModelRequest::new(vec![
                    Message::system(&system_prompt),
                    Message::user("hello"),
                ])
                .with_model("claude-sonnet-4-6"),
            )
            .await
            .expect("fake claude response")
            .text();

        assert_eq!(output, "captured");
        assert_eq!(
            std::fs::read_to_string(format!("{}.stdin", script.display())).expect("captured stdin"),
            format!("[SYSTEM]\n{system_prompt}\n[/SYSTEM]\n\nhello")
        );
    }

    #[tokio::test]
    async fn provider_spawn_error_includes_the_os_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = ClaudeAgentSdkConfig::default();
        config.binary = dir.path().join("missing-claude").display().to_string();
        let provider = ClaudeAgentSdkProvider::new(config);

        let error = provider
            .invoke_cli(None, "hello", "claude-sonnet-4-6")
            .await
            .expect_err("missing binary must fail");

        assert!(error.to_string().contains("failed to spawn claude binary"));
        assert!(error.to_string().contains("os error"));
        assert_eq!(
            error.chain().count(),
            2,
            "io::Error source must be preserved"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn chat_model_uses_prompt_guided_protocol_and_model_override() {
        use std::os::unix::fs::PermissionsExt;
        use tinyagents::harness::tool::ToolSchema;

        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("claude");
        std::fs::write(
            &script,
            r#"#!/bin/sh
cat > "$0.stdin"
printf '%s\n' "$@" > "$0.args"
printf '%s\n' '{"type":"result","result":"Calling.<tool_call>{\"name\":\"lookup\",\"arguments\":{\"query\":\"needle\"}}</tool_call>","is_error":false}'
"#,
        )
        .expect("write fake claude");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("make fake claude executable");

        let mut config = ClaudeAgentSdkConfig::default();
        config.binary = script.display().to_string();
        let provider = ClaudeAgentSdkProvider::for_model(config, "profile-model");
        let request = ModelRequest {
            messages: vec![
                Message::system("Base system"),
                Message::user("original question"),
                Message::assistant("calling"),
                Message::tool("call-1", "first result"),
                Message::tool("call-2", "second result"),
            ],
            tools: vec![ToolSchema::new(
                "lookup",
                "looks up data",
                serde_json::json!({
                    "type": "object",
                    "properties": { "query": { "type": "string" } }
                }),
            )],
            model: Some("request-model".to_string()),
            ..Default::default()
        };

        let response = provider
            .invoke(&(), request)
            .await
            .expect("fake claude response");

        assert_eq!(response.text(), "Calling.");
        assert_eq!(response.message.tool_calls.len(), 1);
        assert_eq!(response.message.tool_calls[0].name, "lookup");
        assert_eq!(
            response.message.tool_calls[0].arguments,
            serde_json::json!({"query": "needle"})
        );
        let stdin =
            std::fs::read_to_string(format!("{}.stdin", script.display())).expect("captured stdin");
        assert!(stdin.contains("Base system"));
        assert!(stdin.contains("## Tool Use Protocol"));
        assert!(
            stdin.contains("[Tool results]\n<tool_result>\nfirst result\n</tool_result>"),
            "unexpected CLI stdin: {stdin:?}"
        );
        assert!(stdin.ends_with("<tool_result>\nsecond result\n</tool_result>"));
        let args =
            std::fs::read_to_string(format!("{}.args", script.display())).expect("captured args");
        assert!(args.contains("request-model"));
        assert_eq!(
            provider
                .profile()
                .and_then(|profile| profile.model.as_deref()),
            Some("profile-model")
        );
        assert_eq!(
            provider
                .profile()
                .and_then(|profile| profile.provider.as_deref()),
            Some("claude-agent-sdk")
        );
    }
}
