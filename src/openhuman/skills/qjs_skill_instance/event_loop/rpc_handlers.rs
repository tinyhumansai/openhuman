//! RPC message handlers for the skill event loop.
//!
//! Each function handles one RPC method (oauth/complete, auth/complete, etc.)
//! and returns the result as a `Result<serde_json::Value, String>`.

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::openhuman::{memory::MemoryClientRef, skills::quickjs_libs::qjs_ops};

use super::{persist_state_to_memory, MemoryWriteJob};
use crate::openhuman::skills::qjs_skill_instance::js_handlers::{
    handle_js_call, handle_js_void_call,
};

/// Handle `oauth/complete` RPC: inject credential into JS, persist to disk, call onOAuthComplete.
pub(crate) async fn handle_oauth_complete(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    params: serde_json::Value,
    data_dir: &std::path::Path,
) -> Result<serde_json::Value, String> {
    let cred_json = serde_json::to_string(&params).unwrap_or_else(|_| "null".to_string());
    let code = format!(
        r#"(function() {{
            if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {{
                globalThis.oauth.__setCredential({cred});
            }}
            if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {{
                globalThis.state.set('__oauth_credential', {cred});
            }}
        }})()"#,
        cred = cred_json
    );
    ctx.with(|js_ctx| {
        let _ = js_ctx.eval::<rquickjs::Value, _>(code.as_bytes());
    })
    .await;

    let cred_path = data_dir.join("oauth_credential.json");
    if let Err(e) = std::fs::write(&cred_path, &cred_json) {
        log::error!(
            "[skill:{}] Failed to persist OAuth credential: {e}",
            skill_id
        );
    } else {
        log::info!(
            "[skill:{}] OAuth credential persisted to {}",
            skill_id,
            cred_path.display()
        );
    }
    let params_str = serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
    handle_js_call(rt, ctx, "onOAuthComplete", &params_str).await
}

/// Handle `oauth/revoked` RPC: clear credential from JS/disk/memory, call onOAuthRevoked.
pub(crate) async fn handle_oauth_revoked(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    params: serde_json::Value,
    data_dir: &std::path::Path,
    memory_client: &Option<MemoryClientRef>,
) -> Result<serde_json::Value, String> {
    let clear_code = r#"(function() {
        if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {
            globalThis.oauth.__setCredential(null);
        }
        if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {
            globalThis.state.set('__oauth_credential', '');
        }
    })()"#;
    ctx.with(|js_ctx| {
        let _ = js_ctx.eval::<rquickjs::Value, _>(clear_code.as_bytes());
    })
    .await;

    let cred_path = data_dir.join("oauth_credential.json");
    let _ = std::fs::remove_file(&cred_path);
    log::info!(
        "[skill:{}] OAuth credential cleared from store and disk",
        skill_id
    );

    // Fire-and-forget: delete memory for this integration
    if let Some(client) = memory_client.clone() {
        let skill = skill_id.to_string();
        let integration_id = params
            .get("integrationId")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        tokio::spawn(async move {
            if let Err(e) = client.clear_skill_memory(&skill, &integration_id).await {
                log::warn!("[memory] clear_skill_memory failed: {e}");
            } else {
                log::info!("[memory] Cleared memory for {}:{}", skill, integration_id);
            }
        });
    }

    let params_str = serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
    handle_js_void_call(rt, ctx, "onOAuthRevoked", &params_str)
        .await
        .map(|()| serde_json::json!({ "ok": true }))
}

