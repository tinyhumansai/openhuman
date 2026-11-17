//! Picker and decision handling.
use super::*;

pub(super) fn text_row(id: &str, label: &str, detail: &str) -> OverlayRow {
    OverlayRow {
        id: id.into(),
        label: label.into(),
        detail: detail.into(),
        payload: serde_json::Value::Null,
    }
}

pub(super) fn plan_review_overlay(review: &PendingPlanReview) -> Overlay {
    let mut overlay = Overlay::new(OverlayKind::PlanReview, "Plan review");
    overlay.rows = review
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| text_row(&index.to_string(), &format!("{}. {step}", index + 1), ""))
        .collect();
    overlay
        .rows
        .insert(0, text_row("summary", &review.summary, ""));
    overlay.status = "a approve · r reject · e request revision · Esc leaves pending".into();
    overlay
}

pub(super) fn present_pending_plan_review(ui: &mut UiState) {
    if let Some(review) = ui.pending_plan_review.as_ref() {
        ui.overlay = Some(plan_review_overlay(review));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn open_rpc_overlay(
    runtime: &Arc<CoreRuntime>,
    ui: &mut UiState,
    kind: OverlayKind,
    title: &str,
    method: &str,
    params: serde_json::Value,
    array_keys: &[&str],
    id_keys: &[&str],
    label_keys: &[&str],
) {
    ui.overlay_generation = ui.overlay_generation.wrapping_add(1);
    let generation = ui.overlay_generation;
    let thread_id = ui.thread_id.clone();
    let mut overlay = Overlay::new(kind, title);
    overlay.status = "Loading… · Esc closes".into();
    ui.overlay = Some(overlay);
    let runtime = runtime.clone();
    let title = title.to_string();
    let method = method.to_string();
    let array_keys: Vec<String> = array_keys.iter().map(|key| key.to_string()).collect();
    let id_keys: Vec<String> = id_keys.iter().map(|key| key.to_string()).collect();
    let label_keys: Vec<String> = label_keys.iter().map(|key| key.to_string()).collect();
    let future = async move {
        let result =
            match tokio::time::timeout(Duration::from_secs(20), runtime.invoke(&method, params))
                .await
            {
                Ok(result) => result,
                Err(_) => Err("Request timed out".into()),
            };
        let overlay =
            crate::effects::build(kind, &title, result, &array_keys, &id_keys, &label_keys);
        crate::effects::OverlayReply {
            thread_id,
            generation,
            overlay,
        }
    };
    if let Some(tx) = ui.overlay_tx.clone() {
        tokio::spawn(async move {
            let _ = tx.send(future.await);
        });
    } else {
        let reply = future.await;
        crate::effects::apply(reply, ui);
    }
}
pub(super) fn open_history_search(ui: &mut UiState) {
    let mut overlay = Overlay::new(OverlayKind::HistorySearch, "Composer history");
    overlay.rows = ui
        .composer
        .history_search("")
        .into_iter()
        .enumerate()
        .map(|(index, text)| text_row(&index.to_string(), &text, "Enter restores this prompt"))
        .collect();
    overlay.status = "Type to filter · Enter restores · Esc closes".into();
    ui.overlay = Some(overlay);
}

pub(super) async fn handle_overlay_key(
    key: KeyEvent,
    runtime: &Arc<CoreRuntime>,
    _client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) -> bool {
    let kind = ui.overlay.as_ref().map(|overlay| overlay.kind).unwrap();
    if key.code == KeyCode::Esc {
        ui.overlay = None;
        return false;
    }
    if kind == OverlayKind::ConfirmDelete && matches!(key.code, KeyCode::Char('y' | 'Y')) {
        let result = runtime
            .invoke(
                "openhuman.threads_delete",
                json!({
                    "thread_id": ui.thread_id,
                    "deleted_at": chrono::Utc::now().to_rfc3339(),
                }),
            )
            .await;
        ui.overlay = None;
        match result {
            Ok(_) => new_thread(runtime, state, ui).await,
            Err(error) => state.push_system(format!("Could not delete thread: {error}")),
        }
        return false;
    }
    let typing = ui
        .overlay
        .as_ref()
        .is_some_and(|overlay| overlay.input.is_some());
    if kind == OverlayKind::Approvals {
        let decision = decision_shortcut(kind, typing, key.code);
        if let Some(decision) = decision {
            let request_id = selected_overlay_row(ui)
                .map(|row| row.id)
                .unwrap_or_default();
            decide_approval(runtime, &request_id, decision, state, ui).await;
            return false;
        }
    }
    if kind == OverlayKind::PlanReview && !typing {
        if matches!(key.code, KeyCode::Char('e' | 'E')) {
            if let Some(overlay) = &mut ui.overlay {
                overlay.input = Some(String::new());
                overlay.status = "Describe the needed revision, then press Enter".into();
            }
            return false;
        }
        let decision = decision_shortcut(kind, typing, key.code);
        if let Some(decision) = decision {
            if let Some(review) = ui.pending_plan_review.take() {
                match runtime
                    .invoke(
                        "openhuman.plan_review_decide",
                        json!({"request_id": review.request_id, "decision": decision}),
                    )
                    .await
                {
                    Ok(_) => state.push_system(format!("Plan {decision}d.")),
                    Err(error) => state.push_system(format!("Could not decide plan: {error}")),
                }
            }
            ui.overlay = None;
            return false;
        }
    }

    let Some(overlay) = &mut ui.overlay else {
        return false;
    };
    match key.code {
        KeyCode::Up => overlay.selected = overlay.selected.saturating_sub(1),
        KeyCode::Down => {
            let max = overlay.visible_rows().len().saturating_sub(1);
            overlay.selected = (overlay.selected + 1).min(max);
        }
        KeyCode::Backspace => {
            if let Some(input) = &mut overlay.input {
                input.pop();
            } else {
                overlay.filter.pop();
                overlay.clamp_selection();
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(input) = &mut overlay.input {
                input.push(c);
            } else {
                overlay.filter.push(c);
                overlay.clamp_selection();
            }
        }
        KeyCode::Enter => {
            let input = overlay.input.clone().unwrap_or_default();
            let selected = overlay
                .visible_rows()
                .get(overlay.selected)
                .cloned()
                .cloned();
            match kind {
                OverlayKind::Login => {
                    if let Some(row) = selected {
                        if let Some(provider) = crate::account::provider(&row.id) {
                            crate::account::browser(runtime, ui, provider);
                        }
                    }
                }
                OverlayKind::Threads => {
                    if let Some(row) = selected {
                        switch_thread(runtime, state, ui, row.id).await;
                    }
                }
                OverlayKind::HistorySearch => {
                    if let Some(row) = selected {
                        ui.composer.set_text(row.label);
                    }
                    ui.overlay = None;
                }
                OverlayKind::Rename => {
                    if !input.trim().is_empty() {
                        match runtime
                            .invoke(
                                "openhuman.threads_update_title",
                                json!({"thread_id": ui.thread_id, "title": input.trim()}),
                            )
                            .await
                        {
                            Ok(_) => {
                                state.push_system(format!("Thread renamed to {}.", input.trim()))
                            }
                            Err(error) => {
                                state.push_system(format!("Could not rename thread: {error}"))
                            }
                        }
                    }
                    ui.overlay = None;
                }
                OverlayKind::Model => {
                    let model = if overlay.input.is_some() {
                        input.trim().to_string()
                    } else {
                        selected.map(|row| row.id).unwrap_or_default()
                    };
                    ui.model_override = (!model.is_empty()).then_some(model);
                    state.push_system(match &ui.model_override {
                        Some(model) => format!("Using model override {model}."),
                        None => "Using the configured default model.".into(),
                    });
                    ui.overlay = None;
                }
                OverlayKind::Permissions => {
                    if let Some(row) = selected {
                        match runtime
                            .invoke(
                                "openhuman.config_update_autonomy_settings",
                                json!({"level": row.id}),
                            )
                            .await
                        {
                            Ok(_) => {
                                state.push_system(format!("Agent access set to {}.", row.label))
                            }
                            Err(error) => {
                                state.push_system(format!("Could not update access: {error}"))
                            }
                        }
                    }
                    ui.overlay = None;
                }
                OverlayKind::Agents => {
                    if let Some(row) = selected {
                        if row.id != "orchestrator"
                            && row
                                .payload
                                .get("can_run_as_user_facing_worker")
                                .and_then(serde_json::Value::as_bool)
                                != Some(true)
                        {
                            return false;
                        }
                        match runtime
                            .invoke(
                                "openhuman.config_update_agent_settings",
                                json!({"chat_agent_id":row.id}),
                            )
                            .await
                        {
                            Ok(_) => {
                                ui.agent_name = row.id;
                                state.push_system(format!(
                                    "Chat agent: {}. Configured for subsequent turns.",
                                    ui.agent_name
                                ));
                                ui.overlay = None;
                            }
                            Err(error) => {
                                if let Some(overlay) = &mut ui.overlay {
                                    overlay.status = format!("Could not select agent: {error}");
                                }
                            }
                        }
                    }
                }
                OverlayKind::Help => {
                    if let Some(row) = selected {
                        ui.overlay = None;
                        return Box::pin(execute_command(&row.id, runtime, _client_id, state, ui))
                            .await;
                    }
                }
                OverlayKind::Themes => {
                    if let Some(row) = selected {
                        crate::presentation::select_theme(&row.id, ui);
                    }
                }
                OverlayKind::Tools | OverlayKind::Subagents => {
                    if let Some(row) = selected {
                        crate::presentation::inspect(&row.id, state, ui);
                    }
                }
                OverlayKind::Files => {
                    if let Some(row) = selected {
                        ui.composer.replace_current_token(&format!("@{}", row.id));
                    }
                    ui.overlay = None;
                }
                OverlayKind::PlanReview if !input.trim().is_empty() => {
                    if let Some(review) = ui.pending_plan_review.take() {
                        match runtime
                            .invoke(
                                "openhuman.plan_review_decide",
                                json!({
                                    "request_id": review.request_id,
                                    "decision": "revise",
                                    "feedback": input.trim(),
                                }),
                            )
                            .await
                        {
                            Ok(_) => state.push_system("Plan sent back for revision."),
                            Err(error) => {
                                state.push_system(format!("Could not revise plan: {error}"))
                            }
                        }
                    }
                    ui.overlay = None;
                }
                _ => {}
            }
        }
        _ => {}
    }
    false
}

pub(super) fn decision_shortcut(
    kind: OverlayKind,
    typing: bool,
    code: KeyCode,
) -> Option<&'static str> {
    if typing {
        return None;
    }
    match (kind, code) {
        (OverlayKind::Approvals, KeyCode::Char('1')) => Some("approve_once"),
        (OverlayKind::Approvals, KeyCode::Char('2')) => Some("approve_always_for_tool"),
        (OverlayKind::Approvals, KeyCode::Char('3')) => Some("approve_always_for_flow"),
        (OverlayKind::Approvals, KeyCode::Delete) => Some("deny"),
        (OverlayKind::PlanReview, KeyCode::Char('a' | 'A')) => Some("approve"),
        (OverlayKind::PlanReview, KeyCode::Char('r' | 'R')) => Some("reject"),
        _ => None,
    }
}

pub(super) fn selected_overlay_row(ui: &UiState) -> Option<OverlayRow> {
    let overlay = ui.overlay.as_ref()?;
    overlay
        .visible_rows()
        .get(overlay.selected)
        .cloned()
        .cloned()
}

pub(super) async fn decide_approval(
    runtime: &Arc<CoreRuntime>,
    request_id: &str,
    decision: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) {
    if request_id.is_empty() {
        return;
    }
    match runtime
        .invoke(
            "openhuman.approval_decide",
            json!({"request_id": request_id, "decision": decision}),
        )
        .await
    {
        Ok(_) => {
            ui.pending_approvals
                .retain(|approval| approval.request_id != request_id);
            state.push_system(format!("Approval decision: {decision}."));
            ui.overlay = None;
        }
        Err(error) => state.push_system(format!("Could not decide approval: {error}")),
    }
}
