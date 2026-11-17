//! Runtime event projection and pending decisions.
use super::*;
use crate::event_view::WebEventView;
use openhuman_rpc::embed::chat_surface::WebChannelEvent;

pub(super) fn handle_web_event(
    event: &WebChannelEvent,
    state: &mut TranscriptState,
    ui: &mut UiState,
) {
    let ev = WebEventView::from_event(event);
    if ui.thread_id.is_empty() || ev.client_id != state.client_id() || ev.thread_id != ui.thread_id
    {
        return;
    }
    if matches!(
        ev.event.as_str(),
        "chat_done" | "channel_outbound" | "chat_error"
    ) {
        ui.stopping = false;
    }
    match ev.event.as_str() {
        "approval_request" => {
            let approval = PendingApproval {
                request_id: ev.request_id.clone(),
                tool_name: ev.tool_name.clone().unwrap_or_else(|| "tool".into()),
                summary: ev
                    .message
                    .clone()
                    .unwrap_or_else(|| "Approval required".into()),
                args: ev.args.clone().unwrap_or(serde_json::Value::Null),
            };
            ui.pending_approvals
                .retain(|item| item.request_id != approval.request_id);
            ui.pending_approvals.push(approval.clone());
            let preserves_typed_input = ui
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.input.is_some());
            if !preserves_typed_input {
                let mut overlay = Overlay::new(OverlayKind::Approvals, "Approval required");
                overlay.rows.push(OverlayRow {
                    id: approval.request_id,
                    label: approval.tool_name,
                    detail: format!("{}\n{}", approval.summary, approval.args),
                    payload: approval.args,
                });
                overlay.status =
                    "1 approve once · 2 always for tool · 3 always for flow · Delete deny".into();
                ui.overlay = Some(overlay);
            }
        }
        "plan_review_request" => {
            let steps = ev
                .args
                .as_ref()
                .and_then(|args| args.get("steps"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            item.as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| item.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let review = PendingPlanReview {
                request_id: ev.request_id.clone(),
                summary: ev
                    .message
                    .clone()
                    .unwrap_or_else(|| "Review the proposed plan".into()),
                steps,
            };
            let preserves_typed_input = ui
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.input.is_some());
            ui.pending_plan_review = Some(review);
            if !preserves_typed_input {
                present_pending_plan_review(ui);
            }
        }
        _ => state.apply_event_view(&ev),
    }
}
