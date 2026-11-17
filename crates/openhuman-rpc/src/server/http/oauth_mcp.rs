//! `GET /oauth/mcp/callback`: the loopback redirect target for MCP browser OAuth.

use axum::extract::Query;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use super::pages::{error_html, success_html};

/// Query params for the MCP browser-OAuth callback (`/oauth/mcp/callback`).
#[derive(Debug, serde::Deserialize)]
pub(super) struct OAuthMcpCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Loopback redirect target for MCP browser OAuth (RFC 8252). The authorization
/// server redirects the browser here with `?code=…&state=…`; we hand it to
/// `mcp::registry::oauth::complete`, which exchanges the code for a token, stores
/// it as the server's `Authorization` header, and reconnects.
pub(super) async fn oauth_mcp_callback_handler(
    Query(query): Query<OAuthMcpCallbackQuery>,
) -> impl IntoResponse {
    let html = |status: StatusCode, body: String| -> Response {
        (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response()
    };

    if let Some(err) = query.error.as_deref().filter(|s| !s.is_empty()) {
        let desc = query.error_description.as_deref().unwrap_or("");
        log::warn!("[oauth:mcp] authorization error: {err} {desc}");
        return html(
            StatusCode::BAD_REQUEST,
            error_html(&format!("Authorization was denied or failed: {err} {desc}")),
        );
    }

    let (code, state) = match (
        query
            .code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        query
            .state
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        (Some(c), Some(s)) => (c.to_string(), s.to_string()),
        _ => {
            return html(
                StatusCode::BAD_REQUEST,
                error_html("Missing authorization code or state in the callback."),
            )
        }
    };

    log::info!("[oauth:mcp] callback received (state present); completing exchange");

    let config = match crate::core_host::config::Config::load_or_init().await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[oauth:mcp] config load failed: {e}");
            return html(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_html("Internal error loading config. Please try again."),
            );
        }
    };

    match crate::core_host::mcp::registry::oauth::complete(&config, &state, &code).await {
        Ok(server_id) => {
            log::info!("[oauth:mcp] completed sign-in for server_id={server_id}");
            html(
                StatusCode::OK,
                success_html("Signed in. The MCP server is now connected — you can close this tab and return to OpenHuman."),
            )
        }
        Err(e) => {
            log::warn!("[oauth:mcp] complete failed: {e}");
            html(
                StatusCode::BAD_GATEWAY,
                error_html(&format!("Sign-in could not be completed: {e}")),
            )
        }
    }
}

#[cfg(test)]
#[path = "oauth_mcp_tests.rs"]
mod tests;
