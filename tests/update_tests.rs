//! Unit tests for the pure update function.
//!
//! These tests verify that the update function produces the correct state
//! transitions and effects for every kind of event. Since the update function
//! is pure (no IO), these tests are instant and deterministic.
//!
//! The tests are organized by mode and event type:
//! - Normal mode: typing, sending messages, quitting
//! - Scrolling mode: scroll navigation, mode transitions
//! - ToolApproval mode: approving, denying, quitting from approval
//! - API events: streaming lifecycle, tool use, errors
//! - Tool results: the tool-use loop

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use openclank::event::effects::Effect;
use openclank::event::input::AppEvent;
use openclank::event::update::update;
use openclank::state::app::{AppState, Mode, StatusKind};
use openclank::state::message::{
    ContentBlock, Message, Role, ToolName, ToolUseId,
};

/// Helper to create a key press event with no modifiers.
fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

/// Helper to create a key press event with the given modifiers.
fn key_mod(code: KeyCode, modifiers: KeyModifiers) -> AppEvent {
    AppEvent::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

/// Helper to type a string into the input buffer one character at a time,
/// returning the final state. This simulates the user typing naturally.
fn type_string(mut state: AppState, text: &str) -> AppState {
    for ch in text.chars() {
        let (new_state, _) = update(state, key(KeyCode::Char(ch)));
        state = new_state;
    }
    state
}

// ─── Normal mode: typing ─────────────────────────────────────────────

#[test]
fn typing_characters_appends_to_input_buffer() {
    let state = AppState::default();
    let state = type_string(state, "hello");

    assert_eq!(state.input.text(), "hello");
    assert_eq!(state.input.cursor(), 5);
}

#[test]
fn typing_multibyte_characters_tracks_byte_offsets() {
    let state = AppState::default();
    // 'é' is 2 bytes in UTF-8, '日' is 3 bytes
    let state = type_string(state, "é日");

    assert_eq!(state.input.text(), "é日");
    // cursor_pos is a byte offset: 2 bytes for 'é' + 3 bytes for '日'
    assert_eq!(state.input.cursor(), 5);
}

#[test]
fn backspace_deletes_character_before_cursor() {
    let state = AppState::default();
    let state = type_string(state, "hello");

    let (state, _) = update(state, key(KeyCode::Backspace));

    assert_eq!(state.input.text(), "hell");
    assert_eq!(state.input.cursor(), 4);
}

#[test]
fn backspace_at_start_of_input_does_nothing() {
    let state = AppState::default();

    let (state, _) = update(state, key(KeyCode::Backspace));

    assert_eq!(state.input.text(), "");
    assert_eq!(state.input.cursor(), 0);
}

#[test]
fn backspace_handles_multibyte_characters() {
    let state = AppState::default();
    let state = type_string(state, "aé");

    let (state, _) = update(state, key(KeyCode::Backspace));

    // Should delete the 'é' (2 bytes), leaving just 'a'
    assert_eq!(state.input.text(), "a");
    assert_eq!(state.input.cursor(), 1);
}

#[test]
fn delete_removes_character_after_cursor() {
    let state = AppState::default();
    let mut state = type_string(state, "hello");
    // Move cursor to position 3 ("hel|lo") — two left from end
    state.input.move_left(); // after 'o' → after 'l'
    state.input.move_left(); // after 'l' → after 'l' (pos 3)

    let (state, _) = update(state, key(KeyCode::Delete));

    assert_eq!(state.input.text(), "helo");
    assert_eq!(state.input.cursor(), 3);
}

#[test]
fn delete_at_end_of_input_does_nothing() {
    let state = AppState::default();
    let state = type_string(state, "hello");

    let (state, _) = update(state, key(KeyCode::Delete));

    assert_eq!(state.input.text(), "hello");
    assert_eq!(state.input.cursor(), 5);
}

#[test]
fn left_arrow_moves_cursor_back_one_character() {
    let state = AppState::default();
    let state = type_string(state, "hi");

    let (state, _) = update(state, key(KeyCode::Left));

    assert_eq!(state.input.cursor(), 1);
}

#[test]
fn left_arrow_at_start_does_nothing() {
    let state = AppState::default();

    let (state, _) = update(state, key(KeyCode::Left));

    assert_eq!(state.input.cursor(), 0);
}

#[test]
fn right_arrow_moves_cursor_forward_one_character() {
    let state = AppState::default();
    let mut state = type_string(state, "hi");
    state.input.move_home();

    let (state, _) = update(state, key(KeyCode::Right));

    assert_eq!(state.input.cursor(), 1);
}

#[test]
fn right_arrow_at_end_does_nothing() {
    let state = AppState::default();
    let state = type_string(state, "hi");

    let (state, _) = update(state, key(KeyCode::Right));

    assert_eq!(state.input.cursor(), 2);
}

#[test]
fn home_moves_cursor_to_start() {
    let state = AppState::default();
    let state = type_string(state, "hello");

    let (state, _) = update(state, key(KeyCode::Home));

    assert_eq!(state.input.cursor(), 0);
}

#[test]
fn end_moves_cursor_to_end() {
    let state = AppState::default();
    let mut state = type_string(state, "hello");
    state.input.move_home();

    let (state, _) = update(state, key(KeyCode::End));

    assert_eq!(state.input.cursor(), 5);
}

#[test]
fn shift_enter_inserts_newline() {
    let state = AppState::default();
    let state = type_string(state, "line1");

    let (state, _) = update(state, key_mod(KeyCode::Enter, KeyModifiers::SHIFT));
    let state = type_string(state, "line2");

    assert_eq!(state.input.text(), "line1\nline2");
}

// ─── Normal mode: sending messages ───────────────────────────────────

#[test]
fn enter_sends_message_and_clears_input() {
    let state = AppState::default();
    let state = type_string(state, "hello claude");

    let (state, effects) = update(state, key(KeyCode::Enter));

    // Input should be cleared.
    assert_eq!(state.input.text(), "");
    assert_eq!(state.input.cursor(), 0);

    // The message should be in the conversation.
    assert_eq!(state.conversation.messages().len(), 1);
    assert_eq!(state.conversation.messages()[0].text(), "hello claude");
    assert_eq!(state.conversation.messages()[0].role(), &Role::User);

    // A SendMessage effect should be emitted.
    assert_eq!(effects, vec![Effect::SendMessage]);

    // Status should indicate sending.
    assert_eq!(state.status.kind, StatusKind::Streaming);
}

#[test]
fn enter_on_empty_input_does_nothing() {
    let state = AppState::default();

    let (state, effects) = update(state, key(KeyCode::Enter));

    assert_eq!(state.conversation.messages().len(), 0);
    assert!(effects.is_empty());
}

#[test]
fn enter_on_whitespace_only_input_does_nothing() {
    let state = AppState::default();
    let state = type_string(state, "   ");

    let (state, effects) = update(state, key(KeyCode::Enter));

    assert_eq!(state.conversation.messages().len(), 0);
    assert!(effects.is_empty());
}

#[test]
fn enter_trims_whitespace_from_message() {
    let state = AppState::default();
    let state = type_string(state, "  hello  ");

    let (state, _) = update(state, key(KeyCode::Enter));

    assert_eq!(state.conversation.messages()[0].text(), "hello");
}

// ─── Normal mode: quitting ───────────────────────────────────────────

#[test]
fn ctrl_c_quits() {
    let state = AppState::default();

    let (state, effects) = update(state, key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert_eq!(state.mode, Mode::Quitting);
    assert_eq!(effects, vec![Effect::Quit]);
}

#[test]
fn ctrl_d_quits() {
    let state = AppState::default();

    let (state, effects) = update(state, key_mod(KeyCode::Char('d'), KeyModifiers::CONTROL));

    assert_eq!(state.mode, Mode::Quitting);
    assert_eq!(effects, vec![Effect::Quit]);
}

// ─── Scrolling mode ──────────────────────────────────────────────────

#[test]
fn page_up_enters_scrolling_mode() {
    let state = AppState::default();

    let (state, _) = update(state, key(KeyCode::PageUp));

    assert_eq!(state.mode, Mode::Scrolling);
    assert_eq!(state.scroll_offset, 10);
}

#[test]
fn escape_exits_scrolling_mode() {
    let mut state = AppState::default();
    state.mode = Mode::Scrolling;
    state.scroll_offset = 5;

    let (state, _) = update(state, key(KeyCode::Esc));

    assert_eq!(state.mode, Mode::Normal);
}

#[test]
fn scrolling_down_to_zero_returns_to_normal() {
    let mut state = AppState::default();
    state.mode = Mode::Scrolling;
    state.scroll_offset = 1;

    let (state, _) = update(state, key(KeyCode::Down));

    assert_eq!(state.scroll_offset, 0);
    assert_eq!(state.mode, Mode::Normal);
}

#[test]
fn scrolling_up_increments_offset() {
    let mut state = AppState::default();
    state.mode = Mode::Scrolling;
    state.scroll_offset = 5;

    let (state, _) = update(state, key(KeyCode::Up));

    assert_eq!(state.scroll_offset, 6);
}

#[test]
fn g_jumps_to_top() {
    let mut state = AppState::default();
    state.mode = Mode::Scrolling;

    let (state, _) = update(state, key(KeyCode::Char('g')));

    assert_eq!(state.scroll_offset, usize::MAX);
}

#[test]
fn shift_g_jumps_to_bottom_and_exits_scrolling() {
    let mut state = AppState::default();
    state.mode = Mode::Scrolling;
    state.scroll_offset = 100;

    let (state, _) = update(state, key(KeyCode::Char('G')));

    assert_eq!(state.scroll_offset, 0);
    assert_eq!(state.mode, Mode::Normal);
}

// ─── API streaming events ────────────────────────────────────────────

#[test]
fn api_stream_start_creates_draft() {
    let state = AppState::default();

    let (state, _) = update(state, AppEvent::ApiStreamStart);

    assert!(state.conversation.draft().is_some());
    assert_eq!(state.status.kind, StatusKind::Streaming);
}

#[test]
fn api_text_delta_appends_to_draft() {
    let state = AppState::default();
    let (state, _) = update(state, AppEvent::ApiStreamStart);
    let (state, _) = update(state, AppEvent::ApiTextDelta("hello ".to_string()));
    let (state, _) = update(state, AppEvent::ApiTextDelta("world".to_string()));

    assert_eq!(state.conversation.draft().as_ref().unwrap().text(), "hello world");
}

#[test]
fn api_done_finalizes_draft_into_message() {
    let state = AppState::default();
    let (state, _) = update(state, AppEvent::ApiStreamStart);
    let (state, _) = update(state, AppEvent::ApiTextDelta("response".to_string()));
    let (state, _) = update(state, AppEvent::ApiDone);

    // Draft should be gone, message should exist.
    assert!(state.conversation.draft().is_none());
    assert_eq!(state.conversation.messages().len(), 1);
    assert_eq!(state.conversation.messages()[0].text(), "response");
    assert_eq!(state.conversation.messages()[0].role(), &Role::Assistant);
    assert_eq!(state.status.kind, StatusKind::Info);
}

#[test]
fn api_error_discards_draft_and_shows_error() {
    let state = AppState::default();
    let (state, _) = update(state, AppEvent::ApiStreamStart);
    let (state, _) = update(state, AppEvent::ApiTextDelta("partial".to_string()));
    let (state, _) = update(state, AppEvent::ApiError("rate limited".to_string()));

    // Draft should be discarded — no partial message in conversation.
    assert!(state.conversation.draft().is_none());
    assert_eq!(state.conversation.messages().len(), 0);
    assert_eq!(state.status.kind, StatusKind::Error);
    assert!(state.status.text.contains("rate limited"));
}

// ─── Tool use flow ───────────────────────────────────────────────────

#[test]
fn api_tool_use_enters_approval_on_done() {
    let state = AppState::default();
    let tool_id = ToolUseId::new("toolu_abc123").unwrap();
    let (state, _) = update(state, AppEvent::ApiStreamStart);

    // ApiToolUse adds the tool call to the draft but does NOT enter
    // ToolApproval yet — we need to wait for ApiDone to see all tools.
    let (state, _) = update(
        state,
        AppEvent::ApiToolUse {
            id: tool_id.clone(),
            name: ToolName::Bash,
            input: serde_json::json!({"command": "ls"}),
        },
    );
    assert!(state.conversation.draft().is_some(), "draft should still exist after ApiToolUse");

    // ApiDone finalizes the draft and enters ToolApproval for the
    // first tool-use block in the finalized message.
    let (state, _) = update(state, AppEvent::ApiDone);

    assert_eq!(state.mode, Mode::ToolApproval(tool_id.clone()));
    assert!(state.conversation.draft().is_none());
    assert_eq!(state.conversation.messages().len(), 1);

    let tool_uses = state.conversation.messages()[0].tool_uses();
    assert_eq!(tool_uses.len(), 1);
    assert_eq!(tool_uses[0].1, &tool_id);
    assert_eq!(tool_uses[0].2, ToolName::Bash);
}

#[test]
fn approving_tool_emits_execute_effect() {
    let state = AppState::default();
    let tool_id = ToolUseId::new("toolu_abc123").unwrap();

    // Stream a tool-use response and finalize it.
    let (state, _) = update(state, AppEvent::ApiStreamStart);
    let (state, _) = update(
        state,
        AppEvent::ApiToolUse {
            id: tool_id.clone(),
            name: ToolName::Bash,
            input: serde_json::json!({"command": "ls -la"}),
        },
    );
    let (state, _) = update(state, AppEvent::ApiDone);
    assert_eq!(state.mode, Mode::ToolApproval(tool_id.clone()));

    // Press 'y' to approve.
    let (state, effects) = update(state, key(KeyCode::Char('y')));

    assert_eq!(state.mode, Mode::Normal);
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::ExecuteTool(tool_call) => {
            assert_eq!(tool_call.id, tool_id);
            assert_eq!(tool_call.name, ToolName::Bash);
        }
        other => panic!("expected ExecuteTool, got {other:?}"),
    }
}

#[test]
fn denying_tool_emits_deny_effect() {
    let state = AppState::default();
    let tool_id = ToolUseId::new("toolu_abc123").unwrap();

    let (state, _) = update(state, AppEvent::ApiStreamStart);
    let (state, _) = update(
        state,
        AppEvent::ApiToolUse {
            id: tool_id.clone(),
            name: ToolName::ReadFile,
            input: serde_json::json!({"path": "/etc/passwd"}),
        },
    );
    let (state, _) = update(state, AppEvent::ApiDone);

    // Press 'n' to deny.
    let (state, effects) = update(state, key(KeyCode::Char('n')));

    assert_eq!(state.mode, Mode::Normal);
    assert_eq!(effects, vec![Effect::DenyTool(tool_id)]);
}

#[test]
fn ctrl_c_from_tool_approval_quits() {
    let mut state = AppState::default();
    let tool_id = ToolUseId::new("toolu_abc123").unwrap();
    state.mode = Mode::ToolApproval(tool_id);

    let (state, effects) = update(
        state,
        key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );

    assert_eq!(state.mode, Mode::Quitting);
    assert_eq!(effects, vec![Effect::Quit]);
}

// ─── Tool result handling ────────────────────────────────────────────

#[test]
fn tool_result_adds_message_and_sends_to_api() {
    let mut state = AppState::default();
    let tool_id = ToolUseId::new("toolu_abc123").unwrap();

    // Simulate the conversation having a user message and an assistant
    // tool-use message already.
    state.conversation.push(Message::user("list files"));
    state.conversation.push(Message::assistant("Let me check."));

    let (state, effects) = update(
        state,
        AppEvent::ToolResult {
            tool_use_id: tool_id.clone(),
            content: "file1.txt\nfile2.txt".to_string(),
            is_error: false,
        },
    );

    // Should have 3 messages now: user, assistant, tool result.
    assert_eq!(state.conversation.messages().len(), 3);

    // The tool result message should be a user message with a ToolResult block.
    let result_msg = &state.conversation.messages()[2];
    assert_eq!(result_msg.role(), &Role::User);
    let block = &result_msg.content()[0];
    match block {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            assert_eq!(tool_use_id, &tool_id);
            assert_eq!(content, "file1.txt\nfile2.txt");
            assert!(!is_error);
        }
        other => panic!("expected ToolResult block, got {other:?}"),
    }

    // Should send the conversation back to the API.
    assert_eq!(effects, vec![Effect::SendMessage]);
}

#[test]
fn tool_error_result_is_flagged() {
    let state = AppState::default();
    let tool_id = ToolUseId::new("toolu_abc123").unwrap();

    let (state, _) = update(
        state,
        AppEvent::ToolResult {
            tool_use_id: tool_id,
            content: "command not found: foobar".to_string(),
            is_error: true,
        },
    );

    let block = &state.conversation.messages()[0].content()[0];
    match block {
        ContentBlock::ToolResult { is_error, .. } => assert!(is_error),
        other => panic!("expected ToolResult block, got {other:?}"),
    }
}

// ─── Resize events ───────────────────────────────────────────────────

#[test]
fn resize_updates_viewport() {
    let state = AppState::default();

    let (state, effects) = update(state, AppEvent::Resize(120, 40));

    assert_eq!(state.viewport.cols, 120);
    assert_eq!(state.viewport.rows, 40);
    assert!(effects.is_empty());
}
