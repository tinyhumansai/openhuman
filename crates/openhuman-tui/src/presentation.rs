//! Local commands and read-only inspection shared with preview mode.
use super::cockpit::{Overlay, OverlayKind, OverlayRow};
use super::state::TranscriptState;
use super::theme::Theme;
use super::ui_state::{AppTab, UiState};
pub fn row(
    id: impl Into<String>,
    label: impl Into<String>,
    detail: impl Into<String>,
) -> OverlayRow {
    OverlayRow {
        id: id.into(),
        label: label.into(),
        detail: detail.into(),
        payload: serde_json::Value::Null,
    }
}
pub fn open(command: &str, state: &TranscriptState, ui: &mut UiState) -> bool {
    match command {
        "help" => {
            let mut o = Overlay::new(OverlayKind::Help, "Commands");
            o.rows = super::composer::COMMANDS
                .iter()
                .map(|(n, d)| row(*n, format!("/{n}"), *d))
                .collect();
            o.status = "Search · ↑↓ navigate · Enter runs · Esc closes".into();
            ui.overlay = Some(o);
        }
        "themes" => {
            let mut o = Overlay::new(OverlayKind::Themes, "Appearance");
            o.rows = Theme::ALL
                .iter()
                .map(|t| row(t.name(), t.name(), "Applies to this terminal session"))
                .collect();
            o.status = "Enter selects · System uses terminal colors".into();
            ui.overlay = Some(o);
        }
        "tools" | "subagents" => {
            let child = command == "subagents";
            let mut o = Overlay::new(
                if child {
                    OverlayKind::Subagents
                } else {
                    OverlayKind::Tools
                },
                if child { "Subagents" } else { "Tool calls" },
            );
            o.rows = state
                .entries()
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    e.activity.as_ref().filter(|a| a.child == child).map(|a| {
                        row(
                            i.to_string(),
                            a.summary(),
                            "Enter inspects arguments/output · read-only",
                        )
                    })
                })
                .collect();
            o.status = if o.rows.is_empty() {
                "No activity in this conversation".into()
            } else {
                "Enter inspects · Esc returns to the parent".into()
            };
            ui.overlay = Some(o);
        }
        "mouse" => ui.mouse_enabled = !ui.mouse_enabled,
        "chat" => {
            ui.active_tab = AppTab::Chat;
            ui.overlay = None;
        }
        _ => return false,
    }
    ui.focus = 0;
    true
}
pub fn select_theme(id: &str, ui: &mut UiState) {
    if let Some(t) = Theme::ALL.iter().find(|t| t.name() == id) {
        ui.theme = *t;
    }
    ui.overlay = None;
    ui.focus = 0;
}
pub fn inspect(id: &str, state: &TranscriptState, ui: &mut UiState) {
    let Some(a) = id
        .parse::<usize>()
        .ok()
        .and_then(|i| state.entries().get(i))
        .and_then(|e| e.activity.as_ref())
    else {
        return;
    };
    let mut o = Overlay::new(OverlayKind::ToolDetail, a.summary());
    o.rows = a
        .details()
        .lines()
        .enumerate()
        .map(|(i, l)| row(i.to_string(), l, ""))
        .collect();
    o.status = "Stored preview · ↑↓ / wheel scroll · Esc returns to parent".into();
    ui.overlay = Some(o);
    ui.focus = 0;
}
