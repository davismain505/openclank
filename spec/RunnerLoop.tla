---------------------------- MODULE RunnerLoop ----------------------------
(***************************************************************************)
(* A TLA+ model of the runner's effect processing and event delivery loop. *)
(*                                                                         *)
(* Complementary to OpenClank.tla, which models the state machine          *)
(* invariants (alternating roles, no orphan results, etc.). This model     *)
(* focuses on the runner's responsibilities:                               *)
(*                                                                         *)
(*   - Effects from update() must be processed sequentially, in order      *)
(*   - Some effects produce new events (DenyTool -> ToolResult)            *)
(*   - Some effects trigger external streams (SendMessage -> API stream)   *)
(*   - External events can arrive between effect processing steps          *)
(*   - update() is never called concurrently                               *)
(*                                                                         *)
(* The state space is small because we don't model conversation content    *)
(* or message structure -- just the runner's queues and causal ordering.   *)
(***************************************************************************)

EXTENDS Integers, Sequences, FiniteSets

CONSTANTS
    ToolIds,        \* Set of possible tool IDs (e.g. {t1, t2})
    MaxRounds       \* Bound on API round-trips to keep state space finite

(***************************************************************************)
(* Effects that update() can return. These correspond to the Effect enum   *)
(* in src/event/effects.rs:                                                *)
(*                                                                         *)
(*   send_message  -- Effect::SendMessage: send the conversation to the    *)
(*                    Anthropic API. The runner calls backend.send() and    *)
(*                    feeds the resulting stream events back to update().   *)
(*                                                                         *)
(*   execute_tool  -- Effect::ExecuteTool(ToolCall): run an approved tool   *)
(*                    as a subprocess. The runner spawns it async and       *)
(*                    delivers the result as AppEvent::ToolResult later.    *)
(*                                                                         *)
(*   deny_tool     -- Effect::DenyTool(ToolUseId): the user rejected a     *)
(*                    tool call. The runner must synthesize a ToolResult    *)
(*                    event (is_error=true, "denied by user") and deliver   *)
(*                    it through update() so the denial message is added    *)
(*                    to the conversation.                                  *)
(*                                                                         *)
(*   quit          -- Effect::Quit: clean up the terminal and exit.        *)
(***************************************************************************)

VARIABLES
    effectQueue,
    \* Sequence of effects waiting to be processed. Populated by
    \* update() calls, drained one at a time by the runner.
    \* Each element is a record: [type |-> "send_message"] or
    \* [type |-> "execute_tool", tid |-> <id>], etc.

    apiStreaming,
    \* TRUE when an API stream is active. Between the runner calling
    \* backend.send() (in response to a send_message effect) and the
    \* stream producing its final "done" event.

    pendingDenials,
    \* Set of tool IDs whose deny_tool effect has been processed by
    \* the runner but whose synthetic ToolResult event has not yet
    \* been delivered through update(). These denial results must be
    \* in the conversation before any send_message executes.

    pendingExecutions,
    \* Set of tool IDs whose execute_tool effect has been processed
    \* (tool subprocess spawned) but whose ToolResult event has not
    \* yet been delivered through update().

    rounds,
    \* Number of completed API round-trips. Bounded by MaxRounds.

    streamEventsRemaining
    \* How many stream events the current API response will produce
    \* before the "done" event. Used to model the finite stream.

vars == <<effectQueue, apiStreaming, pendingDenials,
          pendingExecutions, rounds, streamEventsRemaining>>

(***************************************************************************)
(* INVARIANTS                                                              *)
(***************************************************************************)

(***************************************************************************)
(* When the runner is about to process a send_message effect, there must   *)
(* be no pending denial results. This ensures the conversation contains    *)
(* all denial messages before being sent to the API.                       *)
(*                                                                         *)
(* Why this matters: update() can return [DenyTool(t1), SendMessage] in    *)
(* the same effect list (when denying the last tool with none approved).   *)
(* The runner must process DenyTool first, deliver the synthetic           *)
(* ToolResult through update(), and only then process SendMessage.         *)
(***************************************************************************)
SendNeverSkipsDenials ==
    effectQueue # <<>> /\ Head(effectQueue).type = "send_message"
    => pendingDenials = {}

(***************************************************************************)
(* An API stream is only active after at least one send_message has been   *)
(* processed. Streams don't appear out of nowhere.                         *)
(***************************************************************************)
StreamOnlyAfterSend ==
    apiStreaming => rounds > 0

(***************************************************************************)
(* The effect queue is only non-empty when no external events are being    *)
(* processed. (Enforced structurally: all event delivery actions require   *)
(* effectQueue = <<>>.)                                                    *)
(***************************************************************************)
EffectsDrainBeforeEvents ==
    TRUE \* Structural -- verified by the action guards below.

Inv ==
    /\ SendNeverSkipsDenials
    /\ StreamOnlyAfterSend

(***************************************************************************)
(* INITIAL STATE                                                           *)
(***************************************************************************)

Init ==
    /\ effectQueue = <<>>
    /\ apiStreaming = FALSE
    /\ pendingDenials = {}
    /\ pendingExecutions = {}
    /\ rounds = 0
    /\ streamEventsRemaining = 0

(***************************************************************************)
(* RUNNER ACTIONS: Processing effects from the queue                       *)
(*                                                                         *)
(* The runner drains effects one at a time, in order. Each effect          *)
(* triggers a specific runner behavior.                                    *)
(***************************************************************************)

(***************************************************************************)
(* Process send_message: call backend.send(), which starts an API stream.  *)
(* Only valid when there are no pending denials -- the conversation must   *)
(* be complete before sending.                                             *)
(***************************************************************************)
RunnerProcessSendMessage ==
    /\ effectQueue # <<>>
    /\ Head(effectQueue).type = "send_message"
    /\ pendingDenials = {}
    /\ rounds < MaxRounds
    /\ effectQueue' = Tail(effectQueue)
    /\ apiStreaming' = TRUE
    /\ rounds' = rounds + 1
    /\ streamEventsRemaining' = 2
    /\ UNCHANGED <<pendingDenials, pendingExecutions>>

(***************************************************************************)
(* Process execute_tool: spawn the tool subprocess asynchronously.         *)
(* The result arrives later via RunnerDeliverToolResult.                   *)
(***************************************************************************)
RunnerProcessExecuteTool ==
    /\ effectQueue # <<>>
    /\ Head(effectQueue).type = "execute_tool"
    /\ LET tid == Head(effectQueue).tid
       IN pendingExecutions' = pendingExecutions \union {tid}
    /\ effectQueue' = Tail(effectQueue)
    /\ UNCHANGED <<apiStreaming, pendingDenials, rounds,
                    streamEventsRemaining>>

(***************************************************************************)
(* Process deny_tool: record the denial. The runner must then deliver      *)
(* a synthetic ToolResult event through update() (via                      *)
(* RunnerDeliverDenialResult below) before any send_message can execute.   *)
(***************************************************************************)
RunnerProcessDenyTool ==
    /\ effectQueue # <<>>
    /\ Head(effectQueue).type = "deny_tool"
    /\ LET tid == Head(effectQueue).tid
       IN pendingDenials' = pendingDenials \union {tid}
    /\ effectQueue' = Tail(effectQueue)
    /\ UNCHANGED <<apiStreaming, pendingExecutions, rounds,
                    streamEventsRemaining>>

(***************************************************************************)
(* Process quit: terminal. No further actions.                             *)
(***************************************************************************)
RunnerProcessQuit ==
    /\ effectQueue # <<>>
    /\ Head(effectQueue).type = "quit"
    /\ effectQueue' = Tail(effectQueue)
    /\ UNCHANGED <<apiStreaming, pendingDenials, pendingExecutions,
                    rounds, streamEventsRemaining>>

(***************************************************************************)
(* RUNNER ACTIONS: Delivering events to update()                           *)
(*                                                                         *)
(* These actions represent the runner calling update() with an event.      *)
(* They ALL require effectQueue = <<>> -- the effect queue must be fully   *)
(* drained before any new event is delivered. This ensures effects from    *)
(* one update() call are fully processed before the next call.             *)
(*                                                                         *)
(* After calling update(), the runner receives new effects, which are      *)
(* placed in effectQueue for processing.                                   *)
(***************************************************************************)

(***************************************************************************)
(* Deliver a synthetic denial ToolResult to update(). The runner           *)
(* constructs this from the DenyTool effect. update() adds the denial      *)
(* message to the conversation and may return [SendMessage] if this was    *)
(* the last outstanding tool result.                                       *)
(***************************************************************************)
RunnerDeliverDenialResult ==
    /\ effectQueue = <<>>
    /\ pendingDenials # {}
    /\ \E tid \in pendingDenials :
        /\ pendingDenials' = pendingDenials \ {tid}
        \* If this was the last pending result (no approved tools running
        \* either), update() returns [SendMessage] to continue the
        \* conversation.
        /\ IF pendingDenials' = {} /\ pendingExecutions = {}
           THEN effectQueue' = <<[type |-> "send_message"]>>
           ELSE effectQueue' = <<>>
    /\ UNCHANGED <<apiStreaming, pendingExecutions, rounds,
                    streamEventsRemaining>>

(***************************************************************************)
(* A tool subprocess completes and the runner delivers the result to       *)
(* update(). If this was the last outstanding result, update() returns     *)
(* [SendMessage].                                                          *)
(***************************************************************************)
RunnerDeliverToolResult ==
    /\ effectQueue = <<>>
    /\ pendingExecutions # {}
    /\ \E tid \in pendingExecutions :
        /\ pendingExecutions' = pendingExecutions \ {tid}
        /\ IF pendingExecutions' = {} /\ pendingDenials = {}
           THEN effectQueue' = <<[type |-> "send_message"]>>
           ELSE effectQueue' = <<>>
    /\ UNCHANGED <<apiStreaming, pendingDenials, rounds,
                    streamEventsRemaining>>

(***************************************************************************)
(* Deliver an API stream event to update(). The runner reads the next      *)
(* event from the active stream. When streamEventsRemaining hits 0, the    *)
(* stream ends (api_done). api_done may cause update() to enter            *)
(* ToolApproval, but those effects come from later user key events.        *)
(***************************************************************************)
RunnerDeliverApiEvent ==
    /\ effectQueue = <<>>
    /\ apiStreaming
    /\ streamEventsRemaining > 0
    /\ streamEventsRemaining' = streamEventsRemaining - 1
    /\ IF streamEventsRemaining' = 0
       THEN apiStreaming' = FALSE  \* Stream done.
       ELSE UNCHANGED apiStreaming
    /\ effectQueue' = <<>>  \* Mid-stream events produce no effects.
    /\ UNCHANGED <<pendingDenials, pendingExecutions, rounds>>

(***************************************************************************)
(* User sends a message. Only when idle: no active stream, no pending      *)
(* tool results. update() returns [SendMessage].                           *)
(***************************************************************************)
RunnerDeliverUserSend ==
    /\ effectQueue = <<>>
    /\ ~apiStreaming
    /\ pendingDenials = {}
    /\ pendingExecutions = {}
    /\ rounds < MaxRounds
    /\ effectQueue' = <<[type |-> "send_message"]>>
    /\ UNCHANGED <<apiStreaming, pendingDenials, pendingExecutions,
                    rounds, streamEventsRemaining>>

(***************************************************************************)
(* User approves a tool. update() returns [ExecuteTool(tid)].              *)
(***************************************************************************)
RunnerDeliverUserApprove ==
    /\ effectQueue = <<>>
    /\ ~apiStreaming
    /\ \E tid \in ToolIds :
        effectQueue' = <<[type |-> "execute_tool", tid |-> tid]>>
    /\ UNCHANGED <<apiStreaming, pendingDenials, pendingExecutions,
                    rounds, streamEventsRemaining>>

(***************************************************************************)
(* User denies a tool. update() returns only [DenyTool(tid)], never        *)
(* [DenyTool, SendMessage] in the same effect list. The SendMessage        *)
(* comes later: the runner processes DenyTool, delivers the synthetic      *)
(* ToolResult through update(), and *that* update() call returns           *)
(* [SendMessage] when all results are in. This ordering is critical —      *)
(* it ensures the denial message is in the conversation before send.       *)
(***************************************************************************)
RunnerDeliverUserDeny ==
    /\ effectQueue = <<>>
    /\ ~apiStreaming
    /\ \E tid \in ToolIds :
        effectQueue' = <<[type |-> "deny_tool", tid |-> tid]>>
    /\ UNCHANGED <<apiStreaming, pendingDenials, pendingExecutions,
                    rounds, streamEventsRemaining>>

(***************************************************************************)
(* NEXT STATE                                                              *)
(***************************************************************************)

Next ==
    \* Effect processing (runner drains the queue)
    \/ RunnerProcessSendMessage
    \/ RunnerProcessExecuteTool
    \/ RunnerProcessDenyTool
    \/ RunnerProcessQuit
    \* Event delivery (runner calls update() with an event)
    \/ RunnerDeliverDenialResult
    \/ RunnerDeliverToolResult
    \/ RunnerDeliverApiEvent
    \/ RunnerDeliverUserSend
    \/ RunnerDeliverUserApprove
    \/ RunnerDeliverUserDeny

Spec == Init /\ [][Next]_vars

THEOREM Spec => []Inv

=============================================================================
