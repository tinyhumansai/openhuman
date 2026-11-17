use super::*;
pub(super) fn config(frame: &mut Frame, area: Rect, ui: &mut UiState, p: Palette) {
    text(
        frame,
        Rect::new(area.x + 2, area.y, area.width.saturating_sub(4), 1),
        "Configuration · changes apply through the core",
        Style::default().fg(p.text),
    );
    for index in 0..ui.config_items.len() {
        let item = &ui.config_items[index];
        let label = format!(
            "{} {:<18} {}",
            if index == ui.config_selected {
                "›"
            } else {
                " "
            },
            item.label,
            item.value
        );
        button(
            frame,
            Rect::new(
                area.x + 2,
                area.y + 2 + index as u16,
                area.width.saturating_sub(4),
                1,
            ),
            &label,
            Action::ConfigRow(index),
            ui,
            p,
        );
    }
    let detail = if let Some(value) = &ui.config_edit {
        format!("> {value}▏\nEnter save · Esc cancel")
    } else {
        format!(
            "{}\n{}",
            ui.config_items[ui.config_selected].hint, ui.config_status
        )
    };
    if area.height > 9 {
        frame.render_widget(
            Paragraph::new(safe_text(&detail))
                .style(Style::default().fg(p.muted))
                .wrap(Wrap { trim: false }),
            Rect::new(
                area.x + 2,
                area.y + 9,
                area.width.saturating_sub(4),
                area.height - 9,
            ),
        );
    }
}
pub(super) fn settings(frame: &mut Frame, area: Rect, ui: &mut UiState, p: Palette) {
    text(
        frame,
        Rect::new(area.x + 2, area.y, area.width.saturating_sub(4), 1),
        format!("Account · {}", ui.auth_summary),
        Style::default().fg(p.text).add_modifier(Modifier::BOLD),
    );
    text(
        frame,
        Rect::new(area.x + 2, area.y + 1, area.width.saturating_sub(4), 1),
        &ui.account_detail,
        Style::default().fg(p.muted),
    );
    for (index, action) in SettingsAction::ALL.iter().enumerate() {
        button(
            frame,
            Rect::new(
                area.x + 2,
                area.y + 3 + index as u16,
                area.width.saturating_sub(4),
                1,
            ),
            action.label(),
            Action::SettingsRow(index),
            ui,
            p,
        );
    }
    button(
        frame,
        Rect::new(area.x + 2, area.y + 7, 25, 1),
        &format!("Appearance · {} ▾", ui.theme.name()),
        Action::Command("themes"),
        ui,
        p,
    );
    button(
        frame,
        Rect::new(area.x + 2, area.y + 8, 30, 1),
        if ui.mouse_enabled {
            "Mouse · on (click to disable)"
        } else {
            "Mouse · off"
        },
        Action::Command("mouse"),
        ui,
        p,
    );
    let detail = if let Some(token) = &ui.login_token {
        format!(
            "Paste a one-time login token\n> {}▏\nEnter sign in · Esc cancel",
            "•".repeat(
                token
                    .chars()
                    .count()
                    .min(area.width.saturating_sub(6) as usize)
            )
        )
    } else if ui.logout_confirm {
        "Log out of this account?\nChoose Confirm or Cancel.".into()
    } else {
        ui.settings_status.clone()
    };
    if area.height > 10 {
        frame.render_widget(
            Paragraph::new(detail)
                .style(Style::default().fg(p.muted))
                .wrap(Wrap { trim: false }),
            Rect::new(
                area.x + 2,
                area.y + 10,
                area.width.saturating_sub(4),
                area.height - 10,
            ),
        );
    }
    if ui.login_url.is_some() && area.height > 12 {
        for (index, (label, command)) in [
            ("Open browser", "login-open"),
            ("Copy link", "login-copy"),
            ("Cancel", "login-cancel"),
        ]
        .iter()
        .enumerate()
        {
            let width = area.width.saturating_sub(4) / 3;
            button(
                frame,
                Rect::new(
                    area.x + 2 + index as u16 * width,
                    area.bottom().saturating_sub(2),
                    width,
                    1,
                ),
                label,
                Action::Command(command),
                ui,
                p,
            );
        }
    } else if ui.login_token.is_some() || ui.logout_confirm {
        button(
            frame,
            Rect::new(area.x + 2, area.bottom().saturating_sub(2), 12, 1),
            "Confirm",
            Action::OverlayKey(crossterm::event::KeyCode::Enter),
            ui,
            p,
        );
        button(
            frame,
            Rect::new(area.x + 16, area.bottom().saturating_sub(2), 12, 1),
            "Cancel",
            Action::OverlayKey(crossterm::event::KeyCode::Esc),
            ui,
            p,
        );
    }
}
pub(super) fn logs(frame: &mut Frame, area: Rect, ui: &mut UiState, p: Palette) {
    let lines = openhuman_rpc::embed::process::tui_log_lines();
    let end = lines
        .len()
        .saturating_sub(ui.log_scroll_from_bottom as usize);
    let start = end.saturating_sub(area.height as usize);
    text(
        frame,
        area,
        lines[start..end]
            .iter()
            .map(|line| safe_text(line))
            .collect::<Vec<_>>()
            .join("\n"),
        Style::default().fg(p.muted),
    );
}
