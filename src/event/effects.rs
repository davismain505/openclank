//! Side effect descriptors returned by the update function.
//!
//! The update function never performs IO directly. Instead, when it needs
//! something to happen in the outside world (send an API request, execute
//! a tool, quit the app), it returns an [`Effect`] value describing what
//! should happen. The runner (TUI or headless) then interprets these
//! effects and performs the actual IO.
//!
//! This separation is what makes the update function pure and testable.
//! In tests, we can inspect the returned effects to verify that the right
//! actions would be triggered without actually performing them.

use crate::state::message::{ToolName, ToolUseId};

/// A tool call that has been approved by the user and is ready for
/// execution by the runner.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// The API-assigned ID for this tool call. The execution result must
    /// be tagged with this ID so the model can match it to the request.
    pub id: ToolUseId,
    /// Which tool to run.
    pub name: ToolName,
    /// The JSON arguments to pass to the tool.
    pub input: serde_json::Value,
}

/// A side effect that the runner should execute after an update.
///
/// Effects are returned as a `Vec<Effect>` from the update function.
/// The runner processes them in order. Most updates produce zero or
/// one effect; tool-approval flows may produce effects that trigger
/// further API calls.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Send the current conversation to the API to get the model's
    /// next response. The runner should send all finalized messages
    /// (plus any tool results) to the chat backend and feed the
    /// resulting stream events back as `AppEvent`s.
    SendMessage,

    /// Execute a tool that the user has approved. The runner should
    /// invoke the tool executor with the contained [`ToolCall`] and
    /// feed the result back as an `AppEvent::ToolResult`.
    ExecuteTool(ToolCall),

    /// The user denied a tool call. The runner should construct a
    /// `ToolResult` with `is_error: true` and a "denied by user"
    /// message, add it to the conversation, and send it back to
    /// the API so the model knows the tool was rejected.
    DenyTool(ToolUseId),

    /// Exit the application. The runner should clean up the terminal
    /// and shut down.
    Quit,
}
