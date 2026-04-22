//! JSON-RPC handler functions for the notifications domain.
//!
//! Three endpoints:
//!  - `notification_ingest`   — write a new notification, kick off background triage
//!  - `notifications_list`    — paginated query with optional provider / min-score filters
//!  - `notification_mark_read`— mark a single notification as read

use chrono::Utc;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::openhuman::agent::triage::{apply_decision, run_triage, TriggerEnvelope, TriggerSource};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::store;
use super::types::{
    IntegrationNotification, NotificationIngestRequest, NotificationSettings,
    NotificationSettingsUpsertRequest, NotificationStatus,
};

// ─────────────────────────────────────────────────────────────────────────────
// notification_ingest
// ─────────────────────────────────────────────────────────────────────────────

/// Ingest a new notification from an embedded webview integration.
///
/// Writes the record immediately, returns the new `id`, then spawns a
/// background task to run the triage pipeline and back-fill the score.
pub async fn handle_ingest(params: Map<String, Value>) -> Result<Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;

    let req: NotificationIngestRequest = serde_json::from_value(Value::Object(params.clone()))
        .map_err(|e| format!("[notifications::rpc] invalid ingest params: {e}"))?;

    let provider_settings = store::get_settings(&config, &req.provider)
        .map_err(|e| format!("[notifications::rpc] get_settings failed: {e}"))?;
    if !provider_settings.enabled {
        let outcome = RpcOutcome::new(
            json!({ "skipped": true, "reason": "provider_disabled" }),
            vec![],
        );
        return outcome.into_cli_compatible_json();
    }
    let id = Uuid::new_v4().to_string();
    let notification = IntegrationNotification {
        id: id.clone(),
        provider: req.provider.clone(),
        account_id: req.account_id.clone(),
        title: req.title.clone(),
        body: req.body.clone(),
        raw_payload: req.raw_payload.clone(),
        importance_score: None,
        triage_action: None,
        triage_reason: None,
        status: NotificationStatus::Unread,
        received_at: Utc::now(),
        scored_at: None,
    };

    store::insert(&config, &notification)
        .map_err(|e| format!("[notifications::rpc] insert failed: {e}"))?;

    tracing::debug!(
        id = %id,
        provider = %req.provider,
        "[notifications::rpc] ingested notification, spawning triage"
    );

    // Spawn background triage — the ingest RPC returns immediately.
    let id_for_triage = id.clone();
    let config_for_triage = config.clone();
    tokio::spawn(async move {
        let envelope = TriggerEnvelope {
            source: TriggerSource::WebviewIntegration {
                provider: req.provider.clone(),
                account_id: req.account_id.clone().unwrap_or_default(),
            },
            external_id: id_for_triage.clone(),
            display_label: format!(
                "webview/{}/{}",
                req.provider,
                req.account_id.as_deref().unwrap_or("default")
            ),
            payload: serde_json::json!({
                "title": req.title,
                "body": req.body,
                "raw": req.raw_payload,
            }),
            received_at: Utc::now(),
        };

        match run_triage(&envelope).await {
            Ok(triage_run) => {
                let action = triage_run.decision.action.as_str().to_string();
                let reason = triage_run.decision.reason.clone();
                // Map TriageAction → importance score heuristic.
                let score = triage_action_to_score(triage_run.decision.action);

                tracing::info!(
                    id = %id_for_triage,
                    action = %action,
                    score = score,
                    "[notifications::rpc] triage complete"
                );

                if let Err(e) = store::update_triage(
                    &config_for_triage,
                    &id_for_triage,
                    score,
                    &action,
                    &reason,
                ) {
                    tracing::warn!(
                        id = %id_for_triage,
                        error = %e,
                        "[notifications::rpc] failed to persist triage result"
                    );
                }

                // Auto-escalate high-importance notifications to the orchestrator.
                if (action == "escalate" || action == "react")
                    && score >= provider_settings.importance_threshold
                    && provider_settings.route_to_orchestrator
                {
                    if let Err(e) = apply_decision(triage_run, &envelope).await {
                        tracing::warn!(
                            id = %id_for_triage,
                            error = %e,
                            "[notifications::rpc] apply_decision failed"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    id = %id_for_triage,
                    error = %e,
                    "[notifications::rpc] triage pipeline failed"
                );
            }
        }
    });

    let outcome = RpcOutcome::new(json!({ "id": id, "skipped": false }), vec![]);
    outcome.into_cli_compatible_json()
}

// ─────────────────────────────────────────────────────────────────────────────
// notifications_list
// ─────────────────────────────────────────────────────────────────────────────

/// Return paginated notifications.
///
/// Optional params: `provider` (string), `limit` (u64), `offset` (u64),
/// `min_score` (f64).
pub async fn handle_list(params: Map<String, Value>) -> Result<Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;

    let provider = params
        .get("provider")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(50);
    let offset = params
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(0);
    let min_score = params
        .get("min_score")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32);

    let items = store::list(&config, limit, offset, provider.as_deref(), min_score)
        .map_err(|e| format!("[notifications::rpc] list failed: {e}"))?;

    let unread = store::unread_count(&config)
        .map_err(|e| format!("[notifications::rpc] unread_count failed: {e}"))?;

    let outcome = RpcOutcome::new(json!({ "items": items, "unread_count": unread }), vec![]);
    outcome.into_cli_compatible_json()
}

// ─────────────────────────────────────────────────────────────────────────────
// notification_mark_read
// ─────────────────────────────────────────────────────────────────────────────

/// Mark a single notification as read.
pub async fn handle_mark_read(params: Map<String, Value>) -> Result<Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;

    let id = params
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "[notifications::rpc] missing required param 'id'".to_string())?
        .to_string();

    store::mark_read(&config, &id)
        .map_err(|e| format!("[notifications::rpc] mark_read failed: {e}"))?;

    tracing::debug!(id = %id, "[notifications::rpc] marked read");

    let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
    outcome.into_cli_compatible_json()
}

/// Read notification routing settings for a provider.
pub async fn handle_settings_get(params: Map<String, Value>) -> Result<Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let provider = params
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "[notifications::rpc] missing required param 'provider'".to_string())?;
    let settings = store::get_settings(&config, provider)
        .map_err(|e| format!("[notifications::rpc] settings_get failed: {e}"))?;
    let outcome = RpcOutcome::new(json!({ "settings": settings }), vec![]);
    outcome.into_cli_compatible_json()
}

/// Upsert notification routing settings for a provider.
pub async fn handle_settings_set(params: Map<String, Value>) -> Result<Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let req: NotificationSettingsUpsertRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[notifications::rpc] invalid settings_set params: {e}"))?;
    let clamped = NotificationSettings {
        provider: req.provider,
        enabled: req.enabled,
        importance_threshold: req.importance_threshold.clamp(0.0, 1.0),
        route_to_orchestrator: req.route_to_orchestrator,
    };
    store::upsert_settings(&config, &clamped)
        .map_err(|e| format!("[notifications::rpc] settings_set failed: {e}"))?;
    let outcome = RpcOutcome::new(json!({ "ok": true, "settings": clamped }), vec![]);
    outcome.into_cli_compatible_json()
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Map the triage decision to a 0.0–1.0 importance score so the frontend
/// can sort/filter without understanding triage action semantics.
fn triage_action_to_score(action: crate::openhuman::agent::triage::TriageAction) -> f32 {
    use crate::openhuman::agent::triage::TriageAction;
    match action {
        TriageAction::Drop => 0.1,
        TriageAction::Acknowledge => 0.35,
        TriageAction::React => 0.65,
        TriageAction::Escalate => 0.9,
    }
}
