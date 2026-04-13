---------------------------- MODULE OpenClank ----------------------------
(***************************************************************************)
(* A TLA+ model of the openclank state machine.                           *)
(*                                                                         *)
(* Four concurrent actors:                                                 *)
(*   1. User     — produces key events                                     *)
(*   2. API      — produces streaming events                               *)
(*   3. Executor — runs approved tools, produces results                   *)
(*   4. Runner   — event loop, dispatches through pure update()            *)
(*                                                                         *)
(* The model explores all possible interleavings to find states where       *)
(* invariants break.                                                       *)
(***************************************************************************)

EXTENDS Integers, Sequences, FiniteSets

CONSTANTS
    ToolUseIds,
    ToolNames,
    UserTexts,
    MaxSends

ASSUME MaxSends >= 0

\* ---- MESSAGE BOUND -------------------------------------------------------
\* Each send can produce at most 4 messages in the longest path:
\*   1. User message (from UserSendMessage)
\*   2. Assistant message with tool_use (from ApiDone)
\*   3. User message with tool_result (from ToolResult/AllDenied)
\*   4. Assistant text response (from the second ApiDone after tool result)
\* So Len(conv.messages) <= 4 * MaxSends is the tightest possible bound.

MaxMessages == 4 * MaxSends

\* ---- DATA TYPES ---------------------------------------------------------

BlockTypes == {"text", "tool_use", "tool_result"}

Role == {"user", "assistant"}

Message == [role : Role, content : SUBSET BlockTypes]

\* ---- STATE VARIABLES ----------------------------------------------------

VARIABLES
    phase,
    \*   "idle"       — waiting for user input
    \*   "streaming"  — API is sending a response
    \*   "approval"   — user is reviewing tool calls
    \*   "executing"  — approved tools are running
    \*   "quitting"   — shutting down

    conv,
    \* [messages |-> Seq(Message), draft |-> "none" | id]

    toolsInDraft,
    \* Tool IDs accumulated during streaming. Emptied when draft is
    \* finalized.

    pendingApproval,
    \* Set of tool IDs awaiting user approval. Non-empty only in
    \* "approval" phase. User resolves them one at a time.

    approvedTools,
    \* Set of tool IDs approved but not yet resolved. Emptied when
    \* all results are sent back to the API.

    lastSendCount,
    \* Monotonically increasing counter of SendMessage calls. Used to
    \* bound the state space.

    toolCallsMade,
    \* Set of tool IDs used in the current streaming turn.

    totalSends
    \* Total number of times UserSendMessage has been called.
    \* Used purely for state space bounding.

\* ---- HELPERS ------------------------------------------------------------

FreshToolId == CHOOSE t \in ToolUseIds : t \notin toolCallsMade

\* ---- TYPE INVARIANT -----------------------------------------------------

TypeInvariant ==
    /\ phase \in {"idle", "streaming", "approval", "executing", "quitting"}
    /\ conv.draft \in {"none"} \union ToolUseIds
    /\ toolsInDraft \subseteq ToolUseIds
    /\ pendingApproval \subseteq ToolUseIds
    /\ approvedTools \subseteq ToolUseIds
    /\ Len(conv.messages) <= MaxMessages

\* ---- STATE MACHINE INVARIANTS -------------------------------------------

(***************************************************************************)
(* 1. Every tool awaiting approval exists in the conversation.             *)
(***************************************************************************)
ApprovalToolsExist ==
    /\ phase = "approval"
    => /\ pendingApproval # {}
       /\ \A tid \in pendingApproval :
              \E i \in 1..Len(conv.messages) :
                  conv.messages[i].role = "assistant"
                  /\ "tool_use" \in conv.messages[i].content

(***************************************************************************)
(* 2. Messages alternate user/assistant, except that consecutive user       *)
(*    messages are allowed when an API error discarded the assistant        *)
(*    response. In the real app, a user can retry after an error.           *)
(***************************************************************************)
AlternatingRoles ==
    \A i \in 1..(Len(conv.messages) - 1) :
        ~(conv.messages[i].role = "assistant"
          /\ conv.messages[i+1].role = "assistant")

(***************************************************************************)
(* 3. No orphan tool results.                                              *)
(***************************************************************************)
NoOrphanResults ==
    \A i \in 1..Len(conv.messages) :
        conv.messages[i].role = "user"
        /\ "tool_result" \in conv.messages[i].content
        => \E j \in 1..(i-1) :
               conv.messages[j].role = "assistant"
               /\ "tool_use" \in conv.messages[j].content

(***************************************************************************)
(* 4. Draft only exists during streaming.                                  *)
(***************************************************************************)
DraftOnlyWhenStreaming ==
    conv.draft # "none" => phase = "streaming"

