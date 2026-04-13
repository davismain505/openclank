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
