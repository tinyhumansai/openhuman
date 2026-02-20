//! WhatsApp webhook handlers and signature verification.

use super::webhook::run_gateway_chat_with_multimodal;
use crate::alphahuman::channels::SendMessage;
use crate::alphahuman::channels::traits::Channel;
use crate::alphahuman::gateway::models::WhatsAppVerifyQuery;
use crate::alphahuman::gateway::state::AppState;
use crate::alphahuman::memory::MemoryCategory;
use crate::alphahuman::security::pairing::constant_time_eq;
use crate::alphahuman::util::truncate_with_ellipsis;
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};

/// GET /whatsapp — Meta webhook verification.
pub async fn handle_whatsapp_verify(
    State(state): State<AppState>,
    Query(params): Query<WhatsAppVerifyQuery>,
) -> impl IntoResponse {
    let Some(ref wa) = state.whatsapp else {
        return (StatusCode::NOT_FOUND, "WhatsApp not configured".to_string());
    };

    // Verify the token matches (constant-time comparison to prevent timing attacks)
    let token_matches = params
        .verify_token
        .as_deref()
        .is_some_and(|t| constant_time_eq(t, wa.verify_token()));
    if params.mode.as_deref() == Some("subscribe") && token_matches {
        if let Some(ch) = params.challenge.clone() {
            tracing::info!("WhatsApp webhook verified successfully");
            return (StatusCode::OK, ch);
        }
        return (StatusCode::BAD_REQUEST, "Missing hub.challenge".to_string());
    }

    tracing::warn!("WhatsApp webhook verification failed — token mismatch");
    (StatusCode::FORBIDDEN, "Forbidden".to_string())
}

/// Verify `WhatsApp` webhook signature (`X-Hub-Signature-256`).
/// Returns true if the signature is valid, false otherwise.
/// See: <https://developers.facebook.com/docs/graph-api/webhooks/getting-started#verification-requests>
pub fn verify_whatsapp_signature(app_secret: &str, body: &[u8], signature_header: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    // Signature format: "sha256=<hex_signature>"
    let Some(hex_sig) = signature_header.strip_prefix("sha256=") else {
        return false;
    };

    // Decode hex signature
    let Ok(expected) = hex::decode(hex_sig) else {
        return false;
    };

    // Compute HMAC-SHA256
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(app_secret.as_bytes()) else {
        return false;
    };
    mac.update(body);

    // Constant-time comparison
    mac.verify_slice(&expected).is_ok()
}

/// POST /whatsapp — incoming message webhook.
pub async fn handle_whatsapp_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(ref wa) = state.whatsapp else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "WhatsApp not configured"})),
        );
    };

    // ── Security: Verify X-Hub-Signature-256 if app_secret is configured ──
    if let Some(ref app_secret) = state.whatsapp_app_secret {
        let signature = headers
            .get("X-Hub-Signature-256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !verify_whatsapp_signature(app_secret, &body, signature) {
            tracing::warn!(
                "WhatsApp webhook signature verification failed (signature: {})",
                if signature.is_empty() { "missing" } else { "invalid" }
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid signature"})),
            );
        }
    }

    // Parse JSON body
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid JSON payload"})),
        );
    };

    // Parse messages from the webhook payload
    let messages = wa.parse_webhook_payload(&payload);

    if messages.is_empty() {
        // Acknowledge the webhook even if no messages (could be status updates)
        return (StatusCode::OK, Json(serde_json::json!({"status": "ok"})));
    }

    // Process each message
    let provider_label = state
        .config
        .lock()
        .default_provider
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    for msg in &messages {
        tracing::info!(
            "WhatsApp message from {}: {}",
            msg.sender,
            truncate_with_ellipsis(&msg.content, 50)
        );

        // Auto-save to memory
        if state.auto_save {
            let key = crate::alphahuman::gateway::constants::whatsapp_memory_key(msg);
            let _ = state
                .mem
                .store(&key, &msg.content, MemoryCategory::Conversation, None)
                .await;
        }

        match run_gateway_chat_with_multimodal(&state, &provider_label, &msg.content).await {
            Ok(response) => {
                // Send reply via WhatsApp
                if let Err(e) = wa
                    .send(&SendMessage::new(response, &msg.reply_target))
                    .await
                {
                    tracing::error!("Failed to send WhatsApp reply: {e}");
                }
            }
            Err(e) => {
                tracing::error!("LLM error for WhatsApp message: {e:#}");
                let _ = wa
                    .send(&SendMessage::new(
                        "Sorry, I couldn't process your message right now.",
                        &msg.reply_target,
                    ))
                    .await;
            }
        }
    }

    // Acknowledge the webhook
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}
