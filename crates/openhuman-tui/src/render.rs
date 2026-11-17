//! Conversation-first rendering; each frame owns its cell-based input targets.
use super::actions::{Action, Hit};
use super::activity::Status;
use super::cockpit::OverlayKind;
use super::state::{EntryKind, TranscriptState};
use super::theme::{safe_text, Palette};
use super::ui_state::{AppTab, SettingsAction, UiState};
mod overlay;
mod views;
use overlay::overlay;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;
use views::{config, logs, settings};

pub fn draw(frame: &mut Frame, state: &TranscriptState, ui: &mut UiState) {
    let area = frame.area();
    let p = ui.theme.palette();
    ui.hits.clear();
    frame.render_widget(
        Block::default().style(Style::default().fg(p.text).bg(p.background)),
        area,
    );
    if area.width < 24 || area.height < 8 {
        ui.transcript_area = Rect::default();
        ui.composer_area = Rect::default();
        text(
            frame,
            area,
            "Resize terminal · Ctrl+D exits",
            Style::default().fg(p.text),
        );
        return;
    }
    text(
        frame,
        Rect::new(area.x + 1, area.y, 12, 1),
        "OpenHuman",
        Style::default().fg(p.text).add_modifier(Modifier::BOLD),
    );
    button(
        frame,
        Rect::new(area.x + 13, area.y, 6, 1),
        "Chat",
        Action::View(AppTab::Chat),
        ui,
        p,
    );
    let mut x = area.right().saturating_sub(39).max(area.x + 20);
    for (label, cmd) in [
        ("Sessions", "sessions"),
        ("Agents", "agents"),
        ("Tools", "tools"),
        ("Settings", "settings"),
    ] {
        let width = label.len() as u16 + 2;
        if x + width <= area.right() {
            button(
                frame,
                Rect::new(x, area.y, width, 1),
                label,
                Action::Command(cmd),
                ui,
                p,
            );
        }
        x += width;
    }
    text(
        frame,
        Rect::new(area.x + 1, area.y + 1, area.width - 2, 1),
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(p.border),
    );
    let body = Rect::new(area.x, area.y + 2, area.width, area.height - 3);
    match ui.active_tab {
        AppTab::Chat => chat(frame, body, state, ui, p),
        AppTab::Logs => logs(frame, body, ui, p),
        AppTab::Config => config(frame, body, ui, p),
        AppTab::Settings => settings(frame, body, ui, p),
    }
    let run = if ui.stopping {
        "Stopping".to_string()
    } else if state.is_streaming() {
        format!("{} Working", ["·", "•", "●", "•"][ui.spinner_tick % 4])
    } else {
        if ui.active_tab == AppTab::Chat {
            "Ready".into()
        } else {
            ui.active_tab.title().into()
        }
    };
    text(
        frame,
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
        format!(
            " {run} · {}{}   Ctrl+P commands · Ctrl+J newline · Ctrl+D exit",
            ui.auth_summary,
            if ui.demo { " · DEMO" } else { "" }
        ),
        Style::default().fg(p.muted),
    );
    if ui.overlay.is_some() {
        overlay(frame, area, ui, p);
    }
    ui.focus = ui.focus.min(ui.hits.len());
}

fn text(frame: &mut Frame, area: Rect, value: impl Into<String>, style: Style) {
    frame.render_widget(Paragraph::new(safe_text(&value.into())).style(style), area);
}
fn button(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    action: Action,
    ui: &mut UiState,
    p: Palette,
) {
    if area.width == 0
        || area.height == 0
        || area.right() > frame.area().right()
        || area.bottom() > frame.area().bottom()
    {
        return;
    }
    let focused = ui.focus > 0 && ui.focus - 1 == ui.hits.len();
    let style = if focused {
        Style::default()
            .fg(p.background)
            .bg(p.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.muted)
    };
    text(frame, area, label, style);
    ui.hits.push(Hit { area, action });
}

