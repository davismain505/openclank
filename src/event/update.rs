//! The pure update function — the behavioral core of the application.
//!
//! [`update`] is the single function where all application logic lives.
//! It takes the current state and an event, and returns a new state plus
//! any side effects to perform. It is completely pure: no IO, no network,
//! no terminal operations. This makes it trivially testable — construct
//! a state, feed in events, assert on the resulting state and effects.
//!
//! The function is organized by mode: different keybindings are active
//! depending on whether we're in Normal (typing), Scrolling (browsing
//! history), or ToolApproval (reviewing a pending tool call) mode.

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::state::app::{AppState, Mode, StatusKind, StatusLine};
use crate::state::message::{ContentBlock, Message, ToolUseId};

use super::effects::{Effect, ToolCall};
use super::input::AppEvent;

/// Process an event against the current state, returning the new state
/// and any side effects to execute.
///
/// This function is the heart of the Elm Architecture. It handles:
/// - Keyboard input (typing, sending messages, scrolling, tool approval)
/// - Terminal resize events
/// - API streaming events (start, text chunks, tool use, done, error)
/// - Tool execution results
///
/// The returned `Vec<Effect>` is usually empty (pure state changes) or
/// contains a single effect. The runner processes effects in order.
pub fn update(mut state: AppState, event: AppEvent) -> (AppState, Vec<Effect>) {
    let mut effects = Vec::new();

    match event {
        // Keyboard input is dispatched based on the current mode. Each
        // mode has its own set of active keybindings. For example, in
        // Normal mode the arrow keys edit the input buffer, but in
        // Scrolling mode they move the scroll position.
        AppEvent::Key(key_event) => {
            match &state.mode {
                // Normal mode: the user is typing in the input area.
                // Most keys insert characters. Enter sends the message.
                // Ctrl+C quits. Page Up enters scroll mode.
                Mode::Normal => {
                    handle_normal_mode_key(&mut state, &mut effects, key_event);
                }

                // Scrolling mode: the user is browsing message history.
                // Up/Down/j/k scroll the view. Escape or 'i' returns to
                // Normal mode. 'g' jumps to top, 'G' jumps to bottom.
                Mode::Scrolling => {
                    handle_scrolling_mode_key(&mut state, key_event);
                }

                // ToolApproval mode: tool-use requests are pending.
                // The user reviews them one at a time from
                // `pending_tools`. 'y' approves (moves to
                // `approved_tools`), 'n' denies (just removes).
                // When `pending_tools` empties, transitions to
                // Executing.
                Mode::ToolApproval => {
                    handle_tool_approval_key(&mut state, &mut effects, key_event);
                }

                // Executing mode: approved tools are running. Keys
                // are ignored except Ctrl+C to quit.
                Mode::Executing => {
                    if key_event.code == KeyCode::Char('c')
                        && key_event.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        state.mode = Mode::Quitting;
                        effects.push(Effect::Quit);
                    }
                }

                // Quitting mode: ignore all input. The runner will see
                // this mode and exit the event loop.
                Mode::Quitting => {}
            }
        }

        // The terminal window was resized. We store the new dimensions
        // so the renderer can lay out the UI correctly, and so the update
        // function can calculate scroll bounds accurately.
        AppEvent::Resize(cols, rows) => {
            state.viewport.cols = cols;
            state.viewport.rows = rows;
        }

        // The API has begun streaming a new assistant response. We create
        // a MessageDraft to accumulate the incoming content. The draft is
        // mutable and will receive text chunks and tool-use blocks as they
        // arrive. The status bar shows "Streaming..." so the user knows
        // we're waiting for content.
        AppEvent::ApiStreamStart => {
            state.conversation.start_draft();
            state.status = StatusLine {
                text: "Streaming...".to_string(),
                kind: StatusKind::Streaming,
            };
        }

        // A chunk of text arrived from the streaming API. We append it
        // to the current message draft. The renderer will display the
        // draft's accumulated content, so the user sees text appear
        // incrementally as it streams in.
        AppEvent::ApiTextDelta(chunk) => {
            if let Some(draft) = state.conversation.draft_mut() {
                draft.append_text(&chunk);
            }
        }

        // The model emitted a tool-use request during streaming. We
        // add it to the draft but do NOT finalize or enter approval
        // yet — the API can send multiple tool-use blocks in a single
        // response (each as a separate content block), and we need to
        // collect them all before presenting them to the user.
        //
        // Approval happens in the ApiDone handler below, after the
        // draft is finalized and we can see all tool calls at once.
        AppEvent::ApiToolUse { id, name, input } => {
            if let Some(draft) = state.conversation.draft_mut() {
                draft.add_tool_use(id, name, input);
            }
        }

        // The API stream has finished (message_stop event). Finalize
        // the draft into an immutable message.
        //
        // If the finalized message contains tool-use blocks, enter
        // ToolApproval mode for the first one. The user will approve
        // or deny each tool call in sequence. If no tool-use blocks
        // exist, return to the ready state.
        AppEvent::ApiDone => {
            state.conversation.finalize_draft();
            state.scroll_offset = 0;

            // Collect all tool-use IDs from the finalized message.
            // If any exist, enter ToolApproval with the full set.
            // The user resolves them one at a time.
            let tool_ids: HashSet<ToolUseId> = state
                .conversation
                .messages()
                .last()
                .map(|msg| {
                    msg.tool_uses()
                        .into_iter()
                        .map(|(_, id, _, _)| id.clone())
                        .collect()
                })
                .unwrap_or_default();

            if tool_ids.is_empty() {
                state.status = StatusLine::default();
            } else {
                state.pending_tools = tool_ids;
                state.approved_tools.clear();
                state.mode = Mode::ToolApproval;
                state.status = StatusLine {
                    text: "Tool call pending approval (y/n)".to_string(),
                    kind: StatusKind::Info,
                };
            }
        }

        // The API stream encountered an error (network failure, rate
        // limit, invalid request, etc.). We discard the draft since it
        // may contain partial content that would be confusing to display
        // as a completed message. The error is shown in the status bar.
        AppEvent::ApiError(err) => {
            state.conversation.discard_draft();
            state.status = StatusLine {
                text: format!("Error: {err}"),
                kind: StatusKind::Error,
            };
        }

        // A tool finished executing (after the user approved it).
        // Remove it from approved_tools. When all approved tools
        // have returned results, send everything (including denial
        // results) back to the API in a single user message.
        AppEvent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            state.approved_tools.remove(&tool_use_id);
            let msg = Message::from_tool_result(tool_use_id, content, is_error);
            state.conversation.push(msg);

            // If all approved tools have returned results, send
            // the conversation back to the API.
            if state.approved_tools.is_empty() && state.mode == Mode::Executing {
                state.mode = Mode::Normal;
                state.status = StatusLine {
                    text: "Sending tool results...".to_string(),
                    kind: StatusKind::Streaming,
                };
                effects.push(Effect::SendMessage);
            }
        }

        // Periodic tick (e.g. every 100ms) for UI animations.
        // Currently a no-op; intended for streaming spinner or cursor
        // blink. Requires a timer in the runner (not yet implemented).
        AppEvent::Tick => {}
    }

    (state, effects)
}

