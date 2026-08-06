//! Ratatui rendering for the tabbed terminal UI — pure view over
//! [`TranscriptState`] + [`UiState`]. No state mutation happens here.
//!
//! Layout (top → bottom):
//!   * transcript viewport (fills remaining height, wraps + scrolls)
//!   * single-line input box (bordered)
//!   * status bar (thread id, turn state, key hints)

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::state::{EntryKind, TranscriptState};
use super::ui_state::{AppTab, SettingsAction, UiState};

/// Ocean accent from the design tokens (`#4A83DD`), kept terminal-native.
const OCEAN: Color = Color::Rgb(0x4A, 0x83, 0xDD);

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Draw one frame.
pub fn draw(frame: &mut Frame, state: &TranscriptState, ui: &UiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_tabs(frame, chunks[0], ui);
    match ui.active_tab {
        AppTab::Logs => draw_logs(frame, chunks[1], ui),
        AppTab::Chat => draw_chat(frame, chunks[1], state, ui),
        AppTab::Config => draw_config(frame, chunks[1], ui),
        AppTab::Settings => draw_settings(frame, chunks[1], ui),
    }
    draw_footer(frame, chunks[2], state, ui);
}

fn draw_tabs(frame: &mut Frame, area: Rect, ui: &UiState) {
    let selected = AppTab::ALL
        .iter()
        .position(|tab| *tab == ui.active_tab)
        .unwrap_or(0);
    let titles = AppTab::ALL
        .iter()
        .enumerate()
        .map(|(idx, tab)| Line::from(format!(" {} {} ", idx + 1, tab.title())))
        .collect::<Vec<_>>();
    let tabs = Tabs::new(titles)
        .select(selected)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" OpenHuman CLI "),
        )
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(Style::default().fg(OCEAN).add_modifier(Modifier::BOLD));
    frame.render_widget(tabs, area);
}

fn draw_chat(frame: &mut Frame, area: Rect, state: &TranscriptState, ui: &UiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);
    draw_transcript(frame, chunks[0], state, ui);
    draw_input(frame, chunks[1], ui);
}

fn draw_logs(frame: &mut Frame, area: Rect, ui: &UiState) {
    let lines = crate::core::logging::tui_log_lines();
    let text = if lines.is_empty() {
        Text::from("Core logs will appear here as OpenHuman starts.")
    } else {
        Text::from(lines.join("\n"))
    };
    let inner_height = area.height.saturating_sub(2).max(1);
    let inner_width = area.width.saturating_sub(2).max(1);
    let total_rows = text
        .lines
        .iter()
        .map(|line| u32::from(wrapped_line_count(line, inner_width)))
        .sum::<u32>();
    let max_scroll = total_rows
        .saturating_sub(u32::from(inner_height))
        .min(u32::from(u16::MAX)) as u16;
    let top = max_scroll.saturating_sub(ui.log_scroll_from_bottom.min(max_scroll));
    let paragraph = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(" Core logs "))
        .style(Style::default().fg(Color::Gray))
        .wrap(Wrap { trim: false })
        .scroll((top, 0));
    frame.render_widget(paragraph, area);
}

