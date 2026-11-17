use super::*;
pub(super) fn overlay(frame: &mut Frame, area: Rect, ui: &mut UiState, p: Palette) {
    let width = area.width.saturating_sub(4).min(100);
    let height = area.height.saturating_sub(4).min(30);
    let panel = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, panel);
    ui.overlay_area = panel;
    let current = ui.overlay.as_ref().unwrap();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", safe_text(&current.title)))
        .style(Style::default().fg(p.text).bg(p.surface))
        .border_style(Style::default().fg(p.border));
    let inner = block.inner(panel);
    frame.render_widget(block, panel);
    ui.hits.clear();
    let current = ui.overlay.as_ref().unwrap();
    let selected = current.selected;
    let kind = current.kind;
    let visible = current.visible_rows();
    let detail_height = if kind == OverlayKind::ToolDetail {
        0
    } else {
        5.min(inner.height / 3)
    };
    let list_height = inner.height.saturating_sub(detail_height + 3).max(1) as usize;
    let start = selected
        .saturating_sub(list_height / 2)
        .min(visible.len().saturating_sub(list_height));
    let rows: Vec<_> = visible
        .iter()
        .skip(start)
        .take(list_height)
        .enumerate()
        .map(|(offset, row)| (start + offset, row.label.clone()))
        .collect();
    let detail = visible
        .get(selected)
        .map(|row| row.detail.clone())
        .unwrap_or_else(|| "No matching items".into());
    let prompt = if let Some(input) = &current.input {
        format!("> {input}▏")
    } else {
        format!("Search: {}▏", current.filter)
    };
    let status = current.status.clone();
    for (offset, (index, label)) in rows.into_iter().enumerate() {
        let target = Rect::new(inner.x, inner.y + offset as u16, inner.width, 1);
        let style = if index == selected {
            Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.text)
        };
        text(
            frame,
            target,
            format!("{} {label}", if index == selected { "›" } else { " " }),
            style,
        );
        ui.hits.push(Hit {
            area: target,
            action: Action::OverlayRow(index),
        });
    }
    if detail_height > 0 {
        frame.render_widget(
            Paragraph::new(safe_text(&detail))
                .style(Style::default().fg(p.muted))
                .wrap(Wrap { trim: false }),
            Rect::new(
                inner.x,
                inner.y + list_height as u16,
                inner.width,
                detail_height,
            ),
        );
    }
    text(
        frame,
        Rect::new(inner.x, inner.bottom() - 3, inner.width, 1),
        prompt,
        Style::default().fg(p.text),
    );
    text(
        frame,
        Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
        status,
        Style::default().fg(p.muted),
    );
    let y = inner.bottom() - 1;
    if kind == OverlayKind::Approvals {
        for (x, label, key) in [
            (0, "Approve once", crossterm::event::KeyCode::Char('1')),
            (15, "Deny", crossterm::event::KeyCode::Delete),
        ] {
            button(
                frame,
                Rect::new(inner.x + x, y, 13, 1),
                label,
                Action::OverlayKey(key),
                ui,
                p,
            );
        }
    } else if kind == OverlayKind::PlanReview {
        for (x, label, key) in [
            (0, "Approve", 'a'),
            (12, "Reject", 'r'),
            (24, "Revise", 'e'),
        ] {
            button(
                frame,
                Rect::new(inner.x + x, y, 10, 1),
                label,
                Action::OverlayKey(crossterm::event::KeyCode::Char(key)),
                ui,
                p,
            );
        }
    } else {
        button(
            frame,
            Rect::new(inner.x, y, 12, 1),
            "Select ↵",
            Action::OverlayKey(crossterm::event::KeyCode::Enter),
            ui,
            p,
        );
    }
    button(
        frame,
        Rect::new(inner.right().saturating_sub(10), y, 10, 1),
        "Close Esc",
        Action::OverlayKey(crossterm::event::KeyCode::Esc),
        ui,
        p,
    );
}