fn chat(frame: &mut Frame, area: Rect, state: &TranscriptState, ui: &mut UiState, p: Palette) {
    let (rows, _, _) = ui
        .composer
        .display(area.width.saturating_sub(3).max(1) as usize);
    let count = ui.composer.command_matches().len().min(4);
    let height = ((rows.len().min(5) + 3 + count) as u16)
        .min(area.height.saturating_sub(1))
        .max(4);
    let transcript_height = area.height.saturating_sub(height + 1);
    ui.transcript_area = Rect::new(area.x + 1, area.y, area.width - 2, transcript_height);
    let (visible, max_scroll) = ui.viewport.rows(
        state,
        ui.transcript_area.width.saturating_sub(2),
        transcript_height,
        ui.scroll_from_bottom,
    );
    ui.scroll_from_bottom = ui.viewport.resolved_offset().min(max_scroll);
    if visible.is_empty() && transcript_height >= 5 {
        text(
            frame,
            Rect::new(area.x + 3, area.y + 2, area.width.saturating_sub(6), 1),
            "What would you like to work on?",
            Style::default().fg(p.text).add_modifier(Modifier::BOLD),
        );
        text(
            frame,
            Rect::new(area.x + 3, area.y + 4, area.width.saturating_sub(6), 1),
            "Describe a task, or open Commands to get started.",
            Style::default().fg(p.muted),
        );
    }
    for (row, visible) in visible.iter().enumerate() {
        let entry = &state.entries()[visible.entry];
        let color = if let Some(a) = &entry.activity {
            match a.status {
                Status::Success => p.success,
                Status::Error => p.error,
                Status::Waiting => p.warning,
                _ => p.muted,
            }
        } else {
            match visible.kind {
                EntryKind::User => p.accent,
                EntryKind::Thinking | EntryKind::System => p.muted,
                EntryKind::Error => p.error,
                _ => p.text,
            }
        };
        let mut style = Style::default().fg(color);
        if visible.first && matches!(visible.kind, EntryKind::User | EntryKind::Assistant) {
            style = style.add_modifier(Modifier::BOLD);
        }
        let target = Rect::new(
            ui.transcript_area.x,
            ui.transcript_area.y + row as u16,
            ui.transcript_area.width,
            1,
        );
        if entry.activity.is_some() || visible.kind == EntryKind::Thinking {
            let prefix = if entry.expanded { "▾ " } else { "▸ " };
            text(
                frame,
                target,
                if visible.first {
                    format!("{prefix}{}", visible.text)
                } else {
                    format!("  {}", visible.text)
                },
                style,
            );
            ui.hits.push(Hit {
                area: target,
                action: Action::Entry(visible.entry),
            });
        } else {
            text(frame, target, &visible.text, style);
        }
    }
    let y = area.y + transcript_height;
    if !ui.pending_approvals.is_empty() {
        button(
            frame,
            Rect::new(area.x + 1, y, area.width.saturating_sub(20).min(36), 1),
            "! Pending approval · Review",
            Action::Command("approvals"),
            ui,
            p,
        );
    } else if ui.pending_plan_review.is_some() {
        button(
            frame,
            Rect::new(area.x + 1, y, area.width.saturating_sub(20).min(36), 1),
            "! Plan review · Open",
            Action::Command("plan"),
            ui,
            p,
        );
    }
    if ui.scroll_from_bottom > 0 && area.width >= 40 {
        button(
            frame,
            Rect::new(area.right() - 18, y, 17, 1),
            "Jump to latest ↓",
            Action::Latest,
            ui,
            p,
        );
    }
    composer(
        frame,
        Rect::new(
            area.x,
            area.bottom().saturating_sub(height),
            area.width,
            height,
        ),
        state,
        ui,
        p,
    );
}

fn composer(frame: &mut Frame, area: Rect, state: &TranscriptState, ui: &mut UiState, p: Palette) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if ui.focus == 0 { p.accent } else { p.border }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let matches = ui.composer.command_matches();
    let count = matches
        .len()
        .min(4)
        .min(inner.height.saturating_sub(2) as usize);
    let selected = ui.suggestion_selected.min(matches.len().saturating_sub(1));
    let first = selected.saturating_sub(count.saturating_sub(1));
    for (row, (name, desc)) in matches.iter().skip(first).take(count).enumerate() {
        button(
            frame,
            Rect::new(inner.x, inner.y + row as u16, inner.width, 1),
            &format!(
                "{} /{name:<12} {desc}",
                if row + first == selected { "›" } else { " " }
            ),
            Action::Complete(name),
            ui,
            p,
        );
    }
    let editor_height = inner.height.saturating_sub(count as u16 + 1).max(1);
    ui.composer_area = Rect::new(inner.x, inner.y + count as u16, inner.width, editor_height);
    let width = inner.width.saturating_sub(1).max(1) as usize;
    let (rows, cursor_row, cursor_col) = ui.composer.display(width);
    let first = cursor_row.saturating_sub(editor_height as usize - 1);
    ui.composer_first_row = first;
    if ui.composer.is_empty() {
        text(
            frame,
            ui.composer_area,
            "Describe a task…",
            Style::default().fg(p.muted),
        );
    } else {
        for (row, line) in rows
            .iter()
            .skip(first)
            .take(editor_height as usize)
            .enumerate()
        {
            text(
                frame,
                Rect::new(inner.x, ui.composer_area.y + row as u16, inner.width, 1),
                line,
                Style::default().fg(p.text),
            );
        }
    }
    ui.hits.push(Hit {
        area: ui.composer_area,
        action: Action::Composer,
    });
    if ui.focus == 0 && ui.overlay.is_none() {
        frame.set_cursor_position(Position::new(
            inner.x + cursor_col.min(width) as u16,
            ui.composer_area.y + (cursor_row - first) as u16,
        ));
    }
    let y = inner.bottom().saturating_sub(1);
    let agent = format!("{} ▾", ui.agent_name);
    let reserved = if inner.width >= 64 {
        30
    } else if state.is_streaming() {
        19
    } else {
        10
    };
    let selectors_width = inner.width.saturating_sub(reserved);
    let aw = (UnicodeWidthStr::width(agent.as_str()) as u16)
        .min(selectors_width / 2)
        .max(1);
    button(
        frame,
        Rect::new(inner.x, y, aw, 1),
        &agent,
        Action::Command("agents"),
        ui,
        p,
    );
    button(
        frame,
        Rect::new(
            inner.x + aw + 1,
            y,
            selectors_width.saturating_sub(aw + 1).min(28),
            1,
        ),
        &format!(
            "{} ▾",
            ui.model_override.as_deref().unwrap_or(&ui.effective_model)
        ),
        Action::Command("model"),
        ui,
        p,
    );
    if inner.width >= 64 {
        button(
            frame,
            Rect::new(inner.right() - 29, y, 10, 1),
            "Commands",
            Action::Command("help"),
            ui,
            p,
        );
    }
    if state.is_streaming() {
        button(
            frame,
            Rect::new(inner.right() - 18, y, 8, 1),
            "Queue",
            Action::Queue,
            ui,
            p,
        );
        button(
            frame,
            Rect::new(inner.right() - 9, y, 8, 1),
            "Stop",
            Action::Stop,
            ui,
            p,
        );
    } else {
        button(
            frame,
            Rect::new(inner.right() - 9, y, 8, 1),
            "Send ↵",
            Action::Send,
            ui,
            p,
        );
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
