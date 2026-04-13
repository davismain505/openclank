//! openclank — a TUI chat client for the Anthropic Claude API.
//!
//! This crate is structured as a library so that all modules can be tested
//! via `cargo test` without going through `main.rs`. The binary in `main.rs`
//! is a thin wrapper that parses CLI args and wires the pieces together.

pub mod backend;
pub mod event;
pub mod state;
