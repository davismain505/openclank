# openclank — Claude TUI for illumos

## Context

Existing AI coding tools (Claude Code, OpenAI Codex, Gemini CLI, opencode) don't work or aren't stable on illumos. We need a Rust TUI chat client for the Anthropic API that compiles and runs on OmniOS. The #1 design priority is **testability** — every behavior must be verifiable without a real terminal or real API calls.

## Architecture: The Elm Architecture (TEA)

All app behavior lives in a pure `update(state, event) -> (state, effects)` function. Rendering is a pure function of state. Side effects are returned as values, never executed inline. This makes the entire app testable with no IO.

```
src/
  main.rs                -- CLI parsing, wiring, entry point
  lib.rs                 -- re-exports (enables cargo test against library)

  state/
    mod.rs
    app.rs               -- AppState, Mode, Viewport, StatusLine
    message.rs           -- Message, Role, Conversation

  event/
    mod.rs
    input.rs             -- AppEvent enum (Key, Resize, ApiChunk, ApiDone, ApiError, Tick)
    update.rs            -- fn update(AppState, AppEvent) -> (AppState, Vec<Effect>)
    effects.rs           -- Effect enum (SendMessage, Quit, etc.) — descriptors, never executed

  render/
    mod.rs
    layout.rs            -- top-level compositor: render_app(frame, state)
    chat_view.rs         -- message history panel
    input_view.rs        -- input box with cursor
    status_bar.rs        -- model name, streaming indicator, errors

  backend/
    mod.rs
    traits.rs            -- ChatBackend trait (returns BoxStream<BackendEvent>)
    anthropic.rs         -- Real API: reqwest + rustls, SSE parsing
    mock.rs              -- MockBackend: canned responses, configurable delays/errors

  headless/
    mod.rs
    frame_dump.rs        -- FrameSnapshot struct, snapshot_from_state() via TestBackend
    runner.rs            -- stdin JSON events in, stdout JSON snapshots out

  tui/
    mod.rs
    terminal.rs          -- crossterm setup/teardown
    runner.rs            -- interactive event loop

  config/
    mod.rs
    cli.rs               -- clap args: --headless, --mock, --model, --api-key
    settings.rs          -- ANTHROPIC_API_KEY from env/config file
```

## Key design decisions

### Pure update function (no IO)
```rust
pub fn update(state: AppState, event: AppEvent) -> (AppState, Vec<Effect>) { ... }
```
- All behavior is here. Easily unit-tested.
- `Effect` is an enum (`SendMessage`, `Quit`, `SetClipboard`) — returned as data, executed by the runner.

### ChatBackend trait with MockBackend
```rust
pub trait ChatBackend: Send + Sync {
    fn send<'a>(&'a self, messages: &'a [Message]) -> BoxStream<'a, Result<BackendEvent, BackendError>>;
}
```
- `MockBackend` returns canned responses with configurable chunk sizes, delays, and error injection.
- `MockBackend::single("Hello!")` for quick tests. `MockBackend::with_responses(vec![...])` for scenarios.
- `--mock` CLI flag uses MockBackend without API key.

### Headless mode (JSON-line protocol)
`openclank --headless --mock` reads events from stdin as JSON lines, writes `FrameSnapshot` to stdout as JSON lines after each event:
```json
// stdin:  {"Key":{"code":"Char","char":"h"}}
// stdout: {"chat_lines":[],"input_text":"h","input_cursor_col":1,"status_text":"Ready","mode":"Normal","frame_number":1}
```
This lets integration tests (or scripts) drive the full app and assert on structured output.

### Rendering via TestBackend + insta snapshots
Every render function is tested by: construct AppState → render to `TestBackend(80, 24)` → `insta::assert_snapshot!()`. Visual regressions are caught automatically.

## Testing strategy

### Layer 1: Unit tests (pure, instant, run anywhere)
- `update()` tests: key events produce correct state transitions and effects
- State construction helpers for readable test setup

### Layer 2: Render snapshot tests (TestBackend + insta)
- Each render function tested independently
- Full-frame snapshots for composed layouts
- `cargo insta review` to accept intentional changes

### Layer 3: Integration tests (MockBackend + TestBackend)
- `TestHarness` struct owns AppState + MockBackend + TestBackend
- `harness.send_events(&[...])` → runs update loop, processes effects against mock
- `harness.assert_snapshot("name")` → insta snapshot of current frame
- Full scenarios: type input → send → receive streamed mock response → verify final frame

### Layer 4: Headless mode tests
- Spawn `openclank --headless --mock` as a child process
- Write event JSON to stdin, read snapshot JSON from stdout
- Assert on structured FrameSnapshot fields

## Build & test workflow (macOS → illumos)

```bash
# Sync sources (excluding build artifacts)
rsync -az --exclude target/ --exclude .git/ ./ root@omnios-big.local:~/openclank/

# Build + test on OmniOS (must source zshrc for cargo)
ssh root@omnios-big.local 'source ~/.zshrc && cd ~/openclank && cargo test 2>&1'

# For release build
ssh root@omnios-big.local 'source ~/.zshrc && cd ~/openclank && cargo build --release 2>&1'
```

A `scripts/test-illumos.sh` helper will wrap this for one-command remote testing.

## Implementation order

### Phase 1: Pure core — `state/*`, `event/*`, unit tests
Define all types. Implement `update()`. Write exhaustive unit tests. **Zero IO, zero platform-specific code.** This runs on both macOS and illumos immediately.

### Phase 2: Rendering + snapshots — `render/*`, insta tests
Implement all render functions. TestBackend snapshot tests. Verify on illumos via SSH.

### Phase 3: Backend trait + mock — `backend/*`
Define ChatBackend trait. Implement MockBackend. Test in isolation.

### Phase 4: Integration test harness — `tests/integration_tests.rs`
TestHarness combining all pure layers. Full scenario tests. This is the crown jewel — end-to-end behavior verification in milliseconds.

### Phase 5: Headless mode — `headless/*`, `config/cli.rs`
JSON-line protocol. Spawned-process tests.

### Phase 6: Real TUI + real API — `tui/*`, `backend/anthropic.rs`, `main.rs`
crossterm terminal plumbing. Anthropic SSE client. Wire everything. This is the only phase with platform-specific code, and it's thin.

## Dependencies

```toml
[dependencies]
ratatui = "0.29"
crossterm = "0.28"
reqwest = { version = "0.12", features = ["rustls-tls", "stream"], default-features = false }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
futures = "0.3"
dirs = "6"
unicode-width = "0.2"
clap = { version = "4", features = ["derive"] }

[dev-dependencies]
insta = { version = "1", features = ["json"] }
```

## Verification

1. `cargo test` passes on macOS (all 4 test layers)
2. `scripts/test-illumos.sh` passes (rsync + cargo test on OmniOS)
3. `cargo build --release` succeeds on OmniOS
4. `openclank --headless --mock` responds correctly to piped JSON events
5. `openclank --mock` runs interactively on OmniOS terminal (manual smoke test, Phase 6 only)
