use std::time::Duration;

use crate::openhuman::skills::types::{ToolContent, ToolResult};

use super::js_helpers::{drive_jobs, format_js_exception};

/// Call a lifecycle function on the skill object.
/// Handles async (Promise-returning) lifecycle methods (init, start, stop).
pub(crate) async fn call_lifecycle(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    name: &str,
) -> Result<(), String> {
    let name = name.to_string();
    let is_promise = ctx
        .with(|js_ctx| {
            let code = format!(
                r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.{name} || globalThis.{name};
                if (typeof fn === 'function') {{
                    var result = fn.call(skill);
                    if (result && typeof result.then === 'function') {{
                        globalThis.__pendingLifecycleDone = false;
                        globalThis.__pendingLifecycleError = undefined;
                        result.then(
                            function() {{ globalThis.__pendingLifecycleDone = true; }},
                            function(e) {{
                                globalThis.__pendingLifecycleError = e && e.message ? e.message : String(e);
                                globalThis.__pendingLifecycleDone = true;
                            }}
                        );
                        return "1";
                    }}
                }}
                return "0";
            }})()"#
            );
            match js_ctx.eval::<String, _>(code.as_bytes()) {
                Ok(s) => Ok(s == "1"),
                Err(e) => {
                    let detail = format_js_exception(&js_ctx, &e);
                    Err(format!("{name}() failed: {detail}"))
                }
            }
        })
        .await?;

    if is_promise {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        loop {
            drive_jobs(rt).await;

            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingLifecycleDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                let error = ctx
                    .with(|js_ctx| {
                        let has_error = js_ctx
                            .eval::<bool, _>(b"globalThis.__pendingLifecycleError !== undefined")
                            .unwrap_or(false);
                        let err = if has_error {
                            Some(
                                js_ctx
                                    .eval::<String, _>(b"String(globalThis.__pendingLifecycleError)")
                                    .unwrap_or_else(|_| "Unknown error".to_string()),
                            )
                        } else {
                            None
                        };
                        let _ = js_ctx.eval::<rquickjs::Value, _>(
                            b"delete globalThis.__pendingLifecycleDone; delete globalThis.__pendingLifecycleError;",
                        );
                        err
                    })
                    .await;

                if let Some(err_msg) = error {
                    return Err(format!("{name}() rejected: {err_msg}"));
                }
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                ctx.with(|js_ctx| {
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingLifecycleDone; delete globalThis.__pendingLifecycleError;",
                    );
                })
                .await;
                return Err(format!("{name}() timed out after 30s"));
            }

            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(())
}

/// Start an async tool call.
///
/// Calls the tool's `execute()` and checks if it returns a Promise.
/// - If sync: returns `Ok(Some(ToolResult))` with the immediate result.
/// - If async (Promise): attaches `.then`/`.catch` handlers that store the
///   resolved value in `globalThis.__pendingTool*` globals, and returns
///   `Ok(None)`. The caller should let the event loop drive the QuickJS
///   runtime and poll `__pendingToolDone` for completion.
pub(crate) async fn start_async_tool_call(
    ctx: &rquickjs::AsyncContext,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<Option<ToolResult>, String> {
    log::info!("[tool_call] start_async_tool_call: tool='{}' args={}", tool_name, arguments);

    let args_str =
        serde_json::to_string(&arguments).map_err(|e| format!("Failed to serialize args: {e}"))?;
    let tool_name = tool_name.to_string();

    let eval_result = ctx
        .with(|js_ctx| {
            let code = format!(
                r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || null);
                var tools = (skill && skill.tools) || globalThis.tools || [];
                for (var i = 0; i < tools.length; i++) {{
                    if (tools[i].name === "{}") {{
                        var args = {};
                        var result = tools[i].execute(args);
                        if (result && typeof result.then === 'function') {{
                            globalThis.__pendingToolResult = undefined;
                            globalThis.__pendingToolError = undefined;
                            globalThis.__pendingToolDone = false;
                            result.then(
                                function(v) {{
                                    globalThis.__pendingToolResult = v;
                                    globalThis.__pendingToolDone = true;
                                }},
                                function(e) {{
                                    globalThis.__pendingToolError = e;
                                    globalThis.__pendingToolDone = true;
                                }}
                            );
                            return "__PROMISE__";
                        }}
                        if (result && typeof result === 'object') {{
                            return JSON.stringify(result);
                        }}
                        return String(result);
                    }}
                }}
                throw new Error("Tool '{}' not found");
            }})()"#,
                tool_name.replace('"', r#"\""#),
                args_str,
                tool_name.replace('"', r#"\""#),
            );

            match js_ctx.eval::<String, _>(code.as_bytes()) {
                Ok(s) => Ok(s),
                Err(e) => {
                    let detail = format_js_exception(&js_ctx, &e);
                    Err(format!("Tool execution failed: {detail}"))
                }
            }
        })
        .await?;

    if eval_result == "__PROMISE__" {
        // Async — caller should poll __pendingToolDone via the event loop
        Ok(None)
    } else {
        // Sync — return immediately
        Ok(Some(ToolResult {
            content: vec![ToolContent::Text { text: eval_result }],
            is_error: false,
        }))
    }
}

