//! Live API tests that hit the real Anthropic Messages API.
//!
//! These tests are **ignored by default** because they require valid
//! credentials and make real API calls. Run them explicitly with:
//!
//!     cargo test --test live_api_tests -- --ignored
//!
//! Credentials are loaded from `~/.claude/.credentials.json` (the same
//! file Claude Code uses) or from the `ANTHROPIC_API_KEY` env var.

use futures::StreamExt;

use openclank::backend::anthropic::AnthropicBackend;
use openclank::backend::traits::{BackendEvent, ChatBackend};
use openclank::state::message::Message;

/// Resolve an API key or skip the test if none is available.
fn get_backend() -> Option<AnthropicBackend> {
    let api_key = AnthropicBackend::load_claude_credentials().ok()?;
    Some(AnthropicBackend::new(
        api_key,
        "claude-sonnet-4-20250514".to_string(),
    ))
}

#[tokio::test]
#[ignore]
async fn live_simple_text_response() {
    let Some(backend) = get_backend() else {
        eprintln!("skipping: no API credentials found");
        return;
    };

    let messages = vec![Message::user("Reply with exactly the word 'pong' and nothing else.")];
    let mut stream = backend.send(&messages);

    let mut full_text = String::new();
    let mut got_done = false;

    while let Some(result) = stream.next().await {
        match result {
            Ok(BackendEvent::TextDelta(text)) => {
                full_text.push_str(&text);
            }
            Ok(BackendEvent::Done) => {
                got_done = true;
            }
            Ok(BackendEvent::ToolUse { .. }) => {
                panic!("unexpected tool use in simple text test");
            }
            Err(e) => {
                panic!("API error: {}", e.message);
            }
        }
    }

    assert!(got_done, "stream should end with Done event");
    let trimmed = full_text.trim().to_lowercase();
    assert!(
        trimmed.contains("pong"),
        "expected 'pong' in response, got: {full_text:?}"
    );
}
