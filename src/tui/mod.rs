//! The interactive terminal runtime.
//!
//! This module contains the only parts of openclank that touch IO:
//! the crossterm terminal setup/teardown and the event loop that
//! drives the pure [`update()`](crate::event::update::update)
//! function with real events from the terminal, the API, and the
//! tool executor.
//!
//! See [`crate::lib`] for the architecture overview and the
//! [`RunnerLoop` TLA+ model](../../spec/RunnerLoop.tla) for the
//! ordering invariants the runner must maintain.

pub mod runner;
pub mod terminal;