fn draw_config(frame: &mut Frame, area: Rect, ui: &UiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(4)])
        .split(area);
    let items = ui
        .config_items
        .iter()
        .map(|item| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<18}", item.label),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    if item.value.is_empty() {
                        "(not set)"
                    } else {
                        &item.value
                    },
                    Style::default().fg(Color::White),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    let mut list_state = ListState::default().with_selected(Some(ui.config_selected));
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Safe configuration "),
        )
        .highlight_symbol("› ")
        .highlight_style(Style::default().fg(OCEAN).add_modifier(Modifier::BOLD));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let selected = &ui.config_items[ui.config_selected.min(ui.config_items.len() - 1)];
    let detail = if let Some(input) = &ui.config_edit {
        let visible = tail_to_width(input, chunks[1].width.saturating_sub(5) as usize);
        format!("Editing {}\n> {}▏", selected.label, visible)
    } else {
        format!("{}\n{}", selected.hint, ui.config_status)
    };
    frame.render_widget(
        Paragraph::new(detail)
            .block(Block::default().borders(Borders::ALL).title(" Edit "))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn draw_settings(frame: &mut Frame, area: Rect, ui: &UiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(3),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                &ui.auth_summary,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(ui.account_detail.clone()),
        ])
        .block(Block::default().borders(Borders::ALL).title(" Account "))
        .wrap(Wrap { trim: false }),
        chunks[0],
    );

    let actions = SettingsAction::ALL
        .iter()
        .map(|action| ListItem::new(action.label()))
        .collect::<Vec<_>>();
    let mut action_state = ListState::default().with_selected(Some(ui.settings_selected));
    frame.render_stateful_widget(
        List::new(actions)
            .block(Block::default().borders(Borders::ALL).title(" Actions "))
            .highlight_symbol("› ")
            .highlight_style(Style::default().fg(OCEAN).add_modifier(Modifier::BOLD)),
        chunks[1],
        &mut action_state,
    );

    let detail = if let Some(token) = &ui.login_token {
        let visible = "•".repeat(
            token
                .chars()
                .count()
                .min(chunks[2].width.saturating_sub(5) as usize),
        );
        format!(
            "Paste a one-time login token, then press Enter.\n> {}▏",
            visible
        )
    } else if ui.logout_confirm {
        "Log out and stop account-bound services? Press y to confirm or Esc to cancel.".to_string()
    } else {
        ui.settings_status.clone()
    };
    frame.render_widget(
        Paragraph::new(detail)
            .block(Block::default().borders(Borders::ALL).title(" Status "))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
}

fn draw_transcript(frame: &mut Frame, area: Rect, state: &TranscriptState, ui: &UiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" OpenHuman chat ")
        .border_style(Style::default().fg(OCEAN));
    let inner = block.inner(area);

    let text = transcript_text(state);
    // Inner width available for wrapping (borders eat 2 cols).
    let wrap_width = inner.width.max(1);
    let total_lines: u16 = text
        .lines
        .iter()
        .map(|line| wrapped_line_count(line, wrap_width))
        .sum::<u16>();
    let viewport = inner.height.max(1);
    let max_scroll = total_lines.saturating_sub(viewport);
    let scroll_from_bottom = ui.scroll_from_bottom.min(max_scroll);
    let top = max_scroll.saturating_sub(scroll_from_bottom);

    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((top, 0));
    frame.render_widget(paragraph, area);
}