/// Handle a key event in Normal mode (the user is typing in the input area).
///
/// Keybindings:
/// - Ctrl+C / Ctrl+D: quit
/// - Enter: send message (if input is non-empty)
/// - Shift+Enter / Alt+Enter: insert newline for multi-line input
/// - Backspace: delete character before cursor
/// - Delete: delete character after cursor
/// - Left/Right: move cursor one character
/// - Home/End: move cursor to start/end of input
/// - Page Up: enter scrolling mode
/// - Any printable character: insert at cursor
fn handle_normal_mode_key(
    state: &mut AppState,
    effects: &mut Vec<Effect>,
    key: crossterm::event::KeyEvent,
) {
    match (key.code, key.modifiers) {
        // Quit the application. Ctrl+C is the universal "stop" signal,
        // and Ctrl+D is the traditional Unix "end of input" signal.
        // Both enter Quitting mode and emit the Quit effect so the
        // runner knows to clean up the terminal.
        (KeyCode::Char('c'), KeyModifiers::CONTROL)
        | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            state.mode = Mode::Quitting;
            effects.push(Effect::Quit);
        }

        // Send the current input as a user message. We trim whitespace
        // and skip empty inputs. After sending:
        // - The input buffer is cleared for the next message
        // - Scroll offset resets so we see the latest messages
        // - A SendMessage effect is emitted so the runner calls the API
        (KeyCode::Enter, KeyModifiers::NONE) => {
            if !state.input.is_blank() {
                let text = state.input.take_trimmed();
                state.conversation.push(Message::user(&text));
                state.scroll_offset = 0;
                state.status = StatusLine {
                    text: "Sending...".to_string(),
                    kind: StatusKind::Streaming,
                };
                effects.push(Effect::SendMessage);
            }
        }

        // Insert a literal newline into the input buffer for multi-line
        // messages. Shift+Enter and Alt+Enter are both supported because
        // terminal emulators vary in which modifier they can send.
        (KeyCode::Enter, KeyModifiers::SHIFT) | (KeyCode::Enter, KeyModifiers::ALT) => {
            state.input.insert_char('\n');
        }

        // Delete the character before the cursor (standard backspace).
        (KeyCode::Backspace, _) => {
            state.input.delete_back();
        }

        // Delete the character after the cursor (forward delete).
        (KeyCode::Delete, _) => {
            state.input.delete_forward();
        }

        // Move the cursor left one character.
        (KeyCode::Left, KeyModifiers::NONE) => {
            state.input.move_left();
        }

        // Move the cursor right one character.
        (KeyCode::Right, KeyModifiers::NONE) => {
            state.input.move_right();
        }

        // Jump cursor to the beginning of the input buffer.
        (KeyCode::Home, _) => {
            state.input.move_home();
        }

        // Jump cursor to the end of the input buffer.
        (KeyCode::End, _) => {
            state.input.move_end();
        }

        // Enter scrolling mode and scroll up 10 lines. This switches
        // the keybinding context so that arrow keys scroll the chat
        // view instead of editing the input.
        (KeyCode::PageUp, _) => {
            state.mode = Mode::Scrolling;
            state.scroll_offset = state.scroll_offset.saturating_add(10);
        }

        // Insert a printable character at the cursor position. We
        // accept both unmodified characters and shifted characters
        // (e.g. Shift+A for uppercase, Shift+1 for '!').
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            state.input.insert_char(c);
        }

        // All other key combinations are ignored in Normal mode.
        _ => {}
    }
}

