//! Event handling: input events, side effects, and the pure update function.
//!
//! This module implements the core of the Elm Architecture (TEA) pattern.
//! The [`update::update`] function is the heart of the application: it takes
//! the current [`AppState`](crate::state::app::AppState) and an
//! [`AppEvent`](input::AppEvent), and returns a new state plus a list of
//! [`Effect`](effects::Effect)s to execute.
//!
//! The update function is completely pure — it performs no IO, no network
//! calls, no terminal operations. Side effects are represented as values
//! in the [`Effect`](effects::Effect) enum and returned to the caller
//! (the runner) for execution. This makes the entire behavioral core
//! of the application trivially testable.

pub mod effects;
pub mod input;
pub mod update;
