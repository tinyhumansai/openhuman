//! RPC message handlers for the skill event loop.
//!
//! This module implements handlers for various JSON-RPC methods targeted at
//! a running skill instance, including authentication flows (OAuth, Auth),
//! synchronization, and revocation events.

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::openhuman::event_bus::{publish_global, DomainEvent};
use crate::openhuman::{memory::MemoryClientRef, skills::quickjs_libs::qjs_ops};

use super::super::js_handlers::{handle_js_call, handle_js_void_call};
use super::{persist_state_to_memory, MemoryWriteJob};

/// Handle `oauth/complete` RPC.
///
/// Mirrors `handle_auth_complete`: temp-inject the new credentials into the
/// JS bridges, ask `start({ oauth, validate: true })` to validate them
/// against the upstream API, and only persist to disk if start() returns
/// `{ status: "complete" }`. There is no separate `onOAuthComplete` JS hook
/// — `start()` is the single entry point for "this skill is now connected".
pub(crate) async fn handle_oauth_complete(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    params: serde_json::Value,
    data_dir: &std::path::Path,
    ops_state: &Arc<RwLock<qjs_ops::SkillState>>,
) -> Result<serde_json::Value, String> {
    let cred_json = serde_json::to_string(&params).unwrap_or_else(|_| "null".to_string());

    // Extract client key share (required for encrypted OAuth proxy requests).
    // We have to inject it before start() runs because the proxy validation
    // call inside start() needs it to encrypt the outgoing request.
    let client_key = params
        .get("clientKeyShare")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let client_key_js = if !client_key.is_empty() {
        format!(
            r#"globalThis.__oauthClientKey = "{key}";"#,
            key = client_key
        )
    } else {
        String::new()
    };

    // Step 1: Temporary injection so the JS `oauth` bridge sees the new
    // credential while start() runs validation through the proxy.
    let inject_code = format!(
        r#"(function() {{
            if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {{
                globalThis.oauth.__setCredential({cred});
            }}
            if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {{
                globalThis.state.set('__oauth_credential', {cred});
            }}
            {client_key_js}
        }})()"#,
        cred = cred_json,
        client_key_js = client_key_js,
    );
    ctx.with(|js_ctx| {
        let _ = js_ctx.eval::<rquickjs::Value, _>(inject_code.as_bytes());
    })
    .await;

    // Step 2: Build a `{ oauth, auth, validate: true }` arg bag and call
    // start(). Auth comes from disk so the skill still has the full picture
    // if it's already connected via auth/complete; oauth is the freshly
    // submitted credential we want validated.
    let auth_on_disk = match std::fs::read_to_string(data_dir.join("auth_credential.json")) {
        Ok(s) if !s.trim().is_empty() => {
            serde_json::from_str::<serde_json::Value>(&s).unwrap_or(serde_json::Value::Null)
        }
        _ => serde_json::Value::Null,
    };
    let start_args_value = serde_json::json!({
        "oauth": params,
        "auth": auth_on_disk,
        "validate": true,
    });
    let start_args =
        serde_json::to_string(&start_args_value).unwrap_or_else(|_| "null".to_string());
    let result = handle_js_call(rt, ctx, "start", &start_args).await;

    // Evaluate validation result
    let validation_failed = match &result {
        Err(_) => true,
        Ok(val) => val.get("status").and_then(|s| s.as_str()) == Some("error"),
    };

    if validation_failed {
        // Rollback temporary injection — credential never made it to disk so
        // a follow-up restart will see the skill as disconnected again.
        let clear_code = r#"(function() {
            if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {
                globalThis.oauth.__setCredential(null);
            }
            if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {
                globalThis.state.set('__oauth_credential', '');
            }
            globalThis.__oauthClientKey = null;
        })()"#;
        ctx.with(|js_ctx| {
            let _ = js_ctx.eval::<rquickjs::Value, _>(clear_code.as_bytes());
        })
        .await;
        log::info!(
            "[skill:{}] oauth/complete validation failed, credential not persisted",
            skill_id
        );
        return result;
    }

    // Step 3: Validation passed — persist credentials and client key to disk
    // so they survive restarts.
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

    if !client_key.is_empty() {
        let key_path = data_dir.join("client_key.json");
        let key_json = serde_json::json!({ "clientKey": client_key }).to_string();
        if let Err(e) = std::fs::write(&key_path, &key_json) {
            log::error!(
                "[skill:{}] Failed to persist client key share: {e}",
                skill_id
            );
        } else {
            log::info!(
                "[skill:{}] Client key share persisted to {}",
                skill_id,
                key_path.display()
            );
        }
    }

    result
}

