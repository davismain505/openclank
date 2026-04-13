//! Top-level layout compositor.
//!
//! [`render_app`] is the single entry point called by the runner on every
//! frame. It divides the terminal into three vertical regions:
//!
//! ```text
//! ┌─────────────────────────────┐
//! │         Chat history        │  (fills remaining space)
//! │                             │
//! ├─────────────────────────────┤
//! │  Input area (3 rows min)    │  (fixed height)
//! ├─────────────────────────────┤
//! │  Status bar (1 row)         │  (fixed height)
//! └─────────────────────────────┘
//! ```
//!
//! In [`Mode::ToolApproval`], the input area is replaced with the tool
//! approval prompt showing what the model wants to execute.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::state::app::{AppState, Mode};

use super::chat_view::render_chat;
use super::input_view::render_input;
use super::status_bar::render_status;
use super::tool_view::render_tool_approval;

/// The fixed height of the input area in rows.
const INPUT_HEIGHT: u16 = 3;

/// The fixed height of the status bar in rows.
const STATUS_HEIGHT: u16 = 1;

/// Render the entire application UI for one frame.
///
/// This is the only render function the runner needs to call. It handles
/// layout computation and delegates to the specialized renderers for
/// each region.
pub fn render_app(frame: &mut Frame, state: &AppState) {
    let area = frame.area();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),                  // chat history
            Constraint::Length(INPUT_HEIGHT),     // input or tool approval
            Constraint::Length(STATUS_HEIGHT),    // status bar
        ])
        .split(area);

    let chat_area = layout[0];
    let input_area = layout[1];
    let status_area = layout[2];

    render_chat(frame, chat_area, state);

    // In ToolApproval mode, replace the input area with the approval
    // prompt. Otherwise, show the normal text input.
    match &state.mode {
        Mode::ToolApproval(tool_id) => {
            render_tool_approval(frame, input_area, state, tool_id);
        }
        _ => {
            render_input(frame, input_area, state);
        }
    }

    render_status(frame, status_area, state);
}
