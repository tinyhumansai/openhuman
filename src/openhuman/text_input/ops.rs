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

    match accessibility::show_overlay(&element_bounds, &params.text, ttl_ms, "") {
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Guard-clause branches ────────────────────────────────────
    //
    // The post-guard paths below these entry-points call into
    // `accessibility::*`, which requires a focused text field on a
    // live OS display — not reproducible in a headless unit-test
    // environment. These tests pin the pure validation logic that
    // every RPC call must hit before any platform work runs.

    #[tokio::test]
    async fn insert_text_rejects_empty_text() {
        let err = insert_text(InsertTextParams {
            text: String::new(),
            validate_focus: None,
            expected_app: None,
            expected_role: None,
        })
        .await
        .unwrap_err();
        assert!(
            err.contains("text must not be empty"),
            "expected empty-text error, got: {err}"
        );
    }

    #[tokio::test]
    async fn show_ghost_rejects_empty_text() {
        let err = show_ghost(ShowGhostTextParams {
            text: String::new(),
            ttl_ms: None,
            bounds: None,
        })
        .await
        .unwrap_err();
        assert!(
            err.contains("ghost text must not be empty"),
            "expected empty-ghost error, got: {err}"
        );
    }

    #[tokio::test]
    async fn accept_ghost_rejects_empty_text() {
        let err = accept_ghost(AcceptGhostTextParams {
            text: String::new(),
            validate_focus: None,
            expected_app: None,
            expected_role: None,
        })
        .await
        .unwrap_err();
        assert!(
            err.contains("text must not be empty"),
            "expected empty-text error, got: {err}"
        );
    }

    // ── dismiss_ghost always succeeds ────────────────────────────

    #[tokio::test]
    async fn dismiss_ghost_always_reports_success_even_without_overlay() {
        // The implementation discards any hide_overlay() error, so
        // every call must yield `dismissed: true` — callers rely on
        // this idempotent contract.
        let out = dismiss_ghost().await.unwrap();
        assert!(out.value.dismissed);
        assert!(out.logs.iter().any(|l| l.contains("dismiss_ghost: ok")));
    }

    // ── Post-guard paths surface accessibility errors ───────────
    //
    // Without a focused text field, `accessibility::*` returns an
    // Err which the RPC wrappers convert into an `InsertTextResult
    // { inserted: false, error: Some(..) }` (for insert/accept) or
    // bubble up as Err for `read_field` / `show_ghost` (when reading
    // bounds fails). We assert only that these paths do not panic
    // and return a deterministic shape — the specific error string
    // depends on the host OS.

    #[tokio::test]
    async fn insert_text_surfaces_accessibility_failure_as_inserted_false() {
        // A non-empty payload bypasses the guard and reaches the
        // `accessibility::apply_text_to_focused_field` call. The contract
        // of `insert_text` is: any platform failure is wrapped in
        // `InsertTextResult { inserted: false, error: Some(..) }` and
        // returned as `Ok` — never propagated as `Err` — so the JSON-RPC
        // caller always gets a structured result. We pin that contract.
        //
        // On a host with a focused text field `inserted` can legitimately
        // be `true`; in a headless CI runner it will be `false`. Either
        // way, `inserted` and `error` must be mutually exclusive.
        let r = insert_text(InsertTextParams {
            text: "hello".into(),
            // Keep validation flags off so the test only exercises the
            // `apply_text_to_focused_field` path; turning them on would
            // route through `validate_focused_target` first which has its
            // own OS-specific behaviour.
            validate_focus: None,
            expected_app: None,
            expected_role: None,
        })
        .await
        .expect("insert_text must wrap platform failures as Ok(inserted=false)");

        if r.value.inserted {
            assert!(
                r.value.error.is_none(),
                "inserted=true must not carry an error: {:?}",
                r.value.error
            );
            assert!(r.logs.iter().any(|l| l.contains("insert_text: ok")));
        } else {
            let err = r
                .value
                .error
                .as_deref()
                .expect("inserted=false must carry an error message");
            assert!(!err.is_empty(), "error message must be non-empty");
            assert!(r.logs.iter().any(|l| l.contains("insert_text: failed")));
        }
    }
}