(***************************************************************************)
(* 5. toolsInDraft non-empty only when drafting.                           *)
(***************************************************************************)
ToolsInDraftOnlyWhenDrafting ==
    toolsInDraft # {} => conv.draft # "none"

(***************************************************************************)
(* 6. Pending approval non-empty only in approval phase.                   *)
(***************************************************************************)
PendingOnlyWhenApproving ==
    pendingApproval # {} => phase = "approval"

(***************************************************************************)
(* 7. Approved tools non-empty only during approval/executing.             *)
(***************************************************************************)
ApprovedOnlyWhenActive ==
    approvedTools # {} => phase \in {"approval", "executing"}

Inv ==
    /\ ApprovalToolsExist
    /\ AlternatingRoles
    /\ NoOrphanResults
    /\ DraftOnlyWhenStreaming
    /\ ToolsInDraftOnlyWhenDrafting
    /\ PendingOnlyWhenApproving
    /\ ApprovedOnlyWhenActive

\* ---- USER ACTIONS -------------------------------------------------------

UserSendMessage(t) ==
    /\ phase = "idle"
    /\ t # ""
    /\ totalSends < MaxSends
    /\ phase' = "streaming"
    /\ conv' = [conv EXCEPT !.messages = Append(conv.messages,
                     [role |-> "user", content |-> {"text"}])]
    /\ toolsInDraft' = {}
    /\ pendingApproval' = pendingApproval
    /\ approvedTools' = {}
    /\ lastSendCount' = Len(conv.messages) + 1
    /\ toolCallsMade' = {}
    /\ totalSends' = totalSends + 1

(***************************************************************************)
(* User approves one tool. Picks any from the pending set. If more remain,  *)
(* stays in approval. If queue empties, moves to executing.                *)
(***************************************************************************)
UserApprove ==
    /\ phase = "approval"
    /\ \E tid \in pendingApproval :
        /\ approvedTools' = approvedTools \union {tid}
        /\ pendingApproval' = pendingApproval \ {tid}
        /\ IF pendingApproval' = {}
           THEN phase' = "executing"
           ELSE phase' = "approval"
    /\ UNCHANGED <<conv, toolsInDraft, lastSendCount, toolCallsMade, totalSends>>

(***************************************************************************)
(* User denies one tool. The denial is recorded (the denied tool is NOT    *)
(* added to approvedTools). When the pending set empties, we transition    *)
(* to "executing" where the runner waits for approved tools to finish,     *)
(* then sends ALL results (both execution outputs and denial messages)     *)
(* in a single user message. This matches the Anthropic API requirement    *)
(* that all tool results for a turn are sent together.                     *)
(***************************************************************************)
UserDeny ==
    /\ phase = "approval"
    /\ \E tid \in pendingApproval :
        /\ pendingApproval' = pendingApproval \ {tid}
        /\ approvedTools' = approvedTools
        /\ IF pendingApproval' = {}
           THEN phase' = "executing"
           ELSE phase' = "approval"
    /\ UNCHANGED <<conv, toolsInDraft, lastSendCount, toolCallsMade, totalSends>>

UserQuit ==
    /\ phase \in {"idle", "streaming", "approval"}
    /\ phase' = "quitting"
    /\ conv' = [conv EXCEPT !.draft = "none"]
    /\ toolsInDraft' = {}
    /\ pendingApproval' = {}
    /\ approvedTools' = {}
    /\ UNCHANGED <<lastSendCount, toolCallsMade, totalSends>>

\* ---- API ACTIONS --------------------------------------------------------

ApiStreamStart ==
    /\ phase = "streaming"
    /\ conv.draft = "none"
    /\ conv' = [conv EXCEPT !.draft = FreshToolId]
    /\ UNCHANGED <<phase, toolsInDraft, pendingApproval,
                    approvedTools, lastSendCount, toolCallsMade, totalSends>>

(***************************************************************************)
(* Tool use arrives during streaming. Accumulates in toolsInDraft.          *)
(* Does NOT finalize the draft or enter approval.                          *)
(***************************************************************************)
ApiToolUse(tid) ==
    /\ phase = "streaming"
    /\ conv.draft # "none"
    /\ toolsInDraft' = toolsInDraft \union {tid}
    /\ toolCallsMade' = toolCallsMade \union {tid}
    /\ UNCHANGED <<phase, conv, pendingApproval,
                    approvedTools, lastSendCount, totalSends>>

