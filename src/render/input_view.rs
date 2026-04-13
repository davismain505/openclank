//! Input area renderer.
//!
//! Draws the text input box at the bottom of the screen where the user
//! types their messages. Shows the current input buffer contents and
//! positions the terminal cursor at the correct location so the blinking
//! cursor appears where the next character will be inserted.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::state::app::{AppState, Mode};

/// Render the input area into the given rectangle.
///
/// The input box has a border and a title that changes based on mode:
/// - Normal: "> " prompt, indicating the user can type
/// - Scrolling: "(scroll)" indicator so the user knows why typing
///   doesn't work
///
/// The cursor position is set so the terminal's native blinking cursor
/// appears at the right location within the input text.
pub fn render_input(frame: &mut Frame, area: Rect, state: &AppState) {
    let title = match state.mode {
        // In scrolling mode, replace the prompt with a hint about how
        // to get back to typing.
        Mode::Scrolling => " Input (scroll mode - press i to type) ",
        _ => " > ",
    };

    let style = match state.mode {
        // Dim the input in scrolling mode since it's not accepting input.
        // This is a load-bearing style choice — it signals to the user
        // that the input area is inactive.
        Mode::Scrolling => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };

    let block = Block::default().borders(Borders::ALL).title(title);
    let paragraph = Paragraph::new(state.input_buffer.as_str())
        .block(block)
        .style(style)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);

    // Position the terminal cursor inside the input box so the
    // blinking cursor appears where the next character will be inserted.
    // Only do this in Normal mode — in other modes, the cursor is hidden
    // or irrelevant.
    if state.mode == Mode::Normal {
        // For multi-line input, find which line and column the cursor is
        // on by splitting the text before the cursor on newlines.
        let text_before = &state.input_buffer[..state.cursor_pos];
        let lines: Vec<&str> = text_before.split('\n').collect();
        let cursor_row = lines.len() - 1;
        let cursor_col = lines.last().map(|l| l.chars().count()).unwrap_or(0);

        let x = area.x + 1 + cursor_col as u16; // +1 for left border
        let y = area.y + 1 + cursor_row as u16; // +1 for top border

        // Only set cursor if it's within the visible area, to avoid
        // panics from out-of-bounds cursor positions.
        if x < area.x + area.width && y < area.y + area.height {
            frame.set_cursor_position((x, y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Render the input view to a TestBackend and return the buffer
    /// as a string suitable for insta snapshots.
    fn render_to_string(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_input(frame, frame.area(), state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut result = String::new();
        for y in 0..height {
            for x in 0..width {
                result.push_str(buffer[(x, y)].symbol());
            }
            if y < height - 1 {
                result.push('\n');
            }
        }
        result
    }

    #[test]
    fn empty_input_normal_mode() {
        let state = AppState::default();
        let output = render_to_string(&state, 30, 3);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn input_with_text() {
        let mut state = AppState::default();
        state.input_buffer = "hello world".to_string();
        state.cursor_pos = 11;
        let output = render_to_string(&state, 30, 3);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn scrolling_mode_title() {
        let mut state = AppState::default();
        state.mode = Mode::Scrolling;
        let output = render_to_string(&state, 50, 3);
        insta::assert_snapshot!(output);
    }

    /// Style spot-check: in scrolling mode, the input text should be
    /// rendered in DarkGray to signal that input is inactive. Insta
    /// snapshots can't capture colors, so we assert directly on the
    /// buffer cell's foreground color.
    #[test]
    fn scrolling_mode_dims_text_to_dark_gray() {
        let mut state = AppState::default();
        state.mode = Mode::Scrolling;
        state.input_buffer = "some text".to_string();

        let backend = TestBackend::new(40, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_input(frame, frame.area(), &state);
            })
            .unwrap();

        // Row 1, col 1 is inside the border where text content starts.
        let cell = &terminal.backend().buffer()[(1, 1)];
        assert_eq!(
            cell.fg,
            Color::DarkGray,
            "input text should be DarkGray in scroll mode, got {:?}",
            cell.fg
        );
    }
}
