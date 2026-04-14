//! openclank binary entry point.
//!
//! Parses CLI arguments, loads Claude Code credentials, constructs
//! the backend and tool executor, and hands off to
//! [`tui::runner::run`]. All the real logic lives in the library;
//! this is just wiring.

use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;

use openclank::backend::anthropic::AnthropicBackend;
use openclank::backend::traits::ChatBackend;
use openclank::state::app::AppState;
use openclank::tools::{RealToolExecutor, ToolExecutor};
use openclank::tui::runner;

/// A terminal interface for Claude that runs on illumos.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Anthropic model identifier.
    #[arg(long, default_value = "claude-sonnet-4-20250514")]
    model: String,

    /// System prompt prepended to every request.
    #[arg(long)]
    system_prompt: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // Load the OAuth token from ~/.claude/.credentials.json. This
    // file is created by logging in to Claude Code; since Claude Code
    // doesn't run on illumos, users copy it from another machine.
    let api_key = match AnthropicBackend::load_claude_credentials() {
        Ok(key) => key,
        Err(e) => {
            eprintln!("openclank: could not load Claude Code credentials: {e}");
            eprintln!(
                "Copy ~/.claude/.credentials.json from a machine where you've \
                 logged in to Claude Code."
            );
            return ExitCode::FAILURE;
        }
    };

    let mut backend = AnthropicBackend::new(api_key, cli.model.clone());
    if let Some(prompt) = cli.system_prompt {
        backend = backend.with_system_prompt(prompt);
    }
    let backend: Arc<dyn ChatBackend> = Arc::new(backend);

    let executor: Arc<dyn ToolExecutor> = Arc::new(RealToolExecutor::new());

    let mut initial_state = AppState::default();
    initial_state.model = cli.model;

    match runner::run(backend, executor, initial_state).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("openclank: runtime error: {e}");
            ExitCode::FAILURE
        }
    }
}