/// Read the result of a completed async tool call from JS globals.
/// Call this only after `globalThis.__pendingToolDone === true`.
pub(crate) async fn read_pending_tool_result(
    ctx: &rquickjs::AsyncContext,
) -> Result<ToolResult, String> {
    let result_text = ctx
        .with(|js_ctx| {
            let code = r#"(function() {
                var err = globalThis.__pendingToolError;
                globalThis.__pendingToolError = undefined;
                globalThis.__pendingToolDone = false;
                if (err !== undefined) {
                    var msg = (typeof err === 'object' && err !== null && err.message)
                        ? err.message
                        : String(err);
                    globalThis.__pendingToolResult = undefined;
                    throw new Error(msg);
                }
                var r = globalThis.__pendingToolResult;
                globalThis.__pendingToolResult = undefined;
                if (r === undefined || r === null) return "null";
                if (typeof r === 'object') return JSON.stringify(r);
                return String(r);
            })()"#;

            match js_ctx.eval::<String, _>(code.as_bytes()) {
                Ok(s) => Ok(s),
                Err(e) => {
                    let detail = format_js_exception(&js_ctx, &e);
                    Err(format!("Tool async execution failed: {detail}"))
                }
            }
        })
        .await?;

    Ok(ToolResult {
        content: vec![ToolContent::Text { text: result_text }],
        is_error: false,
    })
}

/// Handle a server event.
/// Handles async (Promise-returning) onServerEvent handlers.
pub(crate) async fn handle_server_event(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    event: &str,
    data: serde_json::Value,
) -> Result<(), String> {
    let data_str = serde_json::to_string(&data).unwrap_or_else(|_| "null".to_string());
    let event = event.to_string();

    let is_promise = ctx
        .with(|js_ctx| {
            let code = format!(
                r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.onServerEvent || globalThis.onServerEvent;
                if (typeof fn === 'function') {{
                    var result = fn.call(skill, "{}", {});
                    if (result && typeof result.then === 'function') {{
                        globalThis.__pendingEventDone = false;
                        globalThis.__pendingEventError = undefined;
                        result.then(
                            function() {{ globalThis.__pendingEventDone = true; }},
                            function(e) {{
                                globalThis.__pendingEventError = e && e.message ? e.message : String(e);
                                globalThis.__pendingEventDone = true;
                            }}
                        );
                        return "1";
                    }}
                }}
                return "0";
            }})()"#,
                event.replace('"', r#"\""#),
                data_str,
            );

            match js_ctx.eval::<String, _>(code.as_bytes()) {
                Ok(s) => Ok(s == "1"),
                Err(e) => Err(format!("Event handler failed: {e}")),
            }
        })
        .await?;

    if is_promise {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        loop {
            drive_jobs(rt).await;

            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingEventDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                let error = ctx
                    .with(|js_ctx| {
                        let has_error = js_ctx
                            .eval::<bool, _>(b"globalThis.__pendingEventError !== undefined")
                            .unwrap_or(false);
                        let err = if has_error {
                            Some(
                                js_ctx
                                    .eval::<String, _>(b"String(globalThis.__pendingEventError)")
                                    .unwrap_or_else(|_| "Unknown error".to_string()),
                            )
                        } else {
                            None
                        };
                        let _ = js_ctx.eval::<rquickjs::Value, _>(
                            b"delete globalThis.__pendingEventDone; delete globalThis.__pendingEventError;",
                        );
                        err
                    })
                    .await;

                if let Some(err_msg) = error {
                    return Err(format!("onServerEvent() rejected: {err_msg}"));
                }
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                ctx.with(|js_ctx| {
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingEventDone; delete globalThis.__pendingEventError;",
                    );
                })
                .await;
                return Err("onServerEvent() timed out after 30s".to_string());
            }

            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(())
}