/// Handle `auth/complete` RPC: validate via onAuthComplete first, then inject
/// credentials and persist to disk only on success.
pub(crate) async fn handle_auth_complete(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    params: serde_json::Value,
    data_dir: &std::path::Path,
) -> Result<serde_json::Value, String> {
    let cred_json = serde_json::to_string(&params).unwrap_or_else(|_| "null".to_string());
    let is_managed = params
        .get("mode")
        .and_then(|v| v.as_str())
        .map(|m| m == "managed")
        .unwrap_or(false);

    // Temporarily inject credentials so onAuthComplete can use them for validation
    // (e.g. making test API calls). We'll clear them if validation fails.
    let temp_code = format!(
        r#"(function() {{
            if (typeof globalThis.auth !== 'undefined' && globalThis.auth.__setCredential) {{
                globalThis.auth.__setCredential({cred});
            }}
        }})()"#,
        cred = cred_json
    );
    ctx.with(|js_ctx| {
        let _ = js_ctx.eval::<rquickjs::Value, _>(temp_code.as_bytes());
    })
    .await;

    // Call skill's onAuthComplete lifecycle hook for validation
    let params_str = serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
    let result = handle_js_call(rt, ctx, "onAuthComplete", &params_str).await;

    // Check if validation failed (error return or {status:"error"} response)
    let validation_failed = match &result {
        Err(_) => true,
        Ok(val) => val.get("status").and_then(|s| s.as_str()) == Some("error"),
    };

    if validation_failed {
        // Clear the temporary credential injection
        let clear_code = r#"(function() {
            if (typeof globalThis.auth !== 'undefined' && globalThis.auth.__setCredential) {
                globalThis.auth.__setCredential(null);
            }
        })()"#;
        ctx.with(|js_ctx| {
            let _ = js_ctx.eval::<rquickjs::Value, _>(clear_code.as_bytes());
        })
        .await;
        log::info!(
            "[skill:{}] auth/complete validation failed, credentials not persisted",
            skill_id
        );
        return result;
    }

    // Validation succeeded — now persist credentials into state and disk

    // Build managed-mode bridge code (inject into oauth globals too)
    let managed_bridge = if is_managed {
        let creds_json = serde_json::to_string(
            params
                .get("credentials")
                .unwrap_or(&serde_json::Value::Null),
        )
        .unwrap_or_else(|_| "null".to_string());
        format!(
            r#"
            if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {{
                globalThis.oauth.__setCredential({creds});
            }}
            if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {{
                globalThis.state.set('__oauth_credential', {creds});
            }}"#,
            creds = creds_json
        )
    } else {
        String::new()
    };

    let persist_code = format!(
        r#"(function() {{
            if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {{
                globalThis.state.set('__auth_credential', {cred});
            }}
            {managed_bridge}
        }})()"#,
        cred = cred_json,
        managed_bridge = managed_bridge
    );
    ctx.with(|js_ctx| {
        let _ = js_ctx.eval::<rquickjs::Value, _>(persist_code.as_bytes());
    })
    .await;

    // Persist auth credential to disk
    let cred_path = data_dir.join("auth_credential.json");
    if let Err(e) = std::fs::write(&cred_path, &cred_json) {
        log::error!(
            "[skill:{}] Failed to persist auth credential: {e}",
            skill_id
        );
    } else {
        log::info!(
            "[skill:{}] Auth credential persisted to {}",
            skill_id,
            cred_path.display()
        );
    }

    // For managed mode, also persist as oauth_credential.json for backward compat
    if is_managed {
        let oauth_cred_json = serde_json::to_string(
            params
                .get("credentials")
                .unwrap_or(&serde_json::Value::Null),
        )
        .unwrap_or_else(|_| "null".to_string());
        let oauth_path = data_dir.join("oauth_credential.json");
        if let Err(e) = std::fs::write(&oauth_path, &oauth_cred_json) {
            log::error!(
                "[skill:{}] Failed to persist managed OAuth credential: {e}",
                skill_id
            );
        }
    }

    result
}

/// Handle `auth/revoked` RPC: clear auth credential from JS/disk/memory, call onAuthRevoked.
pub(crate) async fn handle_auth_revoked(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    params: serde_json::Value,
    data_dir: &std::path::Path,
    memory_client: &Option<MemoryClientRef>,
) -> Result<serde_json::Value, String> {
    let is_managed = params
        .get("mode")
        .and_then(|v| v.as_str())
        .map(|m| m == "managed")
        .unwrap_or(false);

    let managed_clear = if is_managed {
        r#"
        if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {
            globalThis.oauth.__setCredential(null);
        }
        if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {
            globalThis.state.set('__oauth_credential', '');
        }"#
    } else {
        ""
    };

    let clear_code = format!(
        r#"(function() {{
            if (typeof globalThis.auth !== 'undefined' && globalThis.auth.__setCredential) {{
                globalThis.auth.__setCredential(null);
            }}
            if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {{
                globalThis.state.set('__auth_credential', '');
            }}
            {managed_clear}
        }})()"#,
        managed_clear = managed_clear
    );
    ctx.with(|js_ctx| {
        let _ = js_ctx.eval::<rquickjs::Value, _>(clear_code.as_bytes());
    })
    .await;

    // Remove persisted credential files
    let cred_path = data_dir.join("auth_credential.json");
    let _ = std::fs::remove_file(&cred_path);
    if is_managed {
        let oauth_path = data_dir.join("oauth_credential.json");
        let _ = std::fs::remove_file(&oauth_path);
    }
    log::info!(
        "[skill:{}] Auth credential cleared from store and disk",
        skill_id
    );

    // Fire-and-forget: delete memory for this integration
    if let Some(client) = memory_client.clone() {
        let skill = skill_id.to_string();
        let integration_id = params
            .get("integrationId")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        tokio::spawn(async move {
            if let Err(e) = client.clear_skill_memory(&skill, &integration_id).await {
                log::warn!("[memory] clear_skill_memory failed: {e}");
            } else {
                log::info!("[memory] Cleared memory for {}:{}", skill, integration_id);
            }
        });
    }

    let params_str = serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
    handle_js_void_call(rt, ctx, "onAuthRevoked", &params_str)
        .await
        .map(|()| serde_json::json!({ "ok": true }))
}

/// Handle `skill/sync` RPC: call onSync, persist state to memory on success.
pub(crate) async fn handle_sync(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    ops_state: &Arc<RwLock<qjs_ops::SkillState>>,
    memory_client: &Option<MemoryClientRef>,
    memory_write_tx: &mpsc::Sender<MemoryWriteJob>,
) -> Result<serde_json::Value, String> {
    let result = handle_js_call(rt, ctx, "onSync", "{}").await;
    if result.is_ok() {
        persist_state_to_memory(
            skill_id,
            "periodic sync",
            ops_state,
            memory_client,
            memory_write_tx,
        );
    }
    result
}
