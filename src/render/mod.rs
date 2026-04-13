//! Rendering functions that draw the TUI from application state.
//!
//! Every function in this module is a pure transformation: it reads
//! [`AppState`](crate::state::app::AppState) and draws to a ratatui
//! [`Frame`]. There is no IO, no mutation of application state, and no
//! side effects. This makes every render function testable via ratatui's
//! `TestBackend` — construct a state, render it, and assert on the
//! buffer contents.
//!
//! The top-level entry point is [`layout::render_app`], which divides
//! the terminal into three regions (chat history, input area, status
//! bar) and delegates to the specialized render functions for each.

pub mod chat_view;
pub mod input_view;
pub mod layout;
pub mod status_bar;
pub mod tool_view;

use crate::state::message::ToolName;

/// Format a tool's input arguments as a one-line display string.
///
/// Used by both the chat history view and the tool approval prompt to
/// show a human-readable summary of what a tool call will do. The
/// format depends on the tool type — bash shows the command, file
/// tools show the path with an operation suffix, search tools show
/// the pattern.
///
/// This is the single source of truth for tool input display. Both
/// `chat_view` and `tool_view` call this function to avoid divergent
/// formatting.
pub fn format_tool_input(tool: ToolName, input: &serde_json::Value) -> String {
    match tool {
        // bash: show the command string, which is the whole point.
        ToolName::Bash => input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),

        // read_file: show the file path being read.
        ToolName::ReadFile => input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),

        // write_file: show the path with a "(write)" suffix so it's
        // clear this is a destructive operation.
        ToolName::WriteFile => {
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{path} (write)")
        }

        // edit_file: show the path with an "(edit)" suffix.
        ToolName::EditFile => {
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{path} (edit)")
        }

        // glob: show the search pattern.
        ToolName::Glob => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),

        // grep: show the search pattern.
        ToolName::Grep => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

#[cfg(test)]
mod format_tool_input_tests {
    use super::*;

    #[test]
    fn bash_shows_command() {
        let input = serde_json::json!({"command": "ls -la"});
        assert_eq!(format_tool_input(ToolName::Bash, &input), "ls -la");
    }

    #[test]
    fn bash_missing_command_returns_empty() {
        let input = serde_json::json!({});
        assert_eq!(format_tool_input(ToolName::Bash, &input), "");
    }

    #[test]
    fn read_file_shows_path() {
        let input = serde_json::json!({"path": "/etc/passwd"});
        assert_eq!(format_tool_input(ToolName::ReadFile, &input), "/etc/passwd");
    }

    #[test]
    fn write_file_shows_path_with_suffix() {
        let input = serde_json::json!({"path": "/tmp/out.txt"});
        assert_eq!(
            format_tool_input(ToolName::WriteFile, &input),
            "/tmp/out.txt (write)"
        );
    }

    #[test]
    fn edit_file_shows_path_with_suffix() {
        let input = serde_json::json!({"path": "src/main.rs"});
        assert_eq!(
            format_tool_input(ToolName::EditFile, &input),
            "src/main.rs (edit)"
        );
    }

    #[test]
    fn glob_shows_pattern() {
        let input = serde_json::json!({"pattern": "**/*.rs"});
        assert_eq!(format_tool_input(ToolName::Glob, &input), "**/*.rs");
    }

    #[test]
    fn grep_shows_pattern() {
        let input = serde_json::json!({"pattern": "TODO"});
        assert_eq!(format_tool_input(ToolName::Grep, &input), "TODO");
    }
}
