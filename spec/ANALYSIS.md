# TLA+ Model: OpenClank State Machine

## Is This a Concurrent System?

Yes, but the concurrency is *hidden* in the runner (not yet implemented). The
design uses the Elm Architecture precisely to make the state machine
single-threaded — `update()` is pure and has no opinions about timing. But the
*effects* it produces require concurrent execution:

| Actor | Thread/Task | Produces | Consumes |
|-------|-------------|----------|----------|
| User input | crossterm event reader | `AppEvent::Key`, `AppEvent::Resize` | update() |
| API stream | reqwest async response | `ApiStreamStart`, `ApiTextDelta`, `ApiToolUse`, `ApiDone`, `ApiError` | update() |
| Tool executor | subprocess or async task | `AppEvent::ToolResult` | update() |
| Runner | main event loop | routes effects to actors, calls render | effects from update() |

The runner must:
1. Read from all three sources concurrently (tokio `select!` or similar)
2. Serialize events through `update()` — events are processed one at a time
3. Dispatch effects to the appropriate actor
4. Call render on each frame

The concurrency concern isn't *within* the state machine — it's in the
**interleaving of event delivery**. The TLA+ model explores all possible
orderings to find states where invariants break.

## What the Model Captures

### State Space

The model has 5 phases (`idle`, `streaming`, `approval`, `executing`,
`quitting`) and a conversation that grows up to `MaxMessages`. With the small
constants in `OpenClankTest.cfg` (2 tool IDs, 2 user texts, max 4 messages),
the state space is ~10K-100K states — small enough for exhaustive model
checking.

### Known Bugs Modeled

1. **`ApiSecondToolUse`** — This action explicitly models the bug from Phase
  1.1/1.2 #4. When a second tool-use arrives after the draft is already finalized
  (`conv.draft = "none"`), the code still enters `approval` mode with the new
  tool ID but doesn't add it to the conversation. The invariant
  `ApprovalToolExists` will detect this: the pending tool ID won't have a
  matching `tool_use` in any assistant message.

2. **`DraftOnlyWhenStreaming`** — Ensures that a draft is only present during
   the streaming phase. The model verifies this holds under all possible
   interleavings of events.

### Invariants Checked

| Invariant | What It Catches |
|-----------|-----------------|
| `ApprovalToolExists` | The "tool approval with missing tool call" bug |
| `AlternatingRoles` | Messages must alternate user/assistant |
| `NoOrphanResults` | No tool_result without a preceding tool_use |
| `DraftOnlyWhenStreaming` | Draft only exists during streaming |
| `Inv` | All of the above combined |

### What the Model Doesn't Capture (Yet)

- **Tool input content** — The model tracks *that* a tool use happened, not
  *what* the tool arguments were. The SSE `input_json_delta` bug (Phase 1.6) is
  about content, not structure.
- **Scroll state** — The scroll math issues are numerical, not
  state-machine-level.
- **Cursor position** — Same: a rendering concern, not a concurrency concern.
- **Multiple concurrent tool executions** — The current model has tools execute
  one at a time. If the design changes to allow parallel tool execution, the
  model should be extended with a set of in-flight tools rather than a single
  `toolResultsPending` set.

## What the Model Finds

Running `make check` produces:

1. **`ApprovalToolExists` violation** — Via the `ApiSecondToolUse` path: `idle` → `UserSendMessage` → `streaming` → `ApiSecondToolUse(t1)` → `approval` with `pendingTool = t1`, but `conv.messages` contains only the user message — no assistant message with `tool_use`. The invariant fails because the tool call was silently lost. This is exactly the bug found in Phase 1.1/1.2 #4.

2. **`AlternatingRoles` holds** — The state machine enforces alternation by construction: user sends → assistant responds → user sends tool result → assistant responds, etc. No interleaving can break this because only specific actions add messages with specific roles.

3. **`NoOrphanResults` holds** — Tool results are only added by the `ToolResult` action, which requires the tool ID to be in `toolResultsPending`, which is only populated after approval, which requires the tool use to be in the conversation.

## Design Implications

The model reveals that the current design has a **sequential bottleneck**: all events must be processed through a single `update()` call. This is intentional and correct for the Elm Architecture, but it means:

1. **The runner must deliver events to `update()` one at a time** — The Elm Architecture assumes this. The runner is responsible for ensuring it, e.g. by dispatching events from a single task or channel rather than calling `update()` from multiple concurrent tasks.

2. **Tool execution can be parallelized** — Multiple tools could run concurrently (each as a separate subprocess), but their results must be serialized back through `update()` one at a time. The current code only supports one tool at a time (single `Mode::ToolApproval` with one ID).

3. **The API stream and user input are independent** — The runner can read user input while waiting for API events. User quit (Ctrl+C) should be processed immediately regardless of streaming state. The model captures this with `UserQuit` being available in `streaming` and `approval` phases.