(***************************************************************************)
(* Stream ends. Finalize the draft.                                         *)
(*                                                                         *)
(* If any tool uses accumulated, enter approval with the full set.         *)
(* The assistant message includes tool_use in its content.                 *)
(*                                                                         *)
(* If no tool uses, go to idle with a text-only assistant message.         *)
(***************************************************************************)
ApiDone ==
    /\ phase = "streaming"
    /\ IF toolsInDraft # {}
       THEN /\ phase' = "approval"
            /\ pendingApproval' = toolsInDraft
       ELSE /\ phase' = "idle"
            /\ pendingApproval' = pendingApproval
    /\ conv' = [conv EXCEPT
                    !.draft = "none",
                    !.messages = Append(conv.messages,
                        [role |-> "assistant",
                         content |-> IF toolsInDraft # {}
                                     THEN {"text", "tool_use"}
                                     ELSE {"text"}])]
    /\ toolsInDraft' = {}
    /\ UNCHANGED <<approvedTools, lastSendCount, toolCallsMade, totalSends>>

(***************************************************************************)
(* API stream errors. The draft is discarded (partial content lost) and    *)
(* the app returns to idle. No assistant message is added. This means the  *)
(* user can send another message, producing consecutive user messages —    *)
(* which is why AlternatingRoles only forbids consecutive *assistant*      *)
(* messages, not consecutive user messages.                                *)
(***************************************************************************)
ApiError ==
    /\ phase = "streaming"
    /\ conv' = [conv EXCEPT !.draft = "none"]
    /\ phase' = "idle"
    /\ toolsInDraft' = {}
    /\ UNCHANGED <<pendingApproval, approvedTools, lastSendCount,
                    toolCallsMade, totalSends>>

\* ---- EXECUTOR ACTIONS ---------------------------------------------------

(***************************************************************************)
(* An approved tool finishes executing. Its result is collected. When all   *)
(* approved tools have completed, all results (including denials) are      *)
(* sent to the API in a single user message and we return to streaming.   *)
(***************************************************************************)
ToolResult(tid) ==
    /\ phase = "executing"
    /\ tid \in approvedTools
    /\ approvedTools' = approvedTools \ {tid}
    /\ IF approvedTools' = {}
       THEN /\ totalSends < MaxSends
            /\ phase' = "streaming"
            /\ conv' = [conv EXCEPT
                            !.messages = Append(conv.messages,
                                [role |-> "user",
                                 content |-> {"tool_result"}])]
            /\ lastSendCount' = Len(conv.messages) + 1
            /\ toolCallsMade' = {}
            /\ totalSends' = totalSends + 1
       ELSE /\ UNCHANGED <<phase, conv, lastSendCount, toolCallsMade, totalSends>>
    /\ UNCHANGED <<toolsInDraft, pendingApproval>>

(***************************************************************************)
(* All tools were denied (none approved). We still need to send the        *)
(* denial results back to the API so the model knows the tools were        *)
(* rejected. This happens when we enter "executing" with an empty          *)
(* approvedTools set.                                                      *)
(***************************************************************************)
AllDenied ==
    /\ phase = "executing"
    /\ approvedTools = {}
    /\ totalSends < MaxSends
    /\ phase' = "streaming"
    /\ conv' = [conv EXCEPT
                    !.messages = Append(conv.messages,
                        [role |-> "user",
                         content |-> {"tool_result"}])]
    /\ lastSendCount' = Len(conv.messages) + 1
    /\ toolCallsMade' = {}
    /\ totalSends' = totalSends + 1
    /\ UNCHANGED <<toolsInDraft, pendingApproval, approvedTools>>

\* ---- NEXT STATE ---------------------------------------------------------

Next ==
    \/ \E t \in UserTexts : UserSendMessage(t)
    \/ UserApprove
    \/ UserDeny
    \/ UserQuit
    \/ ApiStreamStart
    \/ ApiDone
    \/ ApiError
    \/ \E tid \in ToolUseIds : ApiToolUse(tid)
    \/ \E tid \in ToolUseIds : ToolResult(tid)
    \/ AllDenied

\* ---- INIT ---------------------------------------------------------------

Init ==
    /\ phase = "idle"
    /\ conv = [messages |-> <<>>, draft |-> "none"]
    /\ toolsInDraft = {}
    /\ pendingApproval = {}
    /\ approvedTools = {}
    /\ lastSendCount = 0
    /\ toolCallsMade = {}
    /\ totalSends = 0

\* ---- SPEC ---------------------------------------------------------------

Spec == Init /\ [][Next]_<<phase, conv, toolsInDraft, pendingApproval,
                        approvedTools, lastSendCount, toolCallsMade,
                        totalSends>>

THEOREM Spec => []Inv

=============================================================================
