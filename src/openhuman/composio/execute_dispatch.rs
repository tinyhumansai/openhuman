//! Shared Composio execute path: prepare args, retry policy, error mapping (#1797).

use std::time::Duration;

use super::auth_retry::{execute_with_auth_retry_inner, AUTH_RETRY_BACKOFF};
use super::client::{direct_execute, ComposioClient, ComposioClientKind};
use super::error_mapping::{format_provider_error, remap_transport_error};
use super::execute_prepare::prepare_execute_arguments;
use super::types::ComposioExecuteResponse;

const SLACK_HISTORY: &str = "SLACK_FETCH_CONVERSATION_HISTORY";
const RATELIMIT_INITIAL_BACKOFF: Duration = Duration::from_secs(2);
const RATELIMIT_MAX_BACKOFF: Duration = Duration::from_secs(30);
const RATELIMIT_MAX_ATTEMPTS: u32 = 6;

pub async fn execute_composio_action(
    client: &ComposioClient,
    tool: &str,
    arguments: Option<serde_json::Value>,
) -> Result<ComposioExecuteResponse, String> {
    let tool = tool.trim();
    if tool.is_empty() {
        return Err("composio: tool slug must not be empty".to_string());
    }

    let prepared = match prepare_execute_arguments(tool, arguments) {
        Ok(args) => args,
        Err(msg) => {
            tracing::debug!(
                tool = %tool,
                error = %msg,
                "[composio][prepare] local validation rejected execute"
            );
            return Err(format_provider_error(tool, &msg));
        }
    };

    tracing::debug!(tool = %tool, "[composio][dispatch] execute_composio_action");
    let resp = match execute_with_retries(client, tool, prepared).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::debug!(tool = %tool, "[composio][dispatch] transport failure");
            return Err(remap_transport_error(tool, &e.to_string()));
        }
    };

    if resp.successful {
        return Ok(resp);
    }

    let raw_err = resp
        .error
        .clone()
        .unwrap_or_else(|| "provider reported failure".to_string());
    Ok(ComposioExecuteResponse {
        error: Some(format_provider_error(tool, &raw_err)),
        ..resp
    })
}

async fn execute_with_retries(
    client: &ComposioClient,
    tool: &str,
    args: serde_json::Value,
) -> anyhow::Result<ComposioExecuteResponse> {
    let mut delay = RATELIMIT_INITIAL_BACKOFF;
    for attempt in 1..=RATELIMIT_MAX_ATTEMPTS {
        let resp = execute_with_auth_retry_inner(
            client,
            tool,
            Some(args.clone()),
            if attempt == 1 {
                AUTH_RETRY_BACKOFF
            } else {
                Duration::ZERO
            },
        )
        .await?;

        if resp.successful {
            return Ok(resp);
        }

        let err_text = resp.error.as_deref().unwrap_or("");
        // Only Slack's conversations.history is allow-listed for transparent
        // rate-limit retries today: it surfaces 429s on bursty agent reads and
        // has stable retry semantics. Other tools surface 429 to the caller
        // (formatted as `[composio:error:rate_limited]`) instead of stalling.
        if tool == SLACK_HISTORY && is_rate_limited(err_text) && attempt < RATELIMIT_MAX_ATTEMPTS {
            tracing::warn!(
                tool = %tool,
                attempt,
                max_attempts = RATELIMIT_MAX_ATTEMPTS,
                sleep_ms = delay.as_millis() as u64,
                "[composio][dispatch] upstream rate limit; backing off (#1797)"
            );
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(RATELIMIT_MAX_BACKOFF);
            continue;
        }

        return Ok(resp);
    }
    unreachable!("loop returns on final attempt");
}

fn is_rate_limited(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("ratelimited")
        || lower.contains("too many requests")
        || lower.contains("429")
}

/// Mode-aware variant: routes through the backend (with auth-retry +
/// rate-limit backoff + error mapping) or the direct tenant client
/// (no auth-retry, but local validation + error mapping still apply).
///
/// Added after #1710's mode-aware client split (`ComposioClientKind`) so
/// the per-action tool surface, dispatcher tool, and RPC `composio_execute`
/// op all share one entry point with consistent error semantics.
pub async fn execute_composio_action_kind(
    kind: ComposioClientKind,
    tool: &str,
    arguments: Option<serde_json::Value>,
    entity_id: &str,
) -> Result<ComposioExecuteResponse, String> {
    let tool_trim = tool.trim();
    if tool_trim.is_empty() {
        return Err("composio: tool slug must not be empty".to_string());
    }

    let prepared = match prepare_execute_arguments(tool_trim, arguments) {
        Ok(args) => args,
        Err(msg) => {
            tracing::debug!(
                tool = %tool_trim,
                error = %msg,
                "[composio][prepare] local validation rejected execute"
            );
            return Err(format_provider_error(tool_trim, &msg));
        }
    };

    match kind {
        ComposioClientKind::Backend(client) => {
            tracing::debug!(tool = %tool_trim, "[composio][dispatch] backend variant");
            let resp = match execute_with_retries(&client, tool_trim, prepared).await {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::debug!(tool = %tool_trim, "[composio][dispatch] transport failure");
                    return Err(remap_transport_error(tool_trim, &e.to_string()));
                }
            };
            Ok(format_response(tool_trim, resp))
        }
        ComposioClientKind::Direct(direct) => {
            tracing::debug!(tool = %tool_trim, "[composio][dispatch] direct variant");
            // Direct path skips auth_retry — the user's stored Composio
            // API key has no backend-side refresh surface and a 401 is a
            // config issue that should surface immediately rather than
            // retry-loop. Local validation + error mapping still apply.
            match direct_execute(&direct, tool_trim, Some(prepared), entity_id).await {
                Ok(resp) => Ok(format_response(tool_trim, resp)),
                Err(e) => Err(remap_transport_error(tool_trim, &e.to_string())),
            }
        }
    }
}

fn format_response(tool: &str, resp: ComposioExecuteResponse) -> ComposioExecuteResponse {
    if resp.successful {
        return resp;
    }
    let raw_err = resp
        .error
        .clone()
        .unwrap_or_else(|| "provider reported failure".to_string());
    ComposioExecuteResponse {
        error: Some(format_provider_error(tool, &raw_err)),
        ..resp
    }
}

#[cfg(test)]
#[path = "execute_dispatch_tests.rs"]
mod tests;