/// Handle a key event in Scrolling mode (the user is browsing history).
///
/// Keybindings (vim-inspired):
/// - Escape / 'i': return to Normal mode
/// - Up / 'k': scroll up one line
/// - Down / 'j': scroll down one line (auto-returns to Normal at bottom)
/// - Page Up: scroll up 10 lines
/// - Page Down: scroll down 10 lines (auto-returns to Normal at bottom)
/// - 'g': jump to top of history
/// - 'G': jump to bottom and return to Normal mode
fn handle_scrolling_mode_key(state: &mut AppState, key: crossterm::event::KeyEvent) {
    match key.code {
        // Exit scrolling mode and return to normal input mode.
        // Escape is the universal "cancel", 'i' mirrors vim's
        // insert-mode entry from normal mode.
        KeyCode::Esc | KeyCode::Char('i') => {
            state.mode = Mode::Normal;
        }

        // Scroll up one line in the chat history. Uses saturating
        // addition so we can't overflow.
        KeyCode::Up | KeyCode::Char('k') => {
            state.scroll_offset = state.scroll_offset.saturating_add(1);
        }

        // Scroll down one line towards the latest messages. If we
        // reach the bottom (offset 0), automatically switch back
        // to Normal mode since there's nothing more to scroll to.
        KeyCode::Down | KeyCode::Char('j') => {
            state.scroll_offset = state.scroll_offset.saturating_sub(1);
            if state.scroll_offset == 0 {
                state.mode = Mode::Normal;
            }
        }

        // Scroll up by a page (10 lines).
        KeyCode::PageUp => {
            state.scroll_offset = state.scroll_offset.saturating_add(10);
        }

        // Scroll down by a page (10 lines). Same auto-return-to-Normal
        // behavior as single-line down scroll.
        KeyCode::PageDown => {
            state.scroll_offset = state.scroll_offset.saturating_sub(10);
            if state.scroll_offset == 0 {
                state.mode = Mode::Normal;
            }
        }

        // Jump to the very top of the conversation history. We use
        // usize::MAX which the renderer will clamp to the actual
        // maximum scroll position.
        KeyCode::Char('g') => {
            state.scroll_offset = usize::MAX;
        }

        // Jump to the bottom of the conversation (latest messages)
        // and return to Normal mode.
        KeyCode::Char('G') => {
            state.scroll_offset = 0;
            state.mode = Mode::Normal;
        }

        // All other keys are ignored in scrolling mode.
        _ => {}
    }
}

