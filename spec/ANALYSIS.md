# TLA+ Models: OpenClank State Machine and Runner Loop

Two TLA+ models verify different aspects of openclank's behavior.
Together they cover the concerns that single-threaded unit tests
cannot: interleavings of concurrent events and the ordering of
effect processing.

## The Two Models

### `OpenClank.tla` — Coarse-grained state machine

Models behaviors at the granularity of complete user-facing
interactions: a user sends a message, the API streams a batch of
tool-use blocks, the user approves or denies the batch, results go
back to the API, the next response arrives. Actions like
`UserSendMessage`, `ApiDone`, and `ToolResult` are single atomic
steps in this model.

### `RunnerLoop.tla` — Fine-grained effect processing

Models behaviors inside what `OpenClank.tla` treats as atomic. The
runner dequeues an effect, acts on it, possibly produces a new event,
delivers it through `update()`, receives more effects, and continues.
The runner model breaks open those "atomic" interactions to check
that the sub-steps are correctly ordered.

## Why Two Models?

Both models are about behaviors over time — there's no real "what
vs when" separation in TLA+. The distinction is **granularity** and
**which invariants are expressible at that granularity**.

At `OpenClank.tla`'s granularity, you can express structural
invariants about the conversation (alternating roles, no orphan
results) because those are properties of the message sequence. You
*can't* express invariants like "denial results must be in the
conversation before `SendMessage` executes" because at this
granularity, `SendMessage` *is* the atomic action — there's no
"before" within it.

At `RunnerLoop.tla`'s granularity, you can express that ordering
invariant because effect processing is no longer atomic; it's a
sequence of steps the model can observe. But at this granularity,
tracking the full conversation structure would be overwhelming, so
the runner model abstracts it away.

Keeping them separate means each model uses the abstraction that
makes its invariants expressible, without paying the state space
cost of the other model's detail.

## Is This a Concurrent System?

Yes, but the concurrency is hidden in the runner (Phase 4). The
Elm Architecture makes the `update()` function single-threaded —
pure, with no opinions about timing — but the *effects* it produces
require concurrent execution:

| Actor        | Produces                                          | Consumes             |
|--------------|---------------------------------------------------|----------------------|
| User input   | `AppEvent::Key`, `AppEvent::Resize`              | update()             |
| API stream   | `ApiStreamStart`, `ApiTextDelta`, `ApiToolUse`,  | update()             |
|              | `ApiDone`, `ApiError`                             |                      |
| Tool exec    | `AppEvent::ToolResult`                            | update()             |
| Runner       | routes effects to actors, calls render            | effects from update()|

The runner must read from all three sources concurrently (tokio
`select!`), serialize events through `update()` one at a time, and
dispatch effects to the appropriate actor.

## OpenClank.tla Invariants

Five phases: `idle`, `streaming`, `approval`, `executing`, `quitting`.
The conversation grows as messages are appended. Tool calls flow
through `toolsInDraft` → `pendingApproval` → `approvedTools` →
resolved, with all results sent back in a single user message.

| Invariant                      | What it catches                               |
|--------------------------------|-----------------------------------------------|
| `ApprovalToolsExist`           | Tool approval for a nonexistent tool call     |
| `AlternatingRoles`             | Two consecutive assistant messages            |
| `NoOrphanResults`              | Tool result without a preceding tool use      |
| `DraftOnlyWhenStreaming`       | Draft leaking outside the streaming phase     |
| `ToolsInDraftOnlyWhenDrafting` | Accumulated tools without a draft             |
| `PendingOnlyWhenApproving`     | Pending approvals outside approval mode       |
| `ApprovedOnlyWhenActive`       | Approved tools outside active phases          |

**Bugs caught during development:** An early version of the code
silently dropped the second tool-use block when multiple arrived
in one response. The model's invariants would have caught this,
and the fix (accumulate in draft, finalize on `ApiDone`) is what
the current model describes. Later, the model caught a related bug
in approve/deny batching: the old code sent one tool's result
immediately while another was still executing. The fix — transition
to `executing` phase and wait for all approved tools — is also in
the current model, along with `AllDenied` for the edge case where
every tool was denied.

## RunnerLoop.tla Invariants

Models the runner's effect queue and event delivery without tracking
conversation content. Small state space — checks in under a second.

| Invariant                | What it catches                                    |
|--------------------------|----------------------------------------------------|
| `SendNeverSkipsDenials`  | `SendMessage` executing before denial results are in the conversation |
| `StreamOnlyAfterSend`    | API streams appearing without a preceding `SendMessage` |

**Bug caught:** When `update()` returned `[DenyTool, SendMessage]`
in the same effect list, the runner could process `SendMessage`
before the synthetic `ToolResult` from `DenyTool` had been delivered
through `update()` — sending an incomplete conversation. The fix:
`update()` never returns `SendMessage` alongside `DenyTool`; the
`ToolResult` handler emits `SendMessage` after all denial results
are in the conversation.

## What the Models Don't Capture

Intentional abstractions:

- **Tool input content** — The SSE `input_json_delta` bug was about
  content, not structure. Verified by unit tests instead.
- **Scroll state, cursor position** — Numerical/visual, not
  state-machine concerns.
- **Draft content** — Models track draft existence, not accumulated
  text. Text delta bugs are caught by unit tests.
- **Tool ID matching between results and uses** — Models track that
  a tool_result follows some tool_use, not that IDs match. ID-swap
  bugs would need a richer model.

## Running the Models

```
make check
```

Downloads `tla2tools.jar` if needed, then runs TLC against both
models. CI uses small bounds (`MaxSends=1`, one tool ID) to keep
runs fast — both models complete in under a second combined.

## Design Implications

The models together reveal the architecture's core invariants:

1. **Sequential update()** — The runner must deliver events to
   `update()` one at a time, never concurrently. Effects from one
   call must be fully processed before the next external event is
   delivered.

2. **Effect ordering matters** — Some effects produce new events
   that must flow through `update()` before later effects execute.
   The runner cannot reorder or batch-process effects arbitrarily.

3. **Tool batching** — All tools in an API response must be resolved
   before any results are sent. The Anthropic API requires all tool
   results for a turn in a single user message.

4. **User quit is privileged** — `UserQuit` is available from any
   non-terminal phase. The user should never be trapped by a stuck
   stream or pending tool.
