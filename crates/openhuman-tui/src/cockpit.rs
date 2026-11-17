//! OpenHuman-native overlays and structured control-plane state.

use serde_json::Value;

pub use openhuman_rpc::unwrap_rpc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Help,
    Threads,
    Rename,
    ConfirmDelete,
    Model,
    Permissions,
    Status,
    Usage,
    Agents,
    Skills,
    Mcp,
    Artifacts,
    Approvals,
    PlanReview,
    Diff,
    HistorySearch,
    Files,
    Themes,
    Login,
    Tools,
    Subagents,
    ToolDetail,
}

#[derive(Debug, Clone)]
pub struct Overlay {
    pub kind: OverlayKind,
    pub title: String,
    pub rows: Vec<OverlayRow>,
    pub selected: usize,
    pub filter: String,
    pub status: String,
    pub input: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OverlayRow {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub payload: Value,
}

impl Overlay {
    pub fn new(kind: OverlayKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            rows: Vec::new(),
            selected: 0,
            filter: String::new(),
            status: String::new(),
            input: None,
        }
    }

    pub fn visible_rows(&self) -> Vec<&OverlayRow> {
        let filter = self.filter.to_ascii_lowercase();
        self.rows
            .iter()
            .filter(|row| {
                filter.is_empty()
                    || row.label.to_ascii_lowercase().contains(&filter)
                    || row.detail.to_ascii_lowercase().contains(&filter)
            })
            .collect()
    }

    pub fn clamp_selection(&mut self) {
        let len = self.visible_rows().len();
        self.selected = self.selected.min(len.saturating_sub(1));
    }
}

#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub request_id: String,
    pub tool_name: String,
    pub summary: String,
    pub args: Value,
}

#[derive(Debug, Clone)]
pub struct PendingPlanReview {
    pub request_id: String,
    pub summary: String,
    pub steps: Vec<String>,
}

pub fn array_at<'a>(value: &'a Value, keys: &[&str]) -> &'a [Value] {
    let value = unwrap_rpc(value);
    for key in keys {
        if let Some(items) = value.get(*key).and_then(Value::as_array) {
            return items;
        }
    }
    &[]
}

pub fn row_from_value(value: &Value, id_keys: &[&str], label_keys: &[&str]) -> OverlayRow {
    let string = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| value.get(*key).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string()
    };
    let id = string(id_keys);
    let label = string(label_keys);
    OverlayRow {
        id: id.clone(),
        label: if label.is_empty() { id } else { label },
        detail: value
            .get("description")
            .or_else(|| value.get("when_to_use"))
            .or_else(|| value.get("status"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        payload: value.clone(),
    }
}

#[cfg(test)]
#[path = "cockpit_tests.rs"]
mod tests;
