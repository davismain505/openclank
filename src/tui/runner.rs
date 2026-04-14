//! The interactive event loop.
//!
//! The runner is the single place where IO happens. Its job is to:
//!
//! 1. Read events from three concurrent sources — the terminal (key
//!    presses, resizes), the active API stream (text deltas, tool
//!    uses, done, error), and tool executor tasks (results) —
//!    multiplexing them through a single tokio channel.
//! 2. Deliver each event to [`update()`](crate::event::update::update)
//!    one at a time. Never concurrently.
//! 3. Process the effects [`update()`](crate::event::update::update)
//!    returns, in order. Some effects (`DenyTool`) produce new events
//!    that flow back through `update()` before the next effect runs;
//!    this ordering is what the [`RunnerLoop` TLA+ model](../../../spec/RunnerLoop.tla)
//!    verifies.
//! 4. Render the current state to the terminal after each update.
//!
//! The runner is async because the API stream and tool execution
//! are both IO-bound. The `update()` function itself is synchronous
//! and pure; this is only about coordinating the IO around it.

use std::sync::Arc;

use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::backend::traits::{BackendEvent, ChatBackend};
use crate::event::effects::{Effect, ToolCall};
use crate::event::input::AppEvent;
use crate::event::update::update;
use crate::render::layout::render_app;
use crate::state::app::{AppState, Mode};
use crate::state::message::{Message, ToolUseId};
use crate::tools::ToolExecutor;

use super::terminal::TerminalGuard;

/// Run the interactive TUI until the user quits.
///
/// Owns the terminal guard, the channel that merges event sources,
/// and the backend/executor used to act on effects. Exits cleanly
/// when `update()` returns `Effect::Quit` or the user presses
/// Ctrl+C / Ctrl+D.
pub async fn run(
    backend: Arc<dyn ChatBackend>,
    executor: Arc<dyn ToolExecutor>,
    initial_state: AppState,
) -> std::io::Result<()> {
    let mut guard = TerminalGuard::enter()?;
    let mut state = initial_state;

    // Single channel for all events. Every source (keyboard, API,
    // tools) funnels events in here; the main loop reads them out
    // one at a time. This is what enforces "update() is never called
    // concurrently" — there's a single consumer.
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    // Spawn the keyboard/resize reader. crossterm's EventStream gives
    // us async events from the terminal; we translate each one to an
    // AppEvent and forward it.
    let keyboard_tx = event_tx.clone();
    tokio::spawn(async move {
        let mut events = EventStream::new();
        while let Some(Ok(event)) = events.next().await {
            let app_event = match event {
                Event::Key(key) => AppEvent::Key(key),
                Event::Resize(cols, rows) => AppEvent::Resize(cols, rows),
                _ => continue,
            };
            if keyboard_tx.send(app_event).is_err() {
                break; // main loop has shut down
            }
        }
    });

    // Initial render before we block on any event.
    guard
        .terminal()
        .draw(|frame| render_app(frame, &state))
        .ok();

    // Main loop. Receive one event, update, process effects, render,
    // repeat. This is the enforcement point for the RunnerLoop model's
    // invariant that update() is never called concurrently.
    while let Some(event) = event_rx.recv().await {
        let (new_state, effects) = update(state, event);
        state = new_state;

        if state.mode == Mode::Quitting {
            break;
        }

        state = process_effects(state, effects, &backend, &executor, &event_tx).await;

        if state.mode == Mode::Quitting {
            break;
        }

        guard
            .terminal()
            .draw(|frame| render_app(frame, &state))
            .ok();
    }

    Ok(())
}

/// Process a list of effects in order, delivering any events they
/// produce through `update()` before moving on to the next effect.
///
/// This is the enforcement point for the RunnerLoop invariant that
/// effects from one `update()` call are fully processed — including
/// any follow-up events they trigger — before the next external
/// event is handled.
///
/// The trickiest effect is `DenyTool`: the runner must synthesize a
/// `ToolResult` event and deliver it through `update()` *before* any
/// later `SendMessage` effect executes, otherwise the API request
/// would be sent without the denial message in the conversation.
/// Doing this synchronously in a loop (rather than posting back to
/// the event channel) guarantees the ordering.
async fn process_effects(
    mut state: AppState,
    effects: Vec<Effect>,
    backend: &Arc<dyn ChatBackend>,
    executor: &Arc<dyn ToolExecutor>,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
) -> AppState {
    let mut queue: Vec<Effect> = effects;
    while !queue.is_empty() {
        let effect = queue.remove(0);
        match effect {
            Effect::SendMessage => {
                state = run_api_stream(state, backend.as_ref()).await;
            }
            Effect::ExecuteTool(call) => {
                spawn_tool_task(call, Arc::clone(executor), event_tx.clone());
            }
            Effect::DenyTool(tool_id) => {
                // Synthesize a denial ToolResult and deliver it through
                // update() immediately. Any new effects it produces
                // (likely SendMessage if this was the last outstanding
                // result) are appended to the queue so they process
                // in order.
                let event = denial_event(tool_id);
                let (new_state, new_effects) = update(state, event);
                state = new_state;
                queue.extend(new_effects);
            }
            Effect::Quit => {
                state.mode = Mode::Quitting;
                return state;
            }
        }
    }
    state
}

/// Drive a full API stream: emit `ApiStreamStart`, iterate backend
/// events mapping them to `AppEvent`s, and process each through
/// `update()` synchronously.
///
/// Keeping the whole stream inside a single `process_effects` step
/// matches how the `RunnerLoop` model treats `send_message` as one
/// action, and ensures no external events are interleaved with the
/// stream (which could otherwise confuse the state machine).
async fn run_api_stream(
    mut state: AppState,
    backend: &dyn ChatBackend,
) -> AppState {
    let (new_state, _) = update(state, AppEvent::ApiStreamStart);
    state = new_state;

    // Snapshot the messages. The backend stream borrows this slice
    // for its lifetime, so we can't mutate state while it's live.
    let messages: Vec<Message> = state.conversation.messages().to_vec();
    let mut stream = backend.send(&messages);

    while let Some(result) = stream.next().await {
        let event = match result {
            Ok(BackendEvent::TextDelta(text)) => AppEvent::ApiTextDelta(text),
            Ok(BackendEvent::ToolUse { id, name, input }) => {
                AppEvent::ApiToolUse { id, name, input }
            }
            Ok(BackendEvent::Done) => AppEvent::ApiDone,
            Err(err) => AppEvent::ApiError(err.message),
        };

        let (new_state, _effects) = update(state, event);
        state = new_state;
        // Stream events don't currently produce effects. If that
        // changes, this is where we'd need to process them — but
        // they'd need special handling because we're already inside
        // effect processing for the SendMessage that started this stream.
    }

    state
}

/// Spawn a tokio task that executes a tool and sends the result back
/// through the main event channel when done. Fire-and-forget.
fn spawn_tool_task(
    call: ToolCall,
    executor: Arc<dyn ToolExecutor>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) {
    tokio::spawn(async move {
        let result = executor.execute(&call).await;
        let event = AppEvent::ToolResult {
            tool_use_id: call.id,
            content: result.content,
            is_error: result.is_error,
        };
        let _ = event_tx.send(event);
    });
}

/// Build a synthetic `ToolResult` event for a denied tool. The
/// runner feeds this back through `update()` so the denial message
/// lands in the conversation before any follow-up `SendMessage`
/// executes.
fn denial_event(tool_id: ToolUseId) -> AppEvent {
    AppEvent::ToolResult {
        tool_use_id: tool_id,
        content: "Tool denied by user".to_string(),
        is_error: true,
    }
}
