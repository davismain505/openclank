//! Application state types.
//!
//! This module contains all the pure data structures that represent the
//! application's state at any point in time. The state is the single source
//! of truth for both the update logic ([`crate::event::update`]) and the
//! renderer ([`crate::render`]).
//!
//! Nothing in this module performs IO. All types are plain data that can be
//! freely cloned, compared, and serialized.

pub mod app;
pub mod message;
