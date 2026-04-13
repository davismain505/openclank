//! Tool approval prompt renderer.
//!
//! When the model requests a tool call, the input area is replaced with
//! an approval prompt showing what the model wants to execute. The user
//! must press 'y' to approve or 'n' to deny. This module renders that
//! prompt.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::app::AppState;
use crate::state::message::{ContentBlock, ToolUseId};

/// Render the tool approval prompt into the area normally used for input.
///
/// Shows the tool name and a compact summary of its arguments, with a
/// "(y/n)" prompt. The tool name is highlighted in yellow to draw
/// attention to what's being requested.
pub fn render_tool_approval(frame: &mut Frame, area: Rect, state: &AppState, tool_id: &ToolUseId) {
    // Find the tool call in the conversation to get its name and input.
    let tool_info = find_tool_info(state, tool_id);

    let line = match tool_info {
        // Found the tool call — show its name and a summary of what
        // it wants to do, with a clear y/n prompt.
        Some((name, summary)) => Line::from(vec![
            Span::styled(" Approve ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("[{name}]"),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {summary} ")),
            Span::styled("(y/n)", Style::default().fg(Color::DarkGray)),
        ]),

        // Shouldn't happen — the tool ID should always exist in the
        // conversation when we're in ToolApproval mode. Show a fallback
        // so the user can still approve or deny.
        None => Line::from(vec![
            Span::styled(
                " Approve tool call? ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled("(y/n)", Style::default().fg(Color::DarkGray)),
        ]),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Tool Approval ");
    let paragraph = Paragraph::new(line).block(block);

    frame.render_widget(paragraph, area);
}

/// Search the conversation for the tool call matching `tool_id` and
/// return its display name and a one-line summary of its arguments.
fn find_tool_info(state: &AppState, tool_id: &ToolUseId) -> Option<(String, String)> {
    for msg in state.conversation.messages().iter().rev() {
        for block in msg.content() {
            let ContentBlock::ToolUse { id, name, input } = block else {
                continue;
            };
            if id != tool_id {
                continue;
            }

            // Use the shared format_tool_input function so the approval
            // prompt shows the same summary as the chat history, including
            // "(write)"/"(edit)" suffixes for destructive operations.
            let summary = super::format_tool_input(*name, input);

            return Some((name.to_string(), summary));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::test_support::render_to_string;
    use crate::state::app::Mode;
    use crate::state::message::{ContentBlock, Message, ToolName, ToolUseId};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn state_with_tool_approval(
        tool_id: &ToolUseId,
        name: ToolName,
        input: serde_json::Value,
    ) -> AppState {
        let mut state = AppState::default();

        let msg = Message::from_content_blocks(
            crate::state::message::Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: tool_id.clone(),
                name,
                input,
            }],
        );
        state.conversation.push(msg);
        state.pending_tools.insert(tool_id.clone());
        state.mode = Mode::ToolApproval;
        state
    }

    #[test]
    fn bash_tool_approval() {
        let tool_id = ToolUseId::new("toolu_test001").unwrap();
        let state = state_with_tool_approval(
            &tool_id,
            ToolName::Bash,
            serde_json::json!({"command": "ls -la /tmp"}),
        );
        let output = render_to_string(60, 3, |frame, area| {
            render_tool_approval(frame, area, &state, &tool_id);
        });
        insta::assert_snapshot!(output);
    }

    #[test]
    fn read_file_tool_approval() {
        let tool_id = ToolUseId::new("toolu_test002").unwrap();
        let state = state_with_tool_approval(
            &tool_id,
            ToolName::ReadFile,
            serde_json::json!({"path": "/etc/passwd"}),
        );
        let output = render_to_string(60, 3, |frame, area| {
            render_tool_approval(frame, area, &state, &tool_id);
        });
        insta::assert_snapshot!(output);
    }

    /// Style spot-check: the tool name in the approval prompt should
    /// be yellow to draw attention.
    #[test]
    fn tool_name_is_yellow() {
        let tool_id = ToolUseId::new("toolu_test003").unwrap();
        let state = state_with_tool_approval(
            &tool_id,
            ToolName::Bash,
            serde_json::json!({"command": "echo hi"}),
        );

        let backend = TestBackend::new(60, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_tool_approval(frame, frame.area(), &state, &tool_id);
            })
            .unwrap();

        // Find the '[' character that starts "[bash]" — it should be
        // in row 1 (inside border) after the "Approve " text.
        let buffer = terminal.backend().buffer();
        let mut found_yellow = false;
        for x in 0..60 {
            let cell = &buffer[(x, 1)];
            if cell.symbol() == "[" && cell.fg == Color::Yellow {
                found_yellow = true;
                break;
            }
        }
        assert!(found_yellow, "tool name bracket should be Yellow");
    }
}
