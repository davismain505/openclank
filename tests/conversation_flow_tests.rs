//! Integration tests for full conversation flows.
//!
//! These tests combine the update function with the MockBackend to verify
//! end-to-end scenarios: the user sends a message, the mock returns a
//! response (text or tool-use), the app processes it, and we assert on
//! the final state.
//!
//! The [`TestHarness`] struct drives the full event loop without any
//! terminal or network IO. It processes effects returned by `update()`
//! the same way the real runner would: `SendMessage` triggers the mock
//! backend, stream events are fed back through `update()`, and so on.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use futures::StreamExt;

use openclank::backend::mock::{MockBackend, MockResponse};
use openclank::backend::traits::{BackendEvent, ChatBackend};
use openclank::event::effects::Effect;
use openclank::event::input::AppEvent;
use openclank::event::update::update;
use openclank::state::app::{AppState, Mode, StatusKind};
use openclank::state::message::{Role, ToolName, ToolUseId};

/// Drives the app through a full conversation without any IO.
///
/// The harness owns an [`AppState`] and a [`MockBackend`]. It provides
/// methods to simulate user input and process backend responses, running
/// the update loop exactly as the real TUI runner would.
struct TestHarness {
    /// The current application state.
    state: AppState,
    /// The mock backend providing canned responses.
    backend: MockBackend,
}

impl TestHarness {
    /// Create a new harness with the given mock backend.
    fn new(backend: MockBackend) -> Self {
        Self {
            state: AppState::default(),
            backend,
        }
    }

    /// Send a key event through the update function and process any
    /// resulting effects. If a `SendMessage` effect is produced, the
    /// mock backend is called and all stream events are fed back
    /// through `update()`.
    async fn press(&mut self, code: KeyCode) {
        self.send_event(AppEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .await;
    }

    /// Type a string one character at a time, processing each key event.
    async fn type_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.press(KeyCode::Char(ch)).await;
        }
    }

    /// Send an event through the update function and process all
    /// resulting effects. Uses an iterative effect queue rather than
    /// recursion — effects can produce new events (e.g. DenyTool
    /// produces a ToolResult), which produce new effects, and so on.
    async fn send_event(&mut self, event: AppEvent) {
        let (new_state, effects) = update(std::mem::take(&mut self.state), event);
        self.state = new_state;

        let mut effect_queue: Vec<Effect> = effects;
        while let Some(effect) = effect_queue.first().cloned() {
            effect_queue.remove(0);
            let new_effects = self.process_effect(effect).await;
            effect_queue.extend(new_effects);
        }
    }

    /// Execute a single effect the same way the real runner would.
    /// Returns any new effects produced by events triggered by this
    /// effect (e.g. DenyTool triggers a ToolResult which may produce
    /// a SendMessage). The caller drains these iteratively.
    async fn process_effect(&mut self, effect: Effect) -> Vec<Effect> {
        match effect {
            Effect::SendMessage => {
                // Start the stream.
                let (new_state, _) =
                    update(std::mem::take(&mut self.state), AppEvent::ApiStreamStart);
                self.state = new_state;

                // Collect all stream events upfront so we release the
                // borrow on self.state before processing them.
                let stream = self.backend.send(&self.state.conversation.messages());
                let results: Vec<_> = stream.collect().await;

                let mut new_effects = Vec::new();
                for result in results {
                    let event = match result {
                        Ok(BackendEvent::TextDelta(text)) => AppEvent::ApiTextDelta(text),
                        Ok(BackendEvent::ToolUse { id, name, input }) => {
                            AppEvent::ApiToolUse { id, name, input }
                        }
                        Ok(BackendEvent::Done) => AppEvent::ApiDone,
                        Err(err) => AppEvent::ApiError(err.message),
                    };
                    let (new_state, effects) =
                        update(std::mem::take(&mut self.state), event);
                    self.state = new_state;
                    new_effects.extend(effects);
                }
                new_effects
            }

            Effect::Quit => {
                Vec::new()
            }

            Effect::ExecuteTool(_) => {
                // Tool execution is handled explicitly by the test
                // sending ToolResult events.
                Vec::new()
            }

            Effect::DenyTool(tool_id) => {
                // The runner synthesizes a denial ToolResult and
                // delivers it through update(), just like the real
                // runner will. This is necessary for the ToolResult
                // handler to add the denial message to the conversation
                // and eventually emit SendMessage.
                let (new_state, effects) = update(
                    std::mem::take(&mut self.state),
                    AppEvent::ToolResult {
                        tool_use_id: tool_id,
                        content: "Tool denied by user".to_string(),
                        is_error: true,
                    },
                );
                self.state = new_state;
                effects
            }
        }
    }
}

// ─── Simple conversation ─────────────────────────────────────────────

#[tokio::test]
async fn simple_text_conversation() {
    let backend = MockBackend::simple("Hello! How can I help?");
    let mut harness = TestHarness::new(backend);

    // User types and sends a message.
    harness.type_str("hi there").await;
    harness.press(KeyCode::Enter).await;

    // After the mock responds, we should have two messages:
    // the user's and the assistant's.
    assert_eq!(harness.state.conversation.messages().len(), 2);

    let user_msg = &harness.state.conversation.messages()[0];
    assert_eq!(user_msg.role(), &Role::User);
    assert_eq!(user_msg.text(), "hi there");

    let assistant_msg = &harness.state.conversation.messages()[1];
    assert_eq!(assistant_msg.role(), &Role::Assistant);
    assert_eq!(assistant_msg.text(), "Hello! How can I help?");

    // The draft should be finalized and status should be back to normal.
    assert!(harness.state.conversation.draft().is_none());
    assert_eq!(harness.state.status.kind, StatusKind::Info);
}