/// Handle a key event in ToolApproval mode (reviewing pending tool calls).
///
/// The current tool being reviewed is the first entry in
/// `state.pending_tools`. 'y' approves it (moves to `approved_tools`
/// and emits `ExecuteTool`), 'n' denies it (emits `DenyTool`). When
/// `pending_tools` empties, we transition to `Executing` mode where
/// the runner waits for all approved tool results before sending
/// everything back to the API.
///
/// Keybindings:
/// - 'y': approve the current tool call
/// - 'n': deny the current tool call
/// - Ctrl+C: quit the application
fn handle_tool_approval_key(
    state: &mut AppState,
    effects: &mut Vec<Effect>,
    key: crossterm::event::KeyEvent,
) {
    // Pick the current tool to review. If pending_tools is empty
    // (shouldn't happen in ToolApproval mode), do nothing.
    let Some(current_tool_id) = state.pending_tools.iter().next().cloned() else {
        return;
    };

    match key.code {
        // Approve the current tool. Move it from pending to approved,
        // emit an ExecuteTool effect so the runner starts it, then
        // advance to the next pending tool or transition to Executing.
        KeyCode::Char('y') => {
            state.pending_tools.remove(&current_tool_id);

            if let Some(tool_call) = find_tool_call_by_id(state, &current_tool_id) {
                state.approved_tools.insert(current_tool_id);
                effects.push(Effect::ExecuteTool(tool_call));
                advance_tool_approval(state);
            } else {
                // The tool ID in pending_tools doesn't match any message
                // in the conversation. This indicates a bug in state
                // management. Show an error and transition out of
                // ToolApproval so the user isn't stuck. We must still
                // drain pending_tools to maintain the mode invariant.
                state.pending_tools.clear();
                state.approved_tools.clear();
                state.mode = Mode::Normal;
                state.status = StatusLine {
                    text: "Internal error: tool call not found".to_string(),
                    kind: StatusKind::Error,
                };
            }
        }

        // Deny the current tool. Remove from pending (but don't add
        // to approved), emit DenyTool so the runner records the denial,
        // then advance.
        KeyCode::Char('n') => {
            state.pending_tools.remove(&current_tool_id);
            effects.push(Effect::DenyTool(current_tool_id));
            advance_tool_approval(state);
        }

        // Allow quitting even while tool approvals are pending.
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.mode = Mode::Quitting;
            state.pending_tools.clear();
            state.approved_tools.clear();
            effects.push(Effect::Quit);
        }

        // All other keys are ignored.
        _ => {}
    }
}

/// After approving or denying a tool, check if more are pending.
/// If so, update the status to show the next one. Otherwise
/// transition to Executing mode, where the runner waits for tool
/// results (for approved tools) or delivers synthetic denial
/// results (for denied tools) before the conversation is sent.
fn advance_tool_approval(state: &mut AppState) {
    if !state.pending_tools.is_empty() {
        // More tools to review.
        state.status = StatusLine {
            text: "Tool call pending approval (y/n)".to_string(),
            kind: StatusKind::Info,
        };
    } else if state.approved_tools.is_empty() {
        // All tools were denied (none approved). Transition to
        // Executing mode. The runner will process DenyTool effects,
        // deliver synthetic ToolResult events through update(), and
        // the ToolResult handler will emit SendMessage when all
        // denial results have been added to the conversation.
        //
        // We do NOT emit SendMessage here — that would race with
        // the DenyTool effects that haven't been processed yet.
        // The TLA+ RunnerLoop model (spec/RunnerLoop.tla) verified
        // that this ordering is necessary.
        state.mode = Mode::Executing;
        state.status = StatusLine {
            text: "Processing tool results...".to_string(),
            kind: StatusKind::Streaming,
        };
    } else {
        // Some tools were approved. Wait for their results in
        // Executing mode. The runner will feed ToolResult events
        // back as each tool finishes.
        state.mode = Mode::Executing;
        state.status = StatusLine {
            text: "Executing tools...".to_string(),
            kind: StatusKind::Streaming,
        };
    }
}

/// Search the conversation for a tool-use content block matching the given ID
/// and construct a [`ToolCall`] from it.
///
/// We search messages in reverse order because the tool call we're looking for
/// is almost always in the most recent assistant message. For each message, we
/// check every content block to see if it's a ToolUse with a matching ID.
///
/// Returns `None` if the ID is not found, which would indicate a bug — the
/// ToolApproval mode should only contain IDs that exist in the conversation.
fn find_tool_call_by_id(state: &AppState, target_id: &ToolUseId) -> Option<ToolCall> {
    // Walk through messages from newest to oldest.
    for message in state.conversation.messages().iter().rev() {
        // Check each content block in this message.
        for block in message.content() {
            // We only care about ToolUse blocks.
            let ContentBlock::ToolUse { id, name, input } = block else {
                continue;
            };

            // Skip blocks that don't match the ID we're looking for.
            if id != target_id {
                continue;
            }

            // Found it — build a ToolCall from the content block's fields.
            return Some(ToolCall {
                id: target_id.clone(),
                name: *name,
                input: input.clone(),
            });
        }
    }

    None
}

// Note: prev_char_boundary and next_char_boundary previously lived here.
// They have been moved into InputBuffer::prev_boundary and
// InputBuffer::next_boundary, where the char-boundary invariant is
// enforced by construction rather than by caller discipline.