/// Handle a cron trigger.
/// Handles async (Promise-returning) onCronTrigger handlers.
pub(crate) async fn handle_cron_trigger(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    schedule_id: &str,
) -> Result<(), String> {
    let schedule_id = schedule_id.to_string();

    let is_promise = ctx
        .with(|js_ctx| {
            let code = format!(
                r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.onCronTrigger || globalThis.onCronTrigger;
                if (typeof fn === 'function') {{
                    var result = fn.call(skill, "{}");
                    if (result && typeof result.then === 'function') {{
                        globalThis.__pendingCronDone = false;
                        globalThis.__pendingCronError = undefined;
                        result.then(
                            function() {{ globalThis.__pendingCronDone = true; }},
                            function(e) {{
                                globalThis.__pendingCronError = e && e.message ? e.message : String(e);
                                globalThis.__pendingCronDone = true;
                            }}
                        );
                        return "1";
                    }}
                }}
                return "0";
            }})()"#,
                schedule_id.replace('"', r#"\""#),
            );
            match js_ctx.eval::<String, _>(code.as_bytes()) {
                Ok(s) => Ok(s == "1"),
                Err(e) => Err(format!("Cron trigger failed: {e}")),
            }
        })
        .await?;

    if is_promise {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        loop {
            drive_jobs(rt).await;

            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingCronDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                let error = ctx
                    .with(|js_ctx| {
                        let has_error = js_ctx
                            .eval::<bool, _>(b"globalThis.__pendingCronError !== undefined")
                            .unwrap_or(false);
                        let err = if has_error {
                            Some(
                                js_ctx
                                    .eval::<String, _>(b"String(globalThis.__pendingCronError)")
                                    .unwrap_or_else(|_| "Unknown error".to_string()),
                            )
                        } else {
                            None
                        };
                        let _ = js_ctx.eval::<rquickjs::Value, _>(
                            b"delete globalThis.__pendingCronDone; delete globalThis.__pendingCronError;",
                        );
                        err
                    })
                    .await;

                if let Some(err_msg) = error {
                    return Err(format!("onCronTrigger() rejected: {err_msg}"));
                }
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                ctx.with(|js_ctx| {
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingCronDone; delete globalThis.__pendingCronError;",
                    );
                })
                .await;
                return Err("onCronTrigger() timed out after 30s".to_string());
            }

            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(())
}

