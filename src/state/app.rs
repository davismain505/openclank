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

use std::collections::HashSet;

use super::message::{Conversation, ToolUseId};

/// A text input buffer with a cursor that maintains UTF-8 char boundary
/// invariants by construction.
///
/// The cursor position is always a valid UTF-8 char boundary within the
/// buffer (or at the end). This is enforced by making the cursor field
/// private and only allowing mutation through methods that preserve the
/// invariant. External code can read the cursor position and text but
/// cannot set them to arbitrary values.
#[derive(Debug, Clone)]
pub struct InputBuffer {
    /// The text content of the buffer.
    text: String,
    /// Byte offset of the cursor. Always satisfies
    /// `text.is_char_boundary(cursor)` and `cursor <= text.len()`.
    cursor: usize,
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }
}

impl InputBuffer {
    /// The text content of the buffer.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cursor position as a byte offset into the text.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether the buffer contains only whitespace (or is empty).
    /// Used to decide whether Enter should send a message.
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Return the trimmed text content and reset the buffer to empty.
    /// Used when the user presses Enter to send a message.
    pub fn take_trimmed(&mut self) -> String {
        let text = self.text.trim().to_string();
        self.text.clear();
        self.cursor = 0;
        text
    }

    /// Insert a character at the cursor position and advance the cursor
    /// past it.
    pub fn insert_char(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Delete the character before the cursor (backspace). Does nothing
    /// if the cursor is at the start.
    pub fn delete_back(&mut self) {
        if let Some(prev) = self.prev_boundary() {
            self.text.drain(prev..self.cursor);
            self.cursor = prev;
        }
    }

    /// Delete the character after the cursor (forward delete). Does
    /// nothing if the cursor is at the end.
    pub fn delete_forward(&mut self) {
        if self.cursor < self.text.len() {
            let next = self.next_boundary();
            self.text.drain(self.cursor..next);
        }
    }

    /// Move the cursor left one character. Does nothing if already at
    /// the start.
    pub fn move_left(&mut self) {
        if let Some(prev) = self.prev_boundary() {
            self.cursor = prev;
        }
    }

    /// Move the cursor right one character. Does nothing if already at
    /// the end.
    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor = self.next_boundary();
        }
    }

    /// Move the cursor to the start of the buffer.
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Move the cursor to the end of the buffer.
    pub fn move_end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Find the byte offset of the previous char boundary before the
    /// cursor. Returns `None` if the cursor is at position 0.
    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
    }

    /// Find the byte offset of the next char boundary after the cursor.
    /// Returns `text.len()` if the cursor is at the last character.
    fn next_boundary(&self) -> usize {
        self.text[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.text.len())
    }
}

#[cfg(test)]
mod input_buffer_tests {
    use super::InputBuffer;

    /// Helper: create an InputBuffer with the given text and cursor at the end.
    fn buf(text: &str) -> InputBuffer {
        let mut b = InputBuffer::default();
        for c in text.chars() {
            b.insert_char(c);
        }
        b
    }

    // ─── insert_char ─────────────────────────────────────────────

    #[test]
    fn insert_ascii() {
        let mut b = InputBuffer::default();
        b.insert_char('h');
        b.insert_char('i');
        assert_eq!(b.text(), "hi");
        assert_eq!(b.cursor(), 2);
    }

    #[test]
    fn insert_multibyte() {
        let mut b = InputBuffer::default();
        b.insert_char('é'); // 2 bytes
        b.insert_char('日'); // 3 bytes
        assert_eq!(b.text(), "é日");
        assert_eq!(b.cursor(), 5);
    }

    #[test]
    fn insert_in_middle() {
        let mut b = buf("ac");
        b.move_left(); // cursor before 'c'
        b.insert_char('b');
        assert_eq!(b.text(), "abc");
    }

    // ─── delete_back ─────────────────────────────────────────────

