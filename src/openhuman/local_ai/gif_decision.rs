//! GIF decision via local AI model + Tenor search via the backend API.

use serde_json::Value;

use crate::api::config::effective_api_url;
use crate::api::jwt::get_session_token;
use crate::api::rest::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::openhuman::local_ai;
use crate::rpc::RpcOutcome;

// ---------------------------------------------------------------------------
// GIF decision — local model decides whether a GIF response is appropriate
// ---------------------------------------------------------------------------

/// Result of the GIF-decision prompt.
#[derive(Debug, serde::Serialize)]
pub struct GifDecision {
    /// Whether the model thinks sending a GIF is appropriate right now.
    pub should_send_gif: bool,
    /// Tenor search query (only meaningful when `should_send_gif` is true).
    pub search_query: Option<String>,
}

/// Ask the local model whether the assistant should respond with a GIF,
/// based on channel type and message content. Designed to be called every
/// ~5-10 messages, not on every message. Lightweight: ~12 output tokens.
pub async fn local_ai_should_send_gif(
    config: &Config,
    message: &str,
    channel_type: &str,
) -> Result<RpcOutcome<GifDecision>, String> {
    tracing::debug!(
        channel_type,
        msg_len = message.len(),
        "[local_ai:gif] evaluating gif decision"
    );

    if message.trim().is_empty() {
        return Ok(RpcOutcome::single_log(
            GifDecision {
                should_send_gif: false,
                search_query: None,
            },
            "empty message — no gif",
        ));
    }

    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        tracing::debug!("[local_ai:gif] local model not ready, skipping");
        return Ok(RpcOutcome::single_log(
            GifDecision {
                should_send_gif: false,
                search_query: None,
            },
            "local model not ready",
        ));
    }

    let prompt = format!(
        "You decide whether an AI assistant should respond with a GIF.\n\
         GIFs are appropriate for: humor, celebration, empathy, reactions to exciting news, \
         casual banter in friendly channels.\n\
         GIFs are NOT appropriate for: technical questions, serious topics, first messages, \
         professional channels (slack, email), or when the user seems upset or frustrated.\n\n\
         Channel: {channel_type}\nUser message: {message}\n\n\
         Reply with EXACTLY one line:\n\
         NONE (no GIF) OR a 2-4 word Tenor search query for a fitting GIF."
    );

    let output = service.prompt(config, &prompt, Some(12), true).await;

    let decision = match output {
        Ok(raw) => {
            let trimmed = raw.trim();
            tracing::debug!(
                response = %trimmed,
                "[local_ai:gif] model response"
            );
            parse_gif_response(trimmed)
        }
        Err(e) => {
            tracing::debug!(error = %e, "[local_ai:gif] inference failed, skipping");
            GifDecision {
                should_send_gif: false,
                search_query: None,
            }
        }
    };

    tracing::debug!(
        should_send = decision.should_send_gif,
        query = ?decision.search_query,
        "[local_ai:gif] decision"
    );
    Ok(RpcOutcome::single_log(decision, "gif decision completed"))
}

/// Parse the model's response into a `GifDecision`.
fn parse_gif_response(text: &str) -> GifDecision {
    let trimmed = text.trim();

    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("NONE")
        || trimmed.eq_ignore_ascii_case("no gif")
    {
        return GifDecision {
            should_send_gif: false,
            search_query: None,
        };
    }

    // The model should return a short search query. Sanity-check length:
    // reject anything too long (probably the model rambled) or too short.
    let word_count = trimmed.split_whitespace().count();
    if word_count > 8 || trimmed.len() > 80 {
        tracing::debug!(
            words = word_count,
            len = trimmed.len(),
            "[local_ai:gif] response too long, treating as NONE"
        );
        return GifDecision {
            should_send_gif: false,
            search_query: None,
        };
    }

    GifDecision {
        should_send_gif: true,
        search_query: Some(trimmed.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Tenor search — proxy through the backend API
// ---------------------------------------------------------------------------

/// A single GIF result from Tenor.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TenorGifResult {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub content_description: String,
    pub url: String,
    #[serde(default)]
    pub media: Value,
    #[serde(default)]
    pub created: i64,
}

/// Wrapper for the Tenor search response.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TenorSearchResult {
    pub results: Vec<TenorGifResult>,
    #[serde(default)]
    pub next: String,
}

/// Search for GIFs via the backend's Tenor proxy endpoint.
/// Requires a valid session JWT (the backend charges against user budget).
pub async fn tenor_search(
    config: &Config,
    query: &str,
    limit: Option<u32>,
) -> Result<RpcOutcome<TenorSearchResult>, String> {
    tracing::debug!(
        query,
        limit = ?limit,
        "[local_ai:gif] searching tenor"
    );

    if query.trim().is_empty() {
        return Err("query is required".to_string());
    }

    let api_url = effective_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let raw = client
        .search_tenor_gifs(&jwt, query, limit)
        .await
        .map_err(|e| format!("tenor search failed: {e}"))?;

    tracing::debug!(
        result_keys = ?raw.as_object().map(|o| o.keys().collect::<Vec<_>>()),
        "[local_ai:gif] tenor search response received"
    );

    // The backend wraps results in { success, data: { results, next, costUsd } }.
    // Extract the inner data.
    let data = raw
        .get("data")
        .cloned()
        .unwrap_or_else(|| raw.clone());

    let result: TenorSearchResult = serde_json::from_value(data).map_err(|e| {
        tracing::debug!(error = %e, "[local_ai:gif] failed to parse tenor response");
        format!("parse tenor response: {e}")
    })?;

    tracing::debug!(
        count = result.results.len(),
        "[local_ai:gif] tenor returned {} results",
        result.results.len()
    );

    Ok(RpcOutcome::single_log(result, "tenor search completed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_none_response() {
        let d = parse_gif_response("NONE");
        assert!(!d.should_send_gif);
        assert!(d.search_query.is_none());
    }

    #[test]
    fn parse_none_case_insensitive() {
        let d = parse_gif_response("none");
        assert!(!d.should_send_gif);
    }

    #[test]
    fn parse_empty_response() {
        let d = parse_gif_response("");
        assert!(!d.should_send_gif);
    }

    #[test]
    fn parse_valid_query() {
        let d = parse_gif_response("happy dance celebration");
        assert!(d.should_send_gif);
        assert_eq!(d.search_query.as_deref(), Some("happy dance celebration"));
    }

    #[test]
    fn parse_short_query() {
        let d = parse_gif_response("thumbs up");
        assert!(d.should_send_gif);
        assert_eq!(d.search_query.as_deref(), Some("thumbs up"));
    }

    #[test]
    fn parse_too_long_response() {
        let long = "this is a very long response that the model should not have generated because it rambled on and on";
        let d = parse_gif_response(long);
        assert!(!d.should_send_gif);
    }

    #[test]
    fn parse_no_gif_variant() {
        let d = parse_gif_response("no gif");
        assert!(!d.should_send_gif);
    }
}
