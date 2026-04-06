//! Helper functions for the QuickJS skill runtime.
//!
//! This module provides utility functions for error formatting, job queue
//! management, tool discovery, and state restoration (e.g., credentials).

use std::sync::Arc;

use parking_lot::RwLock;

use crate::openhuman::skills::types::ToolDefinition;

use super::types::SkillState;

/// Extract a human-readable error message from a QuickJS exception.
///
/// When `rquickjs` returns `Error::Exception`, the actual JS error object
/// is stored in the context and must be retrieved using `Ctx::catch()`.
/// This function attempts to extract the `.message` and `.stack` properties
/// from the exception object, falling back to a string representation if they are missing.
pub(crate) fn format_js_exception(js_ctx: &rquickjs::Ctx<'_>, err: &rquickjs::Error) -> String {
    if !err.is_exception() {
        return format!("{err}");
    }

    let exception = js_ctx.catch();

    // Try to get .message and .stack from the exception object
    if let Some(obj) = exception.as_object() {
        let message: String = obj.get::<_, String>("message").unwrap_or_default();
        let stack: String = obj.get::<_, String>("stack").unwrap_or_default();

        if !message.is_empty() {
            if !stack.is_empty() {
                return format!("{message}\n{stack}");
            }
            return message;
        }
    }

    // Fallback: try to stringify the exception value directly
    if let Ok(s) = exception.get::<String>() {
        return s;
    }

    format!("{err}")
}

/// Drive the QuickJS job queue until no more jobs are pending.
///
/// This is essential for progressing asynchronous operations like Promises
/// within the JavaScript environment. It calls `rt.idle()` which blocks
/// the current task until all pending JS jobs have been processed.
pub(crate) async fn drive_jobs(rt: &rquickjs::AsyncRuntime) {
    // idle() runs all pending futures and jobs
    rt.idle().await;
}

/// Extract tool definitions from the loaded skill.
///
/// This function inspects the `globalThis.__skill` or `globalThis.tools`
/// to find exposed tool definitions (name, description, schema) and updates
/// the instance's shared `SkillState`.
pub(crate) fn extract_tools(js_ctx: &rquickjs::Ctx<'_>, state: &Arc<RwLock<SkillState>>) {
    let code = r#"
        (function() {
            var skill = globalThis.__skill && globalThis.__skill.default
                ? globalThis.__skill.default
                : (globalThis.__skill || null);
            var tools = (skill && skill.tools) || globalThis.tools || [];
            return JSON.stringify(tools.map(function(t) {
                return {
                    name: t.name || "",
                    description: t.description || "",
                    inputSchema: t.inputSchema || t.input_schema || {}
                };
            }));
        })()
    "#;

    // eval with String type hint tells rquickjs to convert the result to a Rust String
    match js_ctx.eval::<String, _>(code) {
        Ok(json_str) => match serde_json::from_str::<Vec<ToolDefinition>>(&json_str) {
            Ok(tools) => {
                state.write().tools = tools;
            }
            Err(e) => {
                log::warn!("[tools] Failed to parse tools JSON: {e}");
            }
        },
        Err(e) => {
            log::warn!("[tools] Failed to extract tools: {e}");
        }
    }
}

/// Load a persisted OAuth credential from the skill's data directory and inject it.
///
/// Reads `{data_dir}/oauth_credential.json` and injects it into both the
/// `oauth` bridge and the in-memory state so that tools have immediate access.
pub(crate) async fn restore_oauth_credential(
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    data_dir: &std::path::Path,
) {
    let cred_path = data_dir.join("oauth_credential.json");
    let cred_json = match std::fs::read_to_string(&cred_path) {
        Ok(s) if !s.is_empty() => s,
        _ => return,
    };

    // Inject credential into both oauth bridge and in-memory state
    let code = format!(
        r#"(function() {{
            var cred = {cred};
            if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {{
                globalThis.oauth.__setCredential(cred);
            }}
            if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {{
                globalThis.state.set('__oauth_credential', cred);
            }}
            return true;
        }})()"#,
        cred = cred_json
    );

    let restored = ctx
        .with(|js_ctx| js_ctx.eval::<bool, _>(code.as_bytes()).unwrap_or(false))
        .await;

    if restored {
        log::info!(
            "[skill:{}] Restored OAuth credential from {}",
            skill_id,
            cred_path.display()
        );
    }
}

/// Load a persisted client key share and inject it into the JS context.
///
/// This key is required for encrypted proxy requests via the `oauth.fetch()` API.
pub(crate) async fn restore_client_key(
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    data_dir: &std::path::Path,
) {
    let key_path = data_dir.join("client_key.json");
    let key_json = match std::fs::read_to_string(&key_path) {
        Ok(s) if !s.is_empty() => s,
        _ => return,
    };

    // Parse the stored JSON to extract the base64 key string
    let key_str = match serde_json::from_str::<serde_json::Value>(&key_json) {
        Ok(v) => v
            .get("clientKey")
            .and_then(|k| k.as_str())
            .unwrap_or("")
            .to_string(),
        Err(_) => return,
    };
    if key_str.is_empty() {
        return;
    }

    let code = format!(
        r#"(function() {{
            globalThis.__oauthClientKey = "{key}";
            return true;
        }})()"#,
        key = key_str
    );

    let restored = ctx
        .with(|js_ctx| js_ctx.eval::<bool, _>(code.as_bytes()).unwrap_or(false))
        .await;

    if restored {
        log::info!(
            "[skill:{}] Restored client key share from {}",
            skill_id,
            key_path.display()
        );
    }
}

/// Load a persisted auth credential and inject it into the JS context.
///
/// Handles both standard auth and "managed" mode, where it also bridges to
/// the OAuth credential system for backward compatibility.
pub(crate) async fn restore_auth_credential(
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    data_dir: &std::path::Path,
) {
    let cred_path = data_dir.join("auth_credential.json");
    let cred_json = match std::fs::read_to_string(&cred_path) {
        Ok(s) if !s.is_empty() => s,
        _ => return,
    };

    // Inject credential into auth bridge and in-memory state.
    // For managed mode, also bridge to the oauth credential system.
    let code = format!(
        r#"(function() {{
            var cred = {cred};
            if (typeof globalThis.auth !== 'undefined' && globalThis.auth.__setCredential) {{
                globalThis.auth.__setCredential(cred);
            }}
            if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {{
                globalThis.state.set('__auth_credential', cred);
            }}
            // For managed mode, bridge to oauth system for backward compat.
            // For non-managed modes, clear any stale oauth credentials.
            if (cred && cred.mode === 'managed' && cred.credentials) {{
                if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {{
                    globalThis.oauth.__setCredential(cred.credentials);
                }}
                if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {{
                    globalThis.state.set('__oauth_credential', cred.credentials);
                }}
            }} else {{
                if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {{
                    globalThis.oauth.__setCredential(null);
                }}
                if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {{
                    globalThis.state.set('__oauth_credential', null);
                }}
            }}
            return true;
        }})()"#,
        cred = cred_json
    );

    let restored = ctx
        .with(|js_ctx| js_ctx.eval::<bool, _>(code.as_bytes()).unwrap_or(false))
        .await;

    if restored {
        log::info!(
            "[skill:{}] Restored auth credential from {}",
            skill_id,
            cred_path.display()
        );
    }
}
