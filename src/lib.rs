//! openclank — a TUI chat client for the Anthropic Claude API.
//!
//! This crate is structured as a library so that all modules can be tested
//! via `cargo test` without going through `main.rs`. The binary in `main.rs`
//! is a thin wrapper that parses CLI args and wires the pieces together.
//!
//! ## Architecture: The Elm Architecture (TEA)
//!
//! The application follows the Elm Architecture pattern. All behavior
//! lives in a pure `update(state, event) -> (state, effects)` function.
//! Side effects are returned as values, never executed inline. The runner
//! (not yet implemented) drives the loop:
//!
//! ```text
//! ┌──────────┐     AppEvent      ┌────────────┐   (AppState,    ┌──────────┐
//! │  Runner   │ ──────────────▶  │  update()   │   Vec<Effect>) │  Runner   │
//! │           │                  │  (pure)     │ ─────────────▶ │           │
//! │           │                  └────────────┘                 │           │
//! │           │                                                 │           │
//! │           │   Effect::SendMessage ──▶ Backend::send()       │           │
//! │           │   BackendEvent ──▶ AppEvent (fed back in)       │           │
//! │           │                                                 │           │
//! │           │   Effect::ExecuteTool ──▶ ToolExecutor           │           │
//! │           │   ToolResult ──▶ AppEvent::ToolResult            │           │
//! │           │                                                 │           │
//! │  render() │ ◀──── &AppState                                 │           │
//! └──────────┘                                                  └──────────┘
//! ```
//!
//! ## Module layout
//!
//! - **[`state`]**: Pure data types ([`AppState`](state::app::AppState),
//!   [`Message`](state::message::Message),
//!   [`Conversation`](state::message::Conversation),
//!   [`InputBuffer`](state::app::InputBuffer)). No IO.
//! - **[`event`]**: The [`update()`](event::update::update) function
//!   (pure state machine), [`AppEvent`](event::input::AppEvent) types,
//!   and [`Effect`](event::effects::Effect) types.
//! - **[`backend`]**: The [`ChatBackend`](backend::traits::ChatBackend)
//!   trait and implementations (Anthropic API, mock for tests).
//! - **[`render`]**: Ratatui rendering functions. Pure transformations
//!   from `AppState` to terminal output.

pub mod backend;
pub mod event;
pub mod render;
pub mod state;
