//! Pointer translation is independent of runtime effects.
use super::actions::{hit_at, Action};
use super::ui_state::{AppTab, UiState};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

pub fn mouse_action(mouse: MouseEvent, ui: &mut UiState) -> Option<Action> {
    let position = Position::new(mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let up = mouse.kind == MouseEventKind::ScrollUp;
            if let Some(overlay) = &mut ui.overlay {
                overlay.selected = if up {
                    overlay.selected.saturating_sub(3)
                } else {
                    (overlay.selected + 3).min(overlay.visible_rows().len().saturating_sub(1))
                };
            } else if ui.active_tab == AppTab::Logs {
                ui.log_scroll_from_bottom = if up {
                    ui.log_scroll_from_bottom.saturating_add(3)
                } else {
                    ui.log_scroll_from_bottom.saturating_sub(3)
                };
            } else if ui.transcript_area.contains(position) {
                ui.scroll_from_bottom = if up {
                    ui.scroll_from_bottom.saturating_add(3)
                } else {
                    ui.scroll_from_bottom.saturating_sub(3)
                };
            }
            None
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let action = hit_at(&ui.hits, mouse.column, mouse.row);
            if action.is_none()
                && !ui.overlay_area.contains(position)
                && ui.overlay.as_ref().is_some_and(|o| {
                    !matches!(
                        o.kind,
                        super::cockpit::OverlayKind::Approvals
                            | super::cockpit::OverlayKind::PlanReview
                            | super::cockpit::OverlayKind::ConfirmDelete
                    )
                })
            {
                ui.overlay = None;
                ui.focus = 0;
                return None;
            }
            if action == Some(Action::Composer) {
                let area = ui.composer_area;
                ui.composer.click(
                    (mouse.row - area.y) as usize + ui.composer_first_row,
                    (mouse.column - area.x) as usize,
                    area.width.saturating_sub(1) as usize,
                );
                ui.focus = 0;
            }
            action
        }
        MouseEventKind::Moved => {
            if let Some(index) = ui.hits.iter().position(|hit| hit.area.contains(position)) {
                if let Action::OverlayRow(row) = ui.hits[index].action {
                    if let Some(overlay) = &mut ui.overlay {
                        overlay.selected = row;
                    }
                } else {
                    ui.focus = if ui.hits[index].action == Action::Composer {
                        0
                    } else {
                        index + 1
                    };
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
