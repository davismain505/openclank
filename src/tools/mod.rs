//! Tool execution.
//!
//! When the user approves a tool call, the runner dispatches it to a
//! [`ToolExecutor`]. The executor runs the tool (shell command, file
//! read/write, search, etc.) and returns a [`ToolExecResult`] containing
//! the output and an error flag. The runner then wraps the result in an
//! [`AppEvent::ToolResult`](crate::event::input::AppEvent::ToolResult)
//! and delivers it back to [`update()`](crate::event::update::update).
//!
//! The [`ToolExecutor`] trait is abstract so tests can substitute mock
//! implementations. The production implementation in [`executor`] runs
//! actual subprocesses and file operations.

pub mod executor;

pub use executor::{RealToolExecutor, ToolExecResult, ToolExecutor};
