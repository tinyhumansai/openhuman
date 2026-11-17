//! Generation-tagged background reads; stale responses never replace a new view.
use super::cockpit::{array_at, row_from_value, Overlay, OverlayKind};
use super::ui_state::UiState;
use serde_json::Value;
pub struct OverlayReply {
    pub thread_id: String,
    pub generation: u64,
    pub overlay: Overlay,
}

pub fn build(
    kind: OverlayKind,
    title: &str,
    result: Result<Value, String>,
    array_keys: &[String],
    id_keys: &[String],
    label_keys: &[String],
) -> Overlay {
    let mut overlay = Overlay::new(kind, title);
    match result {
        Ok(value) => {
            let keys: Vec<_> = array_keys.iter().map(String::as_str).collect();
            let ids: Vec<_> = id_keys.iter().map(String::as_str).collect();
            let labels: Vec<_> = label_keys.iter().map(String::as_str).collect();
            let items = array_at(&value, &keys);
            overlay.rows = items
                .iter()
                .filter(|item| {
                    kind != OverlayKind::Agents
                        || item.get("id").and_then(Value::as_str) == Some("orchestrator")
                        || item
                            .get("can_run_as_user_facing_worker")
                            .and_then(Value::as_bool)
                            == Some(true)
                })
                .map(|item| row_from_value(item, &ids, &labels))
                .collect();
            if array_keys.is_empty() {
                let detail = serde_json::to_string_pretty(super::cockpit::unwrap_rpc(&value))
                    .unwrap_or_default();
                overlay.rows = detail
                    .lines()
                    .enumerate()
                    .map(|(index, line)| super::presentation::row(index.to_string(), line, ""))
                    .collect();
            }
            overlay.status = format!(
                "{} item(s) · type to filter · Enter selects · Esc closes",
                overlay.rows.len()
            );
        }
        Err(error) => {
            overlay.status = format!("Could not load: {error}. Reopen this view to retry.")
        }
    }
    if kind == OverlayKind::Model {
        overlay.rows.insert(
            0,
            super::presentation::row(
                "",
                "Configured default",
                "Use the provider's configured model",
            ),
        );
    }
    overlay
}
pub fn apply(reply: OverlayReply, ui: &mut UiState) {
    if reply.thread_id != ui.thread_id || reply.generation != ui.overlay_generation {
        return;
    }
    if let Some(current) = &ui.overlay {
        if current.kind == reply.overlay.kind {
            let mut next = reply.overlay;
            next.filter = current.filter.clone();
            next.selected = current.selected;
            next.clamp_selection();
            ui.overlay = Some(next);
        }
    }
}
#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