/// Call a JS function on the skill object that returns a JSON value.
/// Handles both sync and async (Promise-returning) functions.
pub(crate) async fn handle_js_call(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    fn_name: &str,
    args_json: &str,
) -> Result<serde_json::Value, String> {
    let fn_name = fn_name.to_string();
    let args_json = args_json.to_string();

    let result_text = ctx
        .with(|js_ctx| {
            let code = format!(
                r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.{fn_name} || globalThis.{fn_name};
                if (typeof fn === 'function') {{
                    var result = fn.call(skill, {args_json});
                    if (result && typeof result.then === 'function') {{
                        globalThis.__pendingRpcResult = undefined;
                        globalThis.__pendingRpcError = undefined;
                        globalThis.__pendingRpcDone = false;
                        result.then(
                            function(v) {{
                                globalThis.__pendingRpcResult = v;
                                globalThis.__pendingRpcDone = true;
                            }},
                            function(e) {{
                                globalThis.__pendingRpcError = e && e.message ? e.message : String(e);
                                globalThis.__pendingRpcDone = true;
                            }}
                        );
                        return "__PROMISE__";
                    }}
                    return JSON.stringify(result === undefined ? null : result);
                }}
                return "null";
            }})()"#
            );

            match js_ctx.eval::<String, _>(code.as_bytes()) {
                Ok(s) => Ok(s),
                Err(e) => {
                    let detail = format_js_exception(&js_ctx, &e);
                    Err(format!("{fn_name}() failed: {detail}"))
                }
            }
        })
        .await?;

    if result_text == "__PROMISE__" {
        // Async — drive the QuickJS job queue until the promise resolves
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        loop {
            drive_jobs(rt).await;

            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingRpcDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                let result = ctx
                    .with(|js_ctx| {
                        let has_error = js_ctx
                            .eval::<bool, _>(b"globalThis.__pendingRpcError !== undefined")
                            .unwrap_or(false);

                        let val = if has_error {
                            let err_msg = js_ctx
                                .eval::<String, _>(b"String(globalThis.__pendingRpcError)")
                                .unwrap_or_else(|_| "Unknown error".to_string());
                            Err(format!("{fn_name}() rejected: {err_msg}"))
                        } else {
                            let json_str = js_ctx
                                .eval::<String, _>(
                                    b"JSON.stringify(globalThis.__pendingRpcResult === undefined ? null : globalThis.__pendingRpcResult)"
                                )
                                .unwrap_or_else(|_| "null".to_string());
                            Ok(json_str)
                        };

                        // Clean up globals
                        let _ = js_ctx.eval::<rquickjs::Value, _>(
                            b"delete globalThis.__pendingRpcDone; delete globalThis.__pendingRpcResult; delete globalThis.__pendingRpcError;",
                        );
                        val
                    })
                    .await;

                return match result {
                    Ok(json_str) => serde_json::from_str(&json_str)
                        .map_err(|e| format!("{fn_name}() returned invalid JSON: {e}")),
                    Err(e) => Err(e),
                };
            }

            if tokio::time::Instant::now() >= deadline {
                ctx.with(|js_ctx| {
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingRpcDone; delete globalThis.__pendingRpcResult; delete globalThis.__pendingRpcError;",
                    );
                })
                .await;
                return Err(format!("{fn_name}() timed out after 30s"));
            }

            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    } else {
        serde_json::from_str(&result_text)
            .map_err(|e| format!("{fn_name}() returned invalid JSON: {e}"))
    }
}

/// Call a JS function on the skill object that returns void.
/// Handles both sync and async (Promise-returning) functions.
pub(crate) async fn handle_js_void_call(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    fn_name: &str,
    args_json: &str,
) -> Result<(), String> {
    let fn_name = fn_name.to_string();
    let args_json = args_json.to_string();

    let is_promise = ctx
        .with(|js_ctx| {
            let code = format!(
                r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.{fn_name} || globalThis.{fn_name};
                if (typeof fn === 'function') {{
                    var result = fn.call(skill, {args_json});
                    if (result && typeof result.then === 'function') {{
                        globalThis.__pendingRpcVoidDone = false;
                        globalThis.__pendingRpcVoidError = undefined;
                        result.then(
                            function() {{ globalThis.__pendingRpcVoidDone = true; }},
                            function(e) {{
                                globalThis.__pendingRpcVoidError = e && e.message ? e.message : String(e);
                                globalThis.__pendingRpcVoidDone = true;
                            }}
                        );
                        return "1";
                    }}
                }}
                return "0";
            }})()"#
            );

            match js_ctx.eval::<String, _>(code.as_bytes()) {
                Ok(s) => Ok(s == "1"),
                Err(e) => {
                    let detail = format_js_exception(&js_ctx, &e);
                    Err(format!("{fn_name}() failed: {detail}"))
                }
            }
        })
        .await?;

    if is_promise {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        loop {
            drive_jobs(rt).await;

            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingRpcVoidDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                let error = ctx
                    .with(|js_ctx| {
                        let has_error = js_ctx
                            .eval::<bool, _>(b"globalThis.__pendingRpcVoidError !== undefined")
                            .unwrap_or(false);
                        let err = if has_error {
                            Some(
                                js_ctx
                                    .eval::<String, _>(b"String(globalThis.__pendingRpcVoidError)")
                                    .unwrap_or_else(|_| "Unknown error".to_string()),
                            )
                        } else {
                            None
                        };
                        let _ = js_ctx.eval::<rquickjs::Value, _>(
                            b"delete globalThis.__pendingRpcVoidDone; delete globalThis.__pendingRpcVoidError;",
                        );
                        err
                    })
                    .await;

                if let Some(err_msg) = error {
                    return Err(format!("{fn_name}() rejected: {err_msg}"));
                }
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                ctx.with(|js_ctx| {
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingRpcVoidDone; delete globalThis.__pendingRpcVoidError;",
                    );
                })
                .await;
                return Err(format!("{fn_name}() timed out after 30s"));
            }

            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(())
}
