//! Terminal setup / teardown with panic-safe restoration.
//!
//! Owning the terminal means switching to the alternate screen and enabling raw
//! mode; both **must** be undone on every exit path — normal return, `?`
//! propagation, and panic — or the user's shell is left in a broken state
//! (no echo, no line editing, stuck on the alternate screen). [`TerminalGuard`]
//! restores on `Drop`, and [`install_panic_hook`] chains a restore ahead of the
//! previous panic hook so the panic message prints to a sane terminal.

use std::io::{self, Stdout};

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

/// The concrete ratatui terminal type used by the TUI.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// RAII guard that enters the alternate screen + raw mode on construction and
/// restores the terminal on drop.
pub struct TerminalGuard {
    terminal: Tui,
}

impl TerminalGuard {
    /// Enter the alternate screen, enable raw mode, install the panic hook, and
    /// return a ready-to-draw terminal wrapped in a restoring guard.
    pub fn enter() -> io::Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)?;
        log::debug!("[tui] terminal: entered alternate screen + raw mode");
        Ok(Self { terminal })
    }

    /// Mutable access to the underlying terminal for drawing.
    pub fn terminal(&mut self) -> &mut Tui {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if let Err(e) = restore() {
            // The subscriber writes to a file (never the terminal), so this is
            // safe to log here.
            log::warn!("[tui] terminal: restore on drop failed: {e}");
        } else {
            log::debug!("[tui] terminal: restored on drop");
        }
    }
}

/// Undo everything [`TerminalGuard::enter`] did. Best-effort — each step is
/// attempted even if an earlier one fails, so a partial setup still gets torn
/// down as far as possible.
fn restore() -> io::Result<()> {
    let mut stdout = io::stdout();
    let _ = execute!(
        stdout,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    disable_raw_mode()
}

/// Chain a terminal-restoring step in front of the process panic hook, so a
/// panic inside the render loop leaves the user with a usable terminal and a
/// readable backtrace instead of a garbled alternate screen.
///
/// Idempotent in effect: called once from [`TerminalGuard::enter`]. If it were
/// ever called twice, the second restore would simply be a no-op on an
/// already-restored terminal.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        original(info);
    }));
}
