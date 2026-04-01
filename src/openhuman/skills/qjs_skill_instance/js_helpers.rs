use std::sync::Arc;

use parking_lot::RwLock;

use crate::openhuman::skills::types::ToolDefinition;

use super::types::SkillState;

/// Extract a human-readable error message from a QuickJS exception.
/// When rquickjs returns `Error::Exception`, the actual JS error value
/// is stored in the context and retrieved with `Ctx::catch()`.
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

    // Fallback: try to stringify the exception value
    if let Ok(s) = exception.get::<String>() {
        return s;
    }

    format!("{err}")
}

/// Drive the QuickJS job queue until no more jobs are pending.
pub(crate) async fn drive_jobs(rt: &rquickjs::AsyncRuntime) {
    // idle() runs all pending futures and jobs
    rt.idle().await;
}

/// Extract tool definitions from the skill.
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

/// Load a persisted OAuth credential from the skill's data directory and inject
/// it into the JS context so tools have access to the credential.
///
/// Reads `{data_dir}/oauth_credential.json` which is written by the
/// `oauth/complete` handler and deleted by `oauth/revoked`.
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
