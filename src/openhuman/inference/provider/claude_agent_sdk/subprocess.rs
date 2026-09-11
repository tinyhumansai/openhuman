//! Subprocess lifecycle for the Claude Agent SDK provider.

use anyhow::Context;
use async_trait::async_trait;
use tinyagents_harness::tool::{coalesce_prompt_tool_results, with_prompt_tool_instructions};
use tinyinference::message::Message;
use tinyinference::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
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
    ) -> tinyinference::Result<ModelResponse> {
        let messages = coalesce_prompt_tool_results(&request.messages);
        let messages = with_prompt_tool_instructions(&messages, &request.tools);
        let system = coalesce_system_prompt(&messages);
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
            .map_err(|error| tinyinference::Error::Model(error.to_string()))?;

        Ok(
            crate::openhuman::agent::tinyagents::model::prompt_guided_text_response(
                output, &request,
            ),
        )
    }
}

/// Join every system message into the one system prompt the CLI accepts.
///
/// The CLI takes a single `--system-prompt`, and this used to `find_map` the
/// first system message and drop the rest (codex on #6068). What gets dropped is
/// not incidental: the artifact contents list and the turn-cap wrap-up are both
/// appended as system messages *precisely because* a system message is the one
/// thing compression and the trim will not remove. Taking only the first put
/// them back to being removable — on this provider alone, and silently.
fn coalesce_system_prompt(messages: &[Message]) -> Option<String> {
    let parts: Vec<String> = messages
        .iter()
        .filter(|message| matches!(message, Message::System(_)))
        .map(|message| message.text())
        .filter(|text| !text.trim().is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

#[cfg(test)]
#[path = "subprocess_tests.rs"]
mod tests;
