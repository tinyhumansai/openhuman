//! Git review, clipboard and export actions.
use super::*;

pub(super) async fn open_git_diff(ui: &mut UiState) {
    let cwd = ui.action_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        if cwd.is_empty() {
            return Err("Action directory is unavailable; run /status first.".to_string());
        }
        let inside = std::process::Command::new("git")
            .args(["-C", &cwd, "rev-parse", "--is-inside-work-tree"])
            .output()
            .map_err(|error| error.to_string())?;
        if !inside.status.success() {
            return Err("The action directory is not a Git repository.".into());
        }
        let status = std::process::Command::new("git")
            .args(["-C", &cwd, "status", "--short", "--branch"])
            .output()
            .map_err(|error| error.to_string())?;
        let output = std::process::Command::new("git")
            .args(["-C", &cwd, "diff", "--no-ext-diff", "--stat", "--patch"])
            .output()
            .map_err(|error| error.to_string())?;
        Ok(format!(
            "{}\n{}",
            String::from_utf8_lossy(&status.stdout),
            String::from_utf8_lossy(&output.stdout)
        ))
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));
    let mut overlay = Overlay::new(OverlayKind::Diff, "Working-tree diff");
    match result {
        Ok(diff) if diff.trim().is_empty() => overlay.rows.push(text_row(
            "clean",
            "Working tree is clean",
            "No tracked changes",
        )),
        Ok(diff) => {
            const MAX_DIFF_LINES: usize = 2_000;
            let line_count = diff.lines().count();
            overlay.rows = diff
                .lines()
                .take(MAX_DIFF_LINES)
                .enumerate()
                .map(|(index, line)| text_row(&index.to_string(), line, ""))
                .collect();
            if line_count > MAX_DIFF_LINES {
                overlay.status = format!(
                    "Showing {MAX_DIFF_LINES} of {line_count} lines; remaining lines omitted"
                );
            }
        }
        Err(error) => overlay.status = error,
    }
    ui.overlay = Some(overlay);
}

pub(super) fn export_transcript(state: &mut TranscriptState, ui: &UiState, argument: &str) {
    let path = if argument.trim().is_empty() {
        let base = if ui.action_dir.is_empty() {
            std::env::current_dir().unwrap_or_default()
        } else {
            std::path::PathBuf::from(&ui.action_dir)
        };
        base.join(format!(
            "openhuman-{}.md",
            ui.thread_id.replace(['/', '\\'], "-")
        ))
    } else {
        let supplied = std::path::PathBuf::from(argument);
        if supplied.is_absolute() || ui.action_dir.is_empty() {
            supplied
        } else {
            std::path::PathBuf::from(&ui.action_dir).join(supplied)
        }
    };
    match std::fs::write(&path, state.export_markdown()) {
        Ok(()) => state.push_system(format!("Transcript exported to {}.", path.display())),
        Err(error) => state.push_system(format!("Could not export transcript: {error}")),
    }
}

pub(super) fn copy_latest_answer(state: &mut TranscriptState) {
    let Some(answer) = state.last_assistant().map(str::to_string) else {
        state.push_system("There is no completed answer to copy.");
        return;
    };
    // OSC 52 works over local terminals and SSH without taking a platform GUI
    // clipboard dependency. Terminals that disable it safely ignore the code.
    let encoded = base64::engine::general_purpose::STANDARD.encode(answer.as_bytes());
    let result = std::io::stdout()
        .write_all(format!("\x1b]52;c;{encoded}\x07").as_bytes())
        .and_then(|_| std::io::stdout().flush());
    state.push_system(if result.is_ok() {
        "Latest answer copied to the terminal clipboard."
    } else {
        "The terminal clipboard could not be updated."
    });
}
