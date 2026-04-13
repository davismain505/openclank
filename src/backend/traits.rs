//! The chat backend trait and its associated types.
//!
//! [`ChatBackend`] is the seam that lets us swap between the real Anthropic
//! API and the [`MockBackend`](super::mock::MockBackend) for testing. Both
//! implementations return a [`BoxStream`] of [`BackendEvent`]s, which the
//! runner consumes to drive the application's state machine.
//!
//! The stream-based design matches the Anthropic streaming API naturally:
//! text arrives in small chunks, tool-use blocks arrive as complete units,
//! and the stream ends with a `Done` event.

use futures::stream::BoxStream;

use crate::state::message::{Message, ToolName, ToolUseId};

/// A single event from the streaming backend.
///
/// The runner maps these into [`AppEvent`](crate::event::input::AppEvent)
/// variants: `TextDelta` → `ApiTextDelta`, `ToolUse` → `ApiToolUse`, etc.
#[derive(Debug, Clone)]
pub enum BackendEvent {
    /// A chunk of text from the model's response. These arrive
    /// incrementally as the model generates tokens.
    TextDelta(String),

    /// The model is requesting a tool call. Unlike text deltas, tool-use
    /// blocks arrive as a complete unit (ID, name, and parsed JSON input)
    /// once the model has finished generating the tool call's arguments.
    ToolUse {
        /// The API-assigned unique ID for this tool call.
        id: ToolUseId,
        /// Which tool the model wants to invoke.
        name: ToolName,
        /// The JSON arguments for the tool.
        input: serde_json::Value,
    },

    /// The stream has finished. No more events will arrive.
    Done,
}

/// An error from the backend.
#[derive(Debug, Clone)]
pub struct BackendError {
    /// A human-readable description of what went wrong.
    pub message: String,
    /// Whether this error is likely transient and worth retrying.
    ///
    /// The runner should implement retry with exponential backoff for
    /// retryable errors (up to ~3 attempts). Non-retryable errors
    /// should be shown to the user immediately without retry.
    ///
    /// Retryable: rate limiting (429), network timeouts, server
    /// errors (5xx). Non-retryable: auth failures (401/403), bad
    /// requests (400).
    pub retryable: bool,
}

/// The abstraction over how messages are sent to the model and responses
/// received.
///
/// Implementations must be `Send + Sync` so they can be shared across
/// async tasks. The `send` method takes the full conversation history
/// and returns a stream of events representing the model's response.
///
/// The stream is a [`BoxStream`] (a heap-allocated, type-erased stream)
/// because different implementations will have different concrete stream
/// types, and we need them behind a single trait object.
pub trait ChatBackend: Send + Sync {
    /// Send the conversation to the model and receive a streaming response.
    ///
    /// The `messages` slice contains the full conversation history that
    /// should be sent to the API. The returned stream yields events as
    /// the model generates its response, ending with a `Done` event on
    /// success or a `BackendError` on failure.
    ///
    /// **Lifetime note:** The returned stream borrows both `self` and
    /// `messages` (shared lifetime `'a`). Both must remain valid for
    /// the entire time the stream is being consumed. In practice, this
    /// means you cannot drop or modify the conversation while iterating
    /// the stream.
    fn send<'a>(
        &'a self,
        messages: &'a [Message],
    ) -> BoxStream<'a, Result<BackendEvent, BackendError>>;
}
