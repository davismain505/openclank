//! Status bar renderer.
//!
//! Draws a single-row status bar at the bottom of the screen. The bar
//! shows the current status message (e.g. "Ready", "Streaming...",
//! "Error: ...") with color-coded styling based on the status kind.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::app::{AppState, StatusKind};

/// Render the status bar into the given single-row area.
///
/// The bar shows the model name on the left and the status message.
/// Colors indicate the status type:
/// - Info: default/white text
/// - Error: red text
/// - Streaming: cyan text to indicate activity
pub fn render_status(frame: &mut Frame, area: Rect, state: &AppState) {
    let status_style = match state.status.kind {
        // Neutral informational text — default terminal color.
        StatusKind::Info => Style::default(),
        // Error messages are red so they stand out immediately.
        StatusKind::Error => Style::default().fg(Color::Red),
        // Streaming indicator is cyan to show active work without
        // being as alarming as red.
        StatusKind::Streaming => Style::default().fg(Color::Cyan),
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", state.model),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(&state.status.text, status_style),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::test_support::render_to_string;
    use crate::state::app::StatusLine;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn default_status_shows_ready() {
        let state = AppState::default();
        let output = render_to_string(60, 1, |frame, area| render_status(frame, area, &state));
        insta::assert_snapshot!(output);
    }

    #[test]
    fn streaming_status() {
        let mut state = AppState::default();
        state.status = StatusLine {
            text: "Streaming...".to_string(),
            kind: StatusKind::Streaming,
        };
        let output = render_to_string(60, 1, |frame, area| render_status(frame, area, &state));
        insta::assert_snapshot!(output);
    }

    #[test]
    fn error_status() {
        let mut state = AppState::default();
        state.status = StatusLine {
            text: "Error: rate limited".to_string(),
            kind: StatusKind::Error,
        };
        let output = render_to_string(60, 1, |frame, area| render_status(frame, area, &state));
        insta::assert_snapshot!(output);
    }

    /// Style spot-check: error status text should be red.
    #[test]
    fn error_status_is_red() {
        let mut state = AppState::default();
        state.status = StatusLine {
            text: "Error: bad".to_string(),
            kind: StatusKind::Error,
        };

        let backend = TestBackend::new(60, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_status(frame, frame.area(), &state);
            })
            .unwrap();

        // Find the first cell that contains 'E' from "Error:" — it
        // should be styled red.
        let buffer = terminal.backend().buffer();
        let model_len = state.model.len() + 2; // " model " with spaces
        let error_cell = &buffer[(model_len as u16, 0)];
        assert_eq!(
            error_cell.fg,
            Color::Red,
            "error text should be Red, got {:?}",
            error_cell.fg
        );
    }
}
