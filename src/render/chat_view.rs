//! Chat history renderer.
//!
//! Draws the scrollable message history in the main area of the screen.
//! Each message is rendered with a role prefix ("You:" or "Claude:") and
//! the message text. Tool-use and tool-result blocks are rendered inline
//! with distinct formatting.
//!
//! The view auto-scrolls to the bottom (latest messages) unless the user
//! has scrolled up. The [`AppState::scroll_offset`] field controls how
//! many lines are scrolled up from the bottom.
//!
//! During streaming, the in-progress [`MessageDraft`] is rendered below
//! the finalized messages with the same formatting.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::state::app::AppState;
use crate::state::message::{ContentBlock, Message, Role};

/// Render the chat history into the given area.
///
/// Messages are converted into styled lines, then displayed in a
/// scrollable paragraph widget. The scroll position is derived from
/// `state.scroll_offset`.
pub fn render_chat(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut lines: Vec<Line> = Vec::new();

    // Render each finalized message.
    for msg in state.conversation.messages() {
        render_message_lines(&mut lines, msg);
        lines.push(Line::default()); // blank line between messages
    }

    // Render the in-progress draft if one exists. The draft shows
    // partial content as it streams in, giving the user real-time
    // feedback that the model is responding.
    if let Some(draft) = state.conversation.draft() {
        let prefix = Span::styled(
            "Claude: ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );
        let text = draft.text();
        if text.is_empty() {
            // Draft exists but no text yet — show a placeholder so the
            // user knows the model is working.
            lines.push(Line::from(vec![
                prefix,
                Span::styled("...", Style::default().fg(Color::DarkGray)),
            ]));
        } else {
            // Render the draft text with the same formatting as a
            // finalized assistant message.
            let text_lines: Vec<&str> = text.lines().collect();
            for (i, text_line) in text_lines.iter().enumerate() {
                if i == 0 {
                    lines.push(Line::from(vec![
                        prefix.clone(),
                        Span::raw(text_line.to_string()),
                    ]));
                } else {
                    lines.push(Line::from(format!("         {text_line}")));
                }
            }
        }
    }

    // Compute scroll position. scroll_offset is "lines from the bottom"
    // (0 = show latest). Ratatui's Paragraph::scroll takes "lines from
    // the top", so we convert: top_scroll = max_scroll - clamped_offset.
    //
    // All arithmetic is done in usize to avoid truncation bugs (the
    // scroll_offset can be usize::MAX from the 'g' key). The final
    // cast to u16 happens after clamping to a value that fits.
    let total_lines = lines.len();
    let visible_height = area.height.saturating_sub(2) as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let clamped_offset = state.scroll_offset.min(max_scroll);
    let scroll_from_top = max_scroll.saturating_sub(clamped_offset) as u16;

    let block = Block::default().borders(Borders::ALL).title(" Chat ");
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_from_top, 0));

    frame.render_widget(paragraph, area);
}

