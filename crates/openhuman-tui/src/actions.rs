//! Shared hit targets and actions for keyboard and pointer navigation.
use super::ui_state::AppTab;
use crossterm::event::KeyCode;
use ratatui::layout::{Position, Rect};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Command(&'static str),
    View(AppTab),
    Send,
    Queue,
    Stop,
    Latest,
    Entry(usize),
    OverlayRow(usize),
    OverlayKey(KeyCode),
    ConfigRow(usize),
    SettingsRow(usize),
    Composer,
    Complete(&'static str),
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub area: Rect,
    pub action: Action,
}

pub fn hit_at(hits: &[Hit], column: u16, row: u16) -> Option<Action> {
    hits.iter()
        .rev()
        .find(|hit| hit.area.contains(Position::new(column, row)))
        .map(|hit| hit.action.clone())
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
