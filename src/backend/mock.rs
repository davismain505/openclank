//! A mock chat backend for testing.
//!
//! [`MockBackend`] returns pre-configured responses without making any
//! network calls. It supports:
//! - Text-only responses (split into configurable chunks to simulate streaming)
//! - Tool-use responses (the model requests a tool call)
//! - Error responses
//! - Multi-turn sequences (each call to `send` returns the next response)
//!
//! This module is gated behind `#[cfg(any(test, feature = "test-support"))]`
//! so it is never included in release builds.

use std::sync::atomic::{AtomicUsize, Ordering};

use futures::stream::{self, BoxStream, StreamExt};

use crate::state::message::{Message, ToolName, ToolUseId};

use super::traits::{BackendError, BackendEvent, ChatBackend};

/// A single pre-configured response that the mock backend will return.
///
/// Each call to [`MockBackend::send`] consumes the next `MockResponse`
/// from the list. If the list is exhausted, subsequent calls return an
/// error.
#[derive(Debug, Clone)]
pub enum MockResponse {
    /// A text-only response. The text is split into chunks of
    /// `chunk_size` characters to simulate streaming.
    Text {
        /// The full response text.
        text: String,
        /// How many characters per chunk. Smaller values simulate
        /// slower, more granular streaming.
        chunk_size: usize,
    },

    /// A response where the model requests a tool call. May include
    /// leading text before the tool-use block (as the real API does
    /// when the model says "Let me check..." before calling a tool).
    ToolUse {
        /// Optional text before the tool call (e.g. "Let me look at that file.").
        leading_text: Option<String>,
        /// The tool call ID (must be a valid `toolu_` prefixed ID).
        id: ToolUseId,
        /// Which tool to call.
        name: ToolName,
        /// The JSON arguments.
        input: serde_json::Value,
    },

    /// Simulate an API error.
    Error {
        /// The error message.
        message: String,
        /// Whether the error is retryable.
        retryable: bool,
    },
}

/// A chat backend that returns pre-configured responses.
///
/// Responses are consumed in order: the first call to `send` returns
/// `responses[0]`, the second returns `responses[1]`, and so on. This
/// lets tests script multi-turn conversations with tool-use loops.
///
/// Thread-safe via an atomic counter — no mutex needed.
pub struct MockBackend {
    /// The pre-configured responses, consumed in order.
    responses: Vec<MockResponse>,
    /// Index of the next response to return. Incremented atomically
    /// on each call to `send`.
    call_index: AtomicUsize,
}

impl MockBackend {
    /// Create a mock that returns a single text response, chunked
    /// into groups of 4 characters. This is the simplest constructor
    /// for basic tests that just need a canned reply.
    pub fn simple(text: &str) -> Self {
        Self {
            responses: vec![MockResponse::Text {
                text: text.to_string(),
                chunk_size: 4,
            }],
            call_index: AtomicUsize::new(0),
        }
    }

    /// Create a mock with a specific sequence of responses.
    /// Each call to `send` returns the next response in order.
    pub fn with_responses(responses: Vec<MockResponse>) -> Self {
        Self {
            responses,
            call_index: AtomicUsize::new(0),
        }
    }
}

impl ChatBackend for MockBackend {
    fn send<'a>(
        &'a self,
        _messages: &'a [Message],
    ) -> BoxStream<'a, Result<BackendEvent, BackendError>> {
        let index = self.call_index.fetch_add(1, Ordering::Relaxed);

        // If we've exhausted the configured responses, return an error
        // so the test fails clearly rather than hanging.
        let Some(response) = self.responses.get(index) else {
            return stream::once(async {
                Err(BackendError {
                    message: "MockBackend: no more responses configured".to_string(),
                    retryable: false,
                })
            })
            .boxed();
        };

        match response.clone() {
            MockResponse::Text { text, chunk_size } => {
                // Split the text into chunks to simulate streaming,
                // then append a Done event to signal end of response.
                let chunks = split_into_chunks(&text, chunk_size);
                let events: Vec<Result<BackendEvent, BackendError>> = chunks
                    .into_iter()
                    .map(|chunk| Ok(BackendEvent::TextDelta(chunk)))
                    .chain(std::iter::once(Ok(BackendEvent::Done)))
                    .collect();
                stream::iter(events).boxed()
            }

            MockResponse::ToolUse {
                leading_text,
                id,
                name,
                input,
            } => {
                // Optionally emit text before the tool call (the real API
                // does this when the model says something like "Let me
                // check that file" before emitting the tool-use block).
                let mut events: Vec<Result<BackendEvent, BackendError>> = Vec::new();
                if let Some(text) = leading_text {
                    events.push(Ok(BackendEvent::TextDelta(text)));
                }
                events.push(Ok(BackendEvent::ToolUse { id, name, input }));
                events.push(Ok(BackendEvent::Done));
                stream::iter(events).boxed()
            }

            MockResponse::Error {
                message,
                retryable,
            } => stream::once(async move { Err(BackendError { message, retryable }) }).boxed(),
        }
    }
}

/// Split a string into chunks of at most `size` characters.
/// Respects UTF-8 character boundaries — each chunk contains
/// whole characters, never partial ones.
fn split_into_chunks(text: &str, size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut chars = text.chars().peekable();
    while chars.peek().is_some() {
        let chunk: String = chars.by_ref().take(size).collect();
        chunks.push(chunk);
    }
    chunks
}
