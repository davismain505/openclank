//! Chat backend abstraction and implementations.
//!
//! This module defines the [`ChatBackend`](traits::ChatBackend) trait that
//! abstracts over how messages are sent to an LLM and responses received.
//! The trait returns a stream of [`BackendEvent`](traits::BackendEvent)s,
//! which the runner maps to [`AppEvent`](crate::event::input::AppEvent)s.
//!
//! Two implementations exist:
//! - [`mock::MockBackend`]: returns canned responses for testing. Supports
//!   text-only responses and tool-use responses with configurable chunking.
//! - `anthropic` (Phase 4): real Anthropic Messages API with SSE streaming.

pub mod anthropic;
#[cfg(any(test, feature = "test-support"))]
pub mod mock;
pub mod traits;