// ─── Tool use flow ───────────────────────────────────────────────────

#[tokio::test]
async fn tool_use_enters_approval_mode() {
    let tool_id = ToolUseId::new("toolu_test001").unwrap();
    let backend = MockBackend::with_responses(vec![MockResponse::ToolUse {
        leading_text: Some("Let me check.".to_string()),
        id: tool_id.clone(),
        name: ToolName::Bash,
        input: serde_json::json!({"command": "ls -la"}),
    }]);
    let mut harness = TestHarness::new(backend);

    harness.type_str("list files").await;
    harness.press(KeyCode::Enter).await;

    // The app should be in ToolApproval mode, waiting for the user
    // to approve or deny the bash command.
    assert_eq!(harness.state.mode, Mode::ToolApproval);

    // The assistant's message should contain both the leading text
    // and the tool-use block.
    let assistant_msg = &harness.state.conversation.messages()[1];
    assert_eq!(assistant_msg.role(), &Role::Assistant);
    assert_eq!(assistant_msg.text(), "Let me check.");
    assert_eq!(assistant_msg.tool_uses().len(), 1);
}

#[tokio::test]
async fn tool_approval_and_result_continues_conversation() {
    let tool_id = ToolUseId::new("toolu_test001").unwrap();

    // Two responses: first requests a tool, second gives a final answer
    // after seeing the tool result.
    let backend = MockBackend::with_responses(vec![
        MockResponse::ToolUse {
            leading_text: None,
            id: tool_id.clone(),
            name: ToolName::Bash,
            input: serde_json::json!({"command": "whoami"}),
        },
        MockResponse::Text {
            text: "You are root.".to_string(),
            chunk_size: 4,
        },
    ]);
    let mut harness = TestHarness::new(backend);

    // User sends a message, mock responds with tool use.
    harness.type_str("who am i").await;
    harness.press(KeyCode::Enter).await;
    assert!(harness.state.mode == Mode::ToolApproval);

    // User approves the tool call.
    harness.press(KeyCode::Char('y')).await;

    // Simulate the tool executor returning a result, which triggers
    // sending the result back to the API.
    harness
        .send_event(AppEvent::ToolResult {
            tool_use_id: tool_id,
            content: "root".to_string(),
            is_error: false,
        })
        .await;

    // After the second mock response, we should have 4 messages:
    // 1. User: "who am i"
    // 2. Assistant: tool-use request
    // 3. User: tool result ("root")
    // 4. Assistant: "You are root."
    assert_eq!(harness.state.conversation.messages().len(), 4);
    assert_eq!(harness.state.conversation.messages()[3].text(), "You are root.");
    assert_eq!(harness.state.mode, Mode::Normal);
}

#[tokio::test]
async fn tool_denial_returns_to_normal() {
    let tool_id = ToolUseId::new("toolu_test001").unwrap();
    let backend = MockBackend::with_responses(vec![MockResponse::ToolUse {
        leading_text: None,
        id: tool_id,
        name: ToolName::WriteFile,
        input: serde_json::json!({"path": "/etc/passwd", "content": "hacked"}),
    }]);
    let mut harness = TestHarness::new(backend);

    harness.type_str("do something dangerous").await;
    harness.press(KeyCode::Enter).await;
    assert!(harness.state.mode == Mode::ToolApproval);

    // User denies the tool call.
    harness.press(KeyCode::Char('n')).await;

    assert_eq!(harness.state.mode, Mode::Normal);
}

// ─── Error handling ──────────────────────────────────────────────────

#[tokio::test]
async fn api_error_shows_in_status() {
    let backend = MockBackend::with_responses(vec![MockResponse::Error {
        message: "rate limit exceeded".to_string(),
        retryable: true,
    }]);
    let mut harness = TestHarness::new(backend);

    harness.type_str("hello").await;
    harness.press(KeyCode::Enter).await;

    // The error should be shown in the status bar.
    assert_eq!(harness.state.status.kind, StatusKind::Error);
    assert!(harness.state.status.text.contains("rate limit exceeded"));

    // The user message should still be in the conversation, but no
    // assistant message (the draft was discarded).
    assert_eq!(harness.state.conversation.messages().len(), 1);
    assert_eq!(harness.state.conversation.messages()[0].role(), &Role::User);
}

// ─── Multi-turn conversation ─────────────────────────────────────────

#[tokio::test]
async fn multi_turn_conversation() {
    let backend = MockBackend::with_responses(vec![
        MockResponse::Text {
            text: "I'm Claude.".to_string(),
            chunk_size: 4,
        },
        MockResponse::Text {
            text: "I help with coding.".to_string(),
            chunk_size: 4,
        },
    ]);
    let mut harness = TestHarness::new(backend);

    // First turn.
    harness.type_str("who are you").await;
    harness.press(KeyCode::Enter).await;
    assert_eq!(harness.state.conversation.messages().len(), 2);

    // Second turn.
    harness.type_str("what do you do").await;
    harness.press(KeyCode::Enter).await;
    assert_eq!(harness.state.conversation.messages().len(), 4);
    assert_eq!(harness.state.conversation.messages()[3].text(), "I help with coding.");
}
