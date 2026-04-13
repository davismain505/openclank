//! Top-level application state.
//!
//! [`AppState`] is the root of the entire state tree. It owns the
//! conversation history, the input buffer, UI mode, viewport dimensions,
//! and the status line. Every frame, the renderer reads `AppState` to
//! decide what to draw. Every event, the update function takes `AppState`
//! and returns a new one.
//!
//! The [`Mode`] enum controls which keyboard shortcuts are active and
//! how the UI is laid out. For example, in [`Mode::ToolApproval`] the
//! input area is replaced with a yes/no prompt for the pending tool call.

use super::message::{Conversation, ToolUseId};

/// Which interaction mode the application is currently in.
///
/// The mode determines which keybindings are active and how the UI is
/// rendered. Transitions between modes are handled by the update function
/// in response to events — never by the renderer or IO layer.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// The user is typing in the input area. This is the default mode.
    /// Arrow keys edit the input, Enter sends the message.
    Normal,

    /// The user is scrolling through message history. Arrow keys move
    /// the scroll position instead of editing input. Press Escape or
    /// 'i' to return to Normal mode.
    Scrolling,

    /// A tool-use request is pending approval. The UI shows the tool
    /// name and arguments, and the user must press 'y' to approve or
    /// 'n' to deny. The contained [`ToolUseId`] identifies which tool
    /// call is being reviewed.
    ToolApproval(ToolUseId),

    /// The application is shutting down. This mode is entered when the
    /// user presses the quit keybinding. The runner checks for this
    /// mode after each update and exits the event loop.
    Quitting,
}

/// What kind of information the status bar is showing.
///
/// This affects the visual styling of the status bar (e.g. errors
/// might render in red, streaming in a pulsing style).
#[derive(Debug, Clone, PartialEq)]
pub enum StatusKind {
    /// Neutral informational text (e.g. "Ready", model name).
    Info,
    /// An error message (e.g. API failure, tool execution error).
    Error,
    /// The model is currently streaming a response.
    Streaming,
}

/// The text and style of the bottom status bar.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusLine {
    /// The text displayed in the status bar.
    pub text: String,
    /// What kind of status this is, which controls styling.
    pub kind: StatusKind,
}

impl Default for StatusLine {
    fn default() -> Self {
        Self {
            text: "Ready".to_string(),
            kind: StatusKind::Info,
        }
    }
}

/// Terminal viewport dimensions in character cells.
///
/// Updated on resize events. The renderer uses this to lay out the UI,
/// and the update function uses it to calculate scroll bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct Viewport {
    /// Terminal width in columns.
    pub cols: u16,
    /// Terminal height in rows.
    pub rows: u16,
}

impl Default for Viewport {
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}

/// The complete application state at a point in time.
///
/// This is the single source of truth for the entire UI. The update
/// function produces a new `AppState` from the old one plus an event.
/// The renderer reads `AppState` to decide what to draw. Neither the
/// update function nor the renderer performs any IO — they are pure
/// functions of this state.
#[derive(Debug, Clone)]
pub struct AppState {
    /// The full conversation history plus any in-progress draft.
    pub conversation: Conversation,

    /// The text the user is currently typing in the input area.
    /// This is separate from the conversation — it only becomes a
    /// message when the user presses Enter.
    pub input_buffer: String,

    /// Byte offset of the cursor within `input_buffer`. This is a byte
    /// offset (not a character offset) because Rust strings are UTF-8
    /// and we need to handle multi-byte characters correctly.
    pub cursor_pos: usize,

    /// How many lines the chat view is scrolled up from the bottom.
    /// Zero means "show the latest messages" (auto-scroll). Any positive
    /// value means the user has scrolled up into history.
    pub scroll_offset: usize,

    /// The current interaction mode (normal input, scrolling, tool
    /// approval, or quitting).
    pub mode: Mode,

    /// The status bar content and style.
    pub status: StatusLine,

    /// Current terminal dimensions. Updated on resize events.
    pub viewport: Viewport,

    /// The model identifier to use for API requests (e.g.
    /// "claude-sonnet-4-20250514"). Displayed in the status bar and
    /// sent with each API request.
    pub model: String,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            conversation: Conversation::default(),
            input_buffer: String::new(),
            cursor_pos: 0,
            scroll_offset: 0,
            mode: Mode::Normal,
            status: StatusLine::default(),
            viewport: Viewport::default(),
            model: "claude-sonnet-4-20250514".to_string(),
        }
    }
}