/// Convert a single finalized message into styled lines and append them
/// to the output vector. Each content block in the message is rendered
/// in sequence, with the role prefix ("You:" / "Claude:") on the first
/// line of the first block only.
fn render_message_lines(lines: &mut Vec<Line>, msg: &Message) {
    let (prefix, prefix_style) = match msg.role() {
        Role::User => (
            "You: ",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Role::Assistant => (
            "Claude: ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    };

    // Track whether we've rendered any block yet, so we know whether
    // the next block's first line needs the role prefix or just indentation.
    let mut first_block = true;

    for block in msg.content() {
        match block {
            // Plain text: the most common block type. The first line of
            // the first text block gets the role prefix ("You: " or
            // "Claude: "). Subsequent lines are indented to align with
            // the text after the prefix.
            ContentBlock::Text(text) => {
                let text_lines: Vec<&str> = text.lines().collect();
                for (i, text_line) in text_lines.iter().enumerate() {
                    if first_block && i == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(prefix, prefix_style),
                            Span::raw(text_line.to_string()),
                        ]));
                    } else {
                        let indent = " ".repeat(prefix.len());
                        lines.push(Line::from(format!("{indent}{text_line}")));
                    }
                }
                first_block = false;
            }

            // Tool-use request: the model is asking to run a tool. We
            // show the tool name in yellow brackets, followed by a
            // compact summary of the tool's arguments (e.g. the shell
            // command for bash, the file path for read_file). This gives
            // the user enough context to understand what the model wanted
            // to do, even after the tool has already been approved.
            ContentBlock::ToolUse { name, input, .. } => {
                let indent = " ".repeat(prefix.len());
                if first_block {
                    lines.push(Line::from(vec![
                        Span::styled(prefix, prefix_style),
                        Span::styled(
                            format!("[tool: {name}]"),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::raw(indent.clone()),
                        Span::styled(
                            format!("[tool: {name}]"),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]));
                }

                // Show the most relevant field from the tool's arguments,
                // rendered in dim gray so it doesn't compete with the
                // main conversation text.
                let input_str = super::format_tool_input(*name, input);
                for input_line in input_str.lines() {
                    lines.push(Line::from(vec![
                        Span::raw(indent.clone()),
                        Span::styled(
                            format!("  {input_line}"),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
                first_block = false;
            }

            // Tool result: the output from a tool that was executed. We
            // show a label ("[tool result]" or "[tool error]") followed
            // by a truncated preview of the output. Errors are red,
            // successful results are dim gray. The preview is capped at
            // 5 lines to keep the chat view readable when tools produce
            // large outputs (like file contents or long command output).
            ContentBlock::ToolResult {
                content, is_error, ..
            } => {
                let style = if *is_error {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let label = if *is_error {
                    "[tool error]"
                } else {
                    "[tool result]"
                };

                if first_block {
                    lines.push(Line::from(vec![
                        Span::styled(prefix, prefix_style),
                        Span::styled(label, style),
                    ]));
                } else {
                    let indent = " ".repeat(prefix.len());
                    lines.push(Line::from(vec![
                        Span::raw(indent),
                        Span::styled(label, style),
                    ]));
                }

                let indent = " ".repeat(prefix.len());
                let preview = truncate_lines(content, 5);
                for result_line in preview.lines() {
                    lines.push(Line::from(vec![
                        Span::raw(indent.clone()),
                        Span::styled(format!("  {result_line}"), style),
                    ]));
                }
                first_block = false;
            }
        }
    }
}



/// Truncate a multi-line string to at most `max_lines` lines. If
/// truncated, appends a "... (N more lines)" indicator so the user
/// knows output was clipped.
fn truncate_lines(text: &str, max_lines: usize) -> String {
    let all_lines: Vec<&str> = text.lines().collect();
    if all_lines.len() <= max_lines {
        return text.to_string();
    }
    let shown: Vec<&str> = all_lines[..max_lines].to_vec();
    let remaining = all_lines.len() - max_lines;
    format!("{}\n... ({remaining} more lines)", shown.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::app::AppState;
    use crate::state::message::{ContentBlock, Message, Role, ToolName, ToolUseId};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Render the chat view to a TestBackend and return the buffer
    /// as a string suitable for insta snapshots.
    fn render_to_string(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_chat(frame, frame.area(), state);
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
    fn empty_chat() {
        let state = AppState::default();
        let output = render_to_string(&state, 40, 10);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn single_user_message() {
        let mut state = AppState::default();
        state.conversation.push(Message::user("hello"));
        let output = render_to_string(&state, 40, 10);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn user_and_assistant_messages() {
        let mut state = AppState::default();
        state.conversation.push(Message::user("hello"));
        state.conversation.push(Message::assistant("Hi there! How can I help?"));
        let output = render_to_string(&state, 40, 10);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn streaming_draft_with_text() {
        let mut state = AppState::default();
        state.conversation.push(Message::user("hello"));
        state.conversation.start_draft();
        state.conversation.draft_mut().unwrap().append_text("Working on it");
        let output = render_to_string(&state, 40, 10);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn streaming_draft_empty_placeholder() {
        let mut state = AppState::default();
        state.conversation.push(Message::user("hello"));
        state.conversation.start_draft();
        let output = render_to_string(&state, 40, 10);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn message_with_tool_use_block() {
        let mut state = AppState::default();
        state.conversation.push(Message::user("list files"));
        state.conversation.push(Message::from_content_blocks(
            Role::Assistant,
            vec![
                ContentBlock::Text("Let me check.".to_string()),
                ContentBlock::ToolUse {
                    id: ToolUseId::new("toolu_test001").unwrap(),
                    name: ToolName::Bash,
                    input: serde_json::json!({"command": "ls -la"}),
                },
            ],
        ));
        let output = render_to_string(&state, 50, 12);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn message_with_tool_result() {
        let mut state = AppState::default();
        state.conversation.push(Message::from_tool_result(
            ToolUseId::new("toolu_test001").unwrap(),
            "file1.txt\nfile2.txt\nfile3.txt".to_string(),
            false,
        ));
        let output = render_to_string(&state, 50, 10);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn tool_error_result_is_distinct() {
        let mut state = AppState::default();
        state.conversation.push(Message::from_tool_result(
            ToolUseId::new("toolu_test001").unwrap(),
            "command not found: foobar".to_string(),
            true,
        ));
        let output = render_to_string(&state, 50, 10);
        insta::assert_snapshot!(output);
    }

    /// Style spot-check: user prefix "You: " should be blue+bold.
    #[test]
    fn user_prefix_is_blue_bold() {
        let mut state = AppState::default();
        state.conversation.push(Message::user("hi"));

        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_chat(frame, frame.area(), &state);
            })
            .unwrap();

        // Row 1, col 1 is inside the border where "Y" of "You: " starts.
        let cell = &terminal.backend().buffer()[(1, 1)];
        assert_eq!(cell.fg, Color::Blue, "user prefix should be Blue");
        assert!(
            cell.modifier.contains(Modifier::BOLD),
            "user prefix should be Bold"
        );
    }

    /// Style spot-check: assistant prefix "Claude: " should be green+bold.
    #[test]
    fn assistant_prefix_is_green_bold() {
        let mut state = AppState::default();
        state.conversation.push(Message::assistant("hi"));

        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_chat(frame, frame.area(), &state);
            })
            .unwrap();

        let cell = &terminal.backend().buffer()[(1, 1)];
        assert_eq!(cell.fg, Color::Green, "assistant prefix should be Green");
        assert!(
            cell.modifier.contains(Modifier::BOLD),
            "assistant prefix should be Bold"
        );
    }
}
