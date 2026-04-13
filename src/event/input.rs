//! Application events that drive state transitions.
//!
//! [`AppEvent`] is the union of all things that can happen in the
//! application: keyboard input, terminal resizes, API streaming events,
//! and tool execution results. The update function pattern-matches on
//! these to decide how to transform the state.
//!
//! Events are produced by different sources:
//! - **Key/Resize**: from the terminal via crossterm (or from test fixtures)
//! - **ApiStreamStart/ApiTextDelta/ApiToolUse/ApiDone/ApiError**: from the
//!   chat backend as the model streams its response
//! - **ToolResult**: from the tool executor after the user approves a
//!   tool call and it finishes running
//! - **Tick**: from a periodic timer, used for animations like streaming
//!   indicators

use crossterm::event::KeyEvent;

use crate::state::message::{ToolName, ToolUseId};

/// An event that can change the application state.
///
/// The update function takes one of these and the current state, and
/// returns a new state plus any side effects to execute.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A keyboard event from the terminal. Contains the key code,
    /// modifiers (ctrl, alt, shift), and event kind (press, repeat, release).
    Key(KeyEvent),

    /// The terminal was resized to the given (columns, rows) dimensions.
    Resize(u16, u16),

    /// The API has started streaming a new response. The update function
    /// creates a [`MessageDraft`](crate::state::message::MessageDraft)
    /// to accumulate the incoming content.
    ApiStreamStart,

    /// A chunk of text arrived from the streaming API. This is appended
    /// to the current message draft.
    ApiTextDelta(String),

    /// The model emitted a tool-use request during streaming. This
    /// contains the full tool call (ID, name, and parsed JSON arguments).
    /// The update function adds this to the draft and transitions to
    /// [`ToolApproval`](crate::state::app::Mode::ToolApproval) mode.
    ApiToolUse {
        /// The unique ID for this tool call, from the API.
        id: ToolUseId,
        /// Which tool the model wants to invoke.
        name: ToolName,
        /// The JSON arguments for the tool.
        input: serde_json::Value,
    },

    /// The API stream has finished. The current message draft is finalized
    /// into an immutable message.
    ApiDone,

    /// The API stream encountered an error. The draft may be discarded
    /// and an error is shown in the status bar.
    ApiError(String),

    /// A tool finished executing (after the user approved it). The result
    /// is added to the conversation and the conversation is sent back to
    /// the API so the model can continue.
    ToolResult {
        /// The [`ToolUseId`] of the tool call that produced this result.
        tool_use_id: ToolUseId,
        /// The output from the tool.
        content: String,
        /// Whether the tool execution failed.
        is_error: bool,
    },

    /// A periodic tick for UI animations (e.g. streaming indicator).
    /// The update function can use this to cycle spinner frames or
    /// similar effects.
    Tick,
}