fn draw_input(frame: &mut Frame, area: Rect, ui: &UiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Message ")
        .border_style(Style::default().fg(Color::DarkGray));
    let inner_width = block.inner(area).width.max(1) as usize;

    // Keep the caret end of a long input visible.
    let display = tail_to_width(&ui.input, inner_width.saturating_sub(1));
    let line = Line::from(vec![
        Span::styled(display, Style::default().fg(Color::White)),
        Span::styled("▏", Style::default().fg(OCEAN)),
    ]);
    let paragraph = Paragraph::new(line).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_footer(frame: &mut Frame, area: Rect, state: &TranscriptState, ui: &UiState) {
    let context = match ui.active_tab {
        AppTab::Logs => "PgUp/PgDn scroll",
        AppTab::Chat => "Enter send · Esc cancel · Ctrl+N new · PgUp/PgDn scroll",
        AppTab::Config => {
            if ui.config_edit.is_some() {
                "Enter save · Esc cancel"
            } else {
                "↑↓ navigate · Enter edit"
            }
        }
        AppTab::Settings => {
            if ui.is_editing() {
                "Enter confirm · Esc cancel"
            } else {
                "↑↓ navigate · Enter select"
            }
        }
    };
    let turn = if ui.active_tab == AppTab::Chat && state.is_streaming() {
        let frame_ch = SPINNER_FRAMES[ui.spinner_tick % SPINNER_FRAMES.len()];
        format!("{frame_ch} streaming")
    } else {
        ui.active_tab.title().to_string()
    };

    let left = Span::styled(
        format!(" {turn} "),
        Style::default().fg(Color::Black).bg(OCEAN),
    );
    let navigation = if ui.is_editing() {
        "Finish or Esc before switching tabs"
    } else {
        "Tab/Shift+Tab switch · Alt+1-4 tabs"
    };
    let hints = Span::styled(
        format!("  {navigation} · {context} · Ctrl+C quit"),
        Style::default().fg(Color::DarkGray),
    );
    let paragraph = Paragraph::new(Line::from(vec![left, hints]));
    frame.render_widget(paragraph, area);
}

/// Build the styled transcript body from the reducer state.
fn transcript_text(state: &TranscriptState) -> Text<'static> {
    let mut lines: Vec<Line> = Vec::new();
    for entry in state.entries() {
        let (prefix, style) = match entry.kind {
            EntryKind::User => (
                "You  ",
                Style::default().fg(OCEAN).add_modifier(Modifier::BOLD),
            ),
            EntryKind::Assistant => ("AI   ", Style::default().fg(Color::White)),
            EntryKind::Thinking => (
                "···  ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
            EntryKind::Tool => ("tool ", Style::default().fg(Color::Yellow)),
            EntryKind::Error => (
                "err  ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            EntryKind::System => ("     ", Style::default().fg(Color::DarkGray)),
        };

        let mut first = true;
        for raw in entry.text.split('\n') {
            let gutter = if first { prefix } else { "     " };
            lines.push(Line::from(vec![
                Span::styled(gutter.to_string(), style.add_modifier(Modifier::DIM)),
                Span::styled(raw.to_string(), style),
            ]));
            first = false;
        }
        // Blank spacer between entries for readability.
        lines.push(Line::from(""));
    }
    Text::from(lines)
}

/// Approximate the number of visual rows a wrapped line occupies at `width`.
/// ratatui wraps on word boundaries; this display-width estimate is close
/// enough for scroll bookkeeping (off-by-one at most, harmless).
fn wrapped_line_count(line: &Line, width: u16) -> u16 {
    let w = width.max(1) as usize;
    let content_width: usize = line
        .spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    if content_width == 0 {
        1
    } else {
        content_width.div_ceil(w).max(1) as u16
    }
}

/// Return the trailing slice of `s` that fits within `width` display columns.
fn tail_to_width(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out: Vec<char> = Vec::new();
    let mut used = 0usize;
    for ch in s.chars().rev() {
        let cw = UnicodeWidthStr::width(ch.to_string().as_str()).max(1);
        if used + cw > width {
            break;
        }
        used += cw;
        out.push(ch);
    }
    out.into_iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn rendered(ui: &UiState) -> String {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let transcript = TranscriptState::new("test-client");
        terminal
            .draw(|frame| draw(frame, &transcript, ui))
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn tab_bar_and_navigation_footer_are_always_rendered() {
        let ui = UiState::new("thread-1".into(), "client-1".into());
        let output = rendered(&ui);
        for title in ["1 Logs", "2 Chat", "3 Config", "4 Settings"] {
            assert!(output.contains(title), "missing tab {title}");
        }
        assert!(output.contains("Tab/Shift+Tab switch"));
        assert!(output.contains("PgUp/PgDn scroll"));
    }

    #[test]
    fn editing_footer_explains_that_tab_switching_is_paused() {
        let mut ui = UiState::new("thread-1".into(), "client-1".into());
        ui.active_tab = AppTab::Config;
        ui.config_edit = Some("value".to_string());
        let output = rendered(&ui);
        assert!(output.contains("Finish or Esc before switching tabs"));
        assert!(!output.contains("Alt+1-4 tabs"));
    }

    #[test]
    fn tail_to_width_keeps_the_end() {
        assert_eq!(tail_to_width("hello world", 5), "world");
        assert_eq!(tail_to_width("hi", 10), "hi");
        assert_eq!(tail_to_width("anything", 0), "");
    }

    #[test]
    fn wrapped_line_count_divides_by_width() {
        let line = Line::from("a".repeat(25));
        assert_eq!(wrapped_line_count(&line, 10), 3);
        let empty = Line::from("");
        assert_eq!(wrapped_line_count(&empty, 10), 1);
    }
}