/// Handle `oauth/revoked` RPC.
///
/// 1. Clears credentials and client keys from the JS runtime.
/// 2. Deletes credential files from disk.
/// 3. Spawns a background task to clear the skill's memory store.
/// 4. Invokes the `onOAuthRevoked` lifecycle handler in JS.
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
        globalThis.__oauthClientKey = null;
    })()"#;
    ctx.with(|js_ctx| {
        let _ = js_ctx.eval::<rquickjs::Value, _>(clear_code.as_bytes());
    })
    .await;

    let cred_path = data_dir.join("oauth_credential.json");
    let _ = std::fs::remove_file(&cred_path);
    let key_path = data_dir.join("client_key.json");
    let _ = std::fs::remove_file(&key_path);
    log::info!(
        "[skill:{}] OAuth credential and client key cleared from store and disk",
        skill_id
    );

    // Trigger memory cleanup for this skill/integration
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

/// Handle `auth/complete` RPC.
///
/// There is no separate `onAuthComplete` JS hook anymore. The flow is:
///   1. Temporarily inject the new credentials into the JS bridges.
///   2. Call `start({ oauth, auth, validate: true })`. start() owns both
///      validation (when `validate` is set) and activation (cron, etc.) — it
///      returns `{ status: "complete" }` or `{ status: "error", errors: [...] }`.
///   3. If start() returned an error, roll back the temporary injection and
///      surface the result to the caller (no persistence on disk).
///   4. Otherwise persist the credentials to disk so they survive restarts.
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

    // Step 1: Temporary injection so the JS bridges (`auth`, and `oauth` for
    // managed mode) see the new credentials while start() runs validation.
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

    // Step 2: Build a `{ oauth, auth, validate: true }` arg bag and call
    // start(). We pass the freshly-submitted auth params (not what's on disk)
    // so the validation step inside start() inspects the *new* credentials.
    // OAuth comes from disk so the skill still has the full picture.
    let oauth_on_disk = match std::fs::read_to_string(data_dir.join("oauth_credential.json")) {
        Ok(s) if !s.trim().is_empty() => {
            serde_json::from_str::<serde_json::Value>(&s).unwrap_or(serde_json::Value::Null)
        }
        _ => serde_json::Value::Null,
    };
    let start_args_value = serde_json::json!({
        "oauth": oauth_on_disk,
        "auth": params,
        "validate": true,
    });
    let start_args =
        serde_json::to_string(&start_args_value).unwrap_or_else(|_| "null".to_string());
    let result = handle_js_call(rt, ctx, "start", &start_args).await;

    // Evaluate validation result
    let validation_failed = match &result {
        Err(_) => true,
        Ok(val) => val.get("status").and_then(|s| s.as_str()) == Some("error"),
    };

    if validation_failed {
        // Rollback temporary injection — credentials never made it to disk so
        // a follow-up restart will see the skill as disconnected again.
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

    // Step 3: Permanent injection and persistence. start() already activated
    // cron etc. above; this just makes sure the bridges and disk reflect the
    // new credentials so a future restart restores the same state.
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

    // Back-compatibility for managed mode
    if is_managed {
        let oauth_cred_json = serde_json::to_string(
            params
                .get("credentials")
                .unwrap_or(&serde_json::Value::Null),
        )
        .unwrap_or_else(|_| "null".to_string());
        let oauth_path = data_dir.join("oauth_credential.json");
        let _ = std::fs::write(&oauth_path, &oauth_cred_json);
    }

    result
}

