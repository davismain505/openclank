//! Crossterm terminal setup and teardown.
//!
//! The TUI takes over the terminal by entering raw mode (so we
//! receive individual key events instead of line-buffered input)
//! and the alternate screen (so the chat history doesn't leave
//! scrollback in the user's shell).
//!
//! The [`TerminalGuard`] RAII wrapper ensures we always clean up
//! on drop — including panic-unwind — so the user's shell is
//! never left in raw mode after openclank exits.

use std::io::{self, Stdout};

use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// A ratatui terminal tied to stdout, with raw mode enabled.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// RAII guard that restores the terminal to its pre-TUI state
/// when dropped.
///
/// Entering raw mode and the alternate screen are terminal-wide
/// side effects. If the program panics or returns without resetting
/// them, the user's shell is left in a broken state (no line
/// editing, no echo, display overwritten by our TUI). The drop
/// impl handles both normal exit and panic unwind.
pub struct TerminalGuard {
    /// The ratatui terminal. `None` after `leave()` or drop so we
    /// don't double-leave.
    terminal: Option<Tui>,
}

impl TerminalGuard {
    /// Enter raw mode and the alternate screen, returning a guard
    /// that will tear everything down on drop.
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal: Some(terminal),
        })
    }

    /// Access the underlying ratatui terminal for rendering.
    pub fn terminal(&mut self) -> &mut Tui {
        self.terminal
            .as_mut()
            .expect("terminal accessed after teardown")
    }

    /// Explicit teardown. Called by drop but can be invoked
    /// manually for error reporting before the guard falls out
    /// of scope.
    pub fn leave(&mut self) {
        if self.terminal.take().is_some() {
            // Best effort — we're probably shutting down anyway.
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.leave();
    }
}
