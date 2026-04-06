//! RPC controller surface for the `text_input` domain.
//!
//! Thin orchestration layer — all platform work delegates to `accessibility::*`.

use crate::openhuman::accessibility;
use crate::rpc::RpcOutcome;

use super::types::*;

/// Read the currently focused text input field.
pub async fn read_field(params: ReadFieldParams) -> Result<RpcOutcome<ReadFieldResult>, String> {
    let ctx = accessibility::focused_text_context_verbose()?;
    let is_terminal = accessibility::is_terminal_app(ctx.app_name.as_deref());

    let bounds = if params.include_bounds.unwrap_or(false) {
        ctx.bounds.as_ref().map(FieldBounds::from_element)
    } else {
        None
    };

    log::debug!(
        "[text_input] read_field app={:?} role={:?} len={} terminal={}",
        ctx.app_name,
        ctx.role,
        ctx.text.len(),
        is_terminal,
    );

    Ok(RpcOutcome::single_log(
        ReadFieldResult {
            app_name: ctx.app_name,
            role: ctx.role,
            text: ctx.text,
            selected_text: ctx.selected_text,
            bounds,
            is_terminal,
        },
        "read_field: ok",
    ))
}

/// Insert text into the currently focused input field.
pub async fn insert_text(params: InsertTextParams) -> Result<RpcOutcome<InsertTextResult>, String> {
    if params.text.is_empty() {
        return Err("text must not be empty".into());
    }

    // Optionally validate that focus hasn't shifted.
    if params.validate_focus.unwrap_or(false)
        || params.expected_app.is_some()
        || params.expected_role.is_some()
    {
        accessibility::validate_focused_target(
            params.expected_app.as_deref(),
            params.expected_role.as_deref(),
        )?;
    }

    log::debug!(
        "[text_input] insert_text len={} validate={}",
        params.text.len(),
        params.validate_focus.unwrap_or(false),
    );

    match accessibility::apply_text_to_focused_field(&params.text) {
        Ok(()) => Ok(RpcOutcome::single_log(
            InsertTextResult {
                inserted: true,
                error: None,
            },
            "insert_text: ok",
        )),
        Err(e) => Ok(RpcOutcome::single_log(
            InsertTextResult {
                inserted: false,
                error: Some(e.clone()),
            },
            format!("insert_text: failed — {e}"),
        )),
    }
}

/// Show ghost text overlay near the focused input field.
pub async fn show_ghost(
    params: ShowGhostTextParams,
) -> Result<RpcOutcome<ShowGhostTextResult>, String> {
    if params.text.is_empty() {
        return Err("ghost text must not be empty".into());
    }

    let ttl_ms = params.ttl_ms.unwrap_or(3000);

    // Resolve bounds: use provided bounds, or read from focused field.
    let element_bounds = match params.bounds {
        Some(b) => b.to_element(),
        None => {
            let ctx = accessibility::focused_text_context_verbose()?;
            ctx.bounds.unwrap_or(accessibility::ElementBounds {
                x: 0,
                y: 0,
                width: 200,
                height: 24,
            })
        }
    };

    log::debug!(
        "[text_input] show_ghost len={} ttl={}ms bounds=({},{},{},{})",
        params.text.len(),
        ttl_ms,
        element_bounds.x,
        element_bounds.y,
        element_bounds.width,
        element_bounds.height,
    );

    match accessibility::show_overlay(&element_bounds, &params.text, ttl_ms) {
        Ok(()) => Ok(RpcOutcome::single_log(
            ShowGhostTextResult {
                shown: true,
                error: None,
            },
            "show_ghost: ok",
        )),
        Err(e) => Ok(RpcOutcome::single_log(
            ShowGhostTextResult {
                shown: false,
                error: Some(e.clone()),
            },
            format!("show_ghost: failed — {e}"),
        )),
    }
}

/// Dismiss the ghost text overlay.
pub async fn dismiss_ghost() -> Result<RpcOutcome<DismissGhostTextResult>, String> {
    log::debug!("[text_input] dismiss_ghost");
    let _ = accessibility::hide_overlay();
    Ok(RpcOutcome::single_log(
        DismissGhostTextResult { dismissed: true },
        "dismiss_ghost: ok",
    ))
}

/// Dismiss ghost text and insert the accepted text in one atomic call.
pub async fn accept_ghost(
    params: AcceptGhostTextParams,
) -> Result<RpcOutcome<AcceptGhostTextResult>, String> {
    if params.text.is_empty() {
        return Err("text must not be empty".into());
    }

    // 1. Dismiss overlay first.
    let _ = accessibility::hide_overlay();

    // 2. Optionally validate focus.
    if params.validate_focus.unwrap_or(false)
        || params.expected_app.is_some()
        || params.expected_role.is_some()
    {
        accessibility::validate_focused_target(
            params.expected_app.as_deref(),
            params.expected_role.as_deref(),
        )?;
    }

    log::debug!(
        "[text_input] accept_ghost len={} validate={}",
        params.text.len(),
        params.validate_focus.unwrap_or(false),
    );

    // 3. Insert text.
    match accessibility::apply_text_to_focused_field(&params.text) {
        Ok(()) => Ok(RpcOutcome::single_log(
            AcceptGhostTextResult {
                inserted: true,
                error: None,
            },
            "accept_ghost: ok",
        )),
        Err(e) => Ok(RpcOutcome::single_log(
            AcceptGhostTextResult {
                inserted: false,
                error: Some(e.clone()),
            },
            format!("accept_ghost: failed — {e}"),
        )),
    }
}