/// Handle `auth/revoked` RPC.
///
/// Clears Auth credentials (and managed OAuth credentials if applicable) from
/// the runtime and disk. Also triggers a background memory cleanup.
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
            globalThis.__oauthClientKey = null;
            {managed_clear}
        }})()"#,
        managed_clear = managed_clear
    );
    ctx.with(|js_ctx| {
        let _ = js_ctx.eval::<rquickjs::Value, _>(clear_code.as_bytes());
    })
    .await;

    let cred_path = data_dir.join("auth_credential.json");
    let _ = std::fs::remove_file(&cred_path);
    let key_path = data_dir.join("client_key.json");
    let _ = std::fs::remove_file(&key_path);
    if is_managed {
        let oauth_path = data_dir.join("oauth_credential.json");
        let _ = std::fs::remove_file(&oauth_path);
    }
    log::info!(
        "[skill:{}] Auth credential and client key cleared from store and disk",
        skill_id
    );

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

/// Handle `skill/sync` RPC.
///
/// Fires `onSync` in the JS runtime as a background task and returns immediately.
/// The JS function runs asynchronously via the QuickJS job queue — progress is
/// published by the skill through `state.setPartial()` and can be read via
/// `sync-status` tool or `skills_status` RPC.
///
/// On completion (success or failure) the JS skill updates its own state. A
/// memory snapshot is persisted once the sync finishes via a completion callback
/// injected into the JS promise chain.
pub(crate) async fn handle_sync(
    _rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    ops_state: &Arc<RwLock<qjs_ops::SkillState>>,
    memory_client: &Option<MemoryClientRef>,
    memory_write_tx: &mpsc::Sender<MemoryWriteJob>,
) -> Result<serde_json::Value, String> {
    let skill_id_owned = skill_id.to_string();

    // Clone handles for the completion callback
    let ops_for_cb = ops_state.clone();
    let mem_client_for_cb = memory_client.clone();
    let mem_tx_for_cb = memory_write_tx.clone();

    // Fire onSync without awaiting the promise — it runs in the QuickJS job
    // queue and the event loop drives it on subsequent ticks.
    let start_result = ctx
        .with(|js_ctx| {
            let code = r#"(function() {
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.onSync || globalThis.onSync;
                if (typeof fn !== 'function') {
                    return "no_handler";
                }
                var result = fn.call(skill, {});
                if (result && typeof result.then === 'function') {
                    // Mark sync as in-flight so completion callback can persist memory
                    globalThis.__syncInFlight = true;
                    result.then(
                        function() {
                            globalThis.__syncInFlight = false;
                            console.log('[notion][sync] background sync completed successfully');
                        },
                        function(e) {
                            globalThis.__syncInFlight = false;
                            console.error('[notion][sync] background sync failed: ' + (e && e.message ? e.message : String(e)));
                        }
                    );
                    return "started";
                }
                // Synchronous — already done
                return "done";
            })()"#;

            match js_ctx.eval::<String, _>(code.as_bytes()) {
                Ok(s) => Ok(s),
                Err(e) => {
                    let detail = super::super::js_helpers::format_js_exception(&js_ctx, &e);
                    Err(format!("onSync() failed to start: {detail}"))
                }
            }
        })
        .await;

    match start_result {
        Ok(ref status) if status == "no_handler" => {
            // Skills without an `onSync` handler should treat a sync RPC
            // as a no-op rather than a hard error. Plenty of skills don't
            // need a periodic sync (e.g. `server-ping`, utility skills),
            // and the cron driver fires `skills_sync` against every skill
            // on its schedule — raising here would turn a blanket sweep
            // into a cascade of RPC errors in logs/dashboards.
            log::debug!(
                "[skill:{}] sync no-op: skill does not implement onSync",
                skill_id_owned
            );
            Ok(serde_json::json!({
                "status": "no_handler",
                "skipped": true,
                "reason": "Skill does not implement onSync"
            }))
        }
        Ok(ref status) => {
            log::info!(
                "[skill:{}] sync started in background (status={})",
                skill_id_owned,
                status
            );

            // If synchronous ("done"), persist memory now
            if status == "done" {
                persist_state_to_memory(
                    &skill_id_owned,
                    "periodic sync",
                    &ops_for_cb,
                    &mem_client_for_cb,
                    &mem_tx_for_cb,
                    true,
                );
            }

            Ok(serde_json::json!({
                "ok": true,
                "status": status,
                "message": "Sync started in background. Query sync-status for progress."
            }))
        }
        Err(e) => Err(e),
    }
}