    #[test]
    fn delete_back_ascii() {
        let mut b = buf("hi");
        b.delete_back();
        assert_eq!(b.text(), "h");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn delete_back_multibyte() {
        let mut b = buf("aé");
        b.delete_back();
        assert_eq!(b.text(), "a");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn delete_back_at_start_does_nothing() {
        let mut b = InputBuffer::default();
        b.delete_back();
        assert_eq!(b.text(), "");
        assert_eq!(b.cursor(), 0);
    }

    // ─── delete_forward ──────────────────────────────────────────

    #[test]
    fn delete_forward_removes_next_char() {
        let mut b = buf("hello");
        b.move_home();
        b.move_right(); // cursor after 'h'
        b.delete_forward(); // deletes 'e'
        assert_eq!(b.text(), "hllo");
    }

    #[test]
    fn delete_forward_at_end_does_nothing() {
        let mut b = buf("hi");
        b.delete_forward();
        assert_eq!(b.text(), "hi");
    }

    #[test]
    fn delete_forward_multibyte() {
        let mut b = buf("é日x");
        b.move_home(); // cursor at 0
        b.delete_forward(); // deletes 'é' (2 bytes)
        assert_eq!(b.text(), "日x");
        assert_eq!(b.cursor(), 0);
    }

    // ─── movement ────────────────────────────────────────────────

    #[test]
    fn move_left_and_right() {
        let mut b = buf("abc");
        assert_eq!(b.cursor(), 3);
        b.move_left();
        assert_eq!(b.cursor(), 2);
        b.move_left();
        assert_eq!(b.cursor(), 1);
        b.move_right();
        assert_eq!(b.cursor(), 2);
    }

    #[test]
    fn move_left_at_start_does_nothing() {
        let mut b = InputBuffer::default();
        b.move_left();
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn move_right_at_end_does_nothing() {
        let mut b = buf("x");
        b.move_right();
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn move_left_over_multibyte() {
        let mut b = buf("aé"); // 'a'=1 byte, 'é'=2 bytes, cursor at 3
        b.move_left(); // should land at 1, not 2
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn home_and_end() {
        let mut b = buf("hello");
        b.move_home();
        assert_eq!(b.cursor(), 0);
        b.move_end();
        assert_eq!(b.cursor(), 5);
    }

    // ─── take_trimmed / is_blank ─────────────────────────────────

    #[test]
    fn take_trimmed_returns_trimmed_and_clears() {
        let mut b = buf("  hello  ");
        let text = b.take_trimmed();
        assert_eq!(text, "hello");
        assert_eq!(b.text(), "");
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn is_blank_on_empty() {
        let b = InputBuffer::default();
        assert!(b.is_blank());
    }

    #[test]
    fn is_blank_on_whitespace_only() {
        let mut b = InputBuffer::default();
        b.insert_char(' ');
        b.insert_char(' ');
        assert!(b.is_blank());
    }

    #[test]
    fn is_blank_false_with_content() {
        let b = buf("hi");
        assert!(!b.is_blank());
    }

    // ─── newline insertion ───────────────────────────────────────

    #[test]
    fn insert_newline() {
        let mut b = buf("line1");
        b.insert_char('\n');
        assert_eq!(b.cursor(), 6); // "line1\n" is 6 bytes
        for c in "line2".chars() {
            b.insert_char(c);
        }
        assert_eq!(b.text(), "line1\nline2");
    }
}

/// Which interaction mode the application is currently in.
///
/// The mode determines which keybindings are active and how the UI is
/// rendered. Transitions between modes are handled by the update function
/// in response to events — never by the renderer or IO layer.
///
/// ## State transitions
///
/// ```text
/// Normal ──[Ctrl+C/D]──────▶ Quitting
/// Normal ──[PageUp]─────────▶ Scrolling
/// Normal ──[ApiDone w/tools]▶ ToolApproval
///
/// Scrolling ──[Esc/i/G/j=0]──▶ Normal
///
/// ToolApproval ──[y/n per tool]──▶ ToolApproval (more pending)
/// ToolApproval ──[last tool resolved]──▶ Executing
/// ToolApproval ──[Ctrl+C]────▶ Quitting
///
/// Executing ──[all results in]──▶ Normal (sends results to API)
///
/// Quitting ──(terminal, all events ignored)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// The user is typing in the input area. This is the default mode.
    /// Arrow keys edit the input, Enter sends the message.
    Normal,

    /// The user is scrolling through message history. Arrow keys move
    /// the scroll position instead of editing input. Press Escape or
    /// 'i' to return to Normal mode.
    Scrolling,

    /// Tool-use requests are pending approval. The UI shows the current
    /// tool's name and arguments, and the user must press 'y' to
    /// approve or 'n' to deny. The tool being shown is the first entry
    /// in `AppState::pending_tools`. When the set empties, the mode
    /// transitions to `Executing`.
    ToolApproval,

    /// Approved tools are running. The runner is executing subprocesses
    /// or file operations for each approved tool. When all results are
    /// collected (and combined with denial results), they are sent back
    /// to the API in a single user message and the mode returns to
    /// Normal (which triggers a new streaming response).
    Executing,

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

    /// The text the user is currently typing in the input area, with
    /// cursor position. This is separate from the conversation — it
    /// only becomes a message when the user presses Enter. The
    /// [`InputBuffer`] type maintains UTF-8 char boundary invariants
    /// by construction.
    pub input: InputBuffer,

    /// How many lines the chat view is scrolled up from the bottom.
    /// Zero means "show the latest messages" (auto-scroll). Any positive
    /// value means the user has scrolled up into history.
    pub scroll_offset: usize,

    /// The current interaction mode (normal input, scrolling, tool
    /// approval, executing, or quitting).
    pub mode: Mode,

    /// Tool calls awaiting user approval. Non-empty only in
    /// `Mode::ToolApproval`. The user resolves them one at a time
    /// (y to approve, n to deny). When this set empties, the mode
    /// transitions to `Executing`.
    pub pending_tools: HashSet<ToolUseId>,

    /// Tool calls the user approved but whose results haven't arrived
    /// yet. Non-empty only in `Mode::ToolApproval` or `Mode::Executing`.
    /// When this empties in `Executing` mode (and all denials are
    /// recorded), all results are sent to the API together.
    pub approved_tools: HashSet<ToolUseId>,

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
            input: InputBuffer::default(),
            scroll_offset: 0,
            mode: Mode::Normal,
            pending_tools: HashSet::new(),
            approved_tools: HashSet::new(),
            status: StatusLine::default(),
            viewport: Viewport::default(),
            model: "claude-sonnet-4-20250514".to_string(),
        }
    }
}
