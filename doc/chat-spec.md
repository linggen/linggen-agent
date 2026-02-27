# Chat System

Message system design: SSE event bus, chat message model, rendering pipeline, message queue, and API contract.

## Related docs

- `agentic-loop.md`: how interrupts work in the loop.
- `agents.md`: agent lifecycle events.
- `tools.md`: tool definitions and safety.

## Architecture overview

```
┌──────────────────────────────────────────────────────────────────┐
│  Rust Backend (src/server/)                                      │
│                                                                  │
│  Engine loop → ServerEvent enum → map_server_event_to_ui_message │
│                                    ↓                             │
│                              UiSseMessage (JSON)                 │
│                                    ↓                             │
│                           GET /api/events (SSE)                  │
└──────────────────────────────┬───────────────────────────────────┘
                               │
                    EventSource connection
                               │
┌──────────────────────────────▼───────────────────────────────────┐
│  Web UI (ui/src/)                                                │
│                                                                  │
│  useSseConnection → sseEventHandlers → chatReducer → ChatPanel   │
│                     (dispatch by kind)   (state)     (render)    │
└──────────────────────────────────────────────────────────────────┘
```

## SSE event bus

`GET /api/events` streams real-time events to all connected clients (WebUI + TUI).

Events are wrapped with increasing `seq` for dedup/recovery.

### ServerEvent types

| ServerEvent | Purpose |
|:------|:--------|
| `Token` | Streaming token from model (agent_id, token, done, thinking) |
| `Message` | Chat message (from, to, content) |
| `AgentStatus` | Status change (agent_id, status, detail, lifecycle) |
| `SubagentSpawned` | Child agent created (parent_id, subagent_id, task) |
| `SubagentResult` | Child agent finished (parent_id, subagent_id, outcome) |
| `QueueUpdated` | Message queue changed (project_root, session_id, agent_id, items) |
| `Outcome` | Agent run outcome (agent_id, outcome) |
| `ContextUsage` | Context window stats (message_count, estimated_tokens, compressed) |
| `ChangeReport` | Files changed (agent_id, files, truncated_count) |
| `StateUpdated` | General state refresh |
| `PlanUpdate` | Plan created/updated (agent_id, plan) |
| `IdlePromptTriggered` | Idle scheduler fired (agent_id, project_root) |
| `TextSegment` | Finalized text segment from agent (agent_id, text) |
| `AskUser` | Agent asking user a question (agent_id, question_id, questions) |
| `ModelFallback` | Model switched due to error/rate-limit (agent_id, preferred, actual, reason) |
| `ToolProgress` | Streaming tool output line (agent_id, tool, line, stream) |
| `Resync` | Broadcast lag recovery signal (reason, lagged_count) |

### Wire format: UiSseMessage

All events are serialized to a common wire format:

```typescript
interface UiSseMessage {
  id: string;       // event ID
  seq: number;      // monotonic sequence for dedup
  rev: number;      // revision
  ts_ms: number;    // timestamp millis
  kind: string;     // event category (see mapping below)
  phase?: string;   // sub-category within kind
  text?: string;    // text payload
  agent_id?: string;
  session_id?: string;
  project_root?: string;
  data?: any;       // structured payload
}
```

### Kind/phase mapping

`ServerEvent` → `UiSseMessage` mapping used by both Web UI and TUI:

| ServerEvent | kind | phase | Web UI handler | TUI handler |
|:---|:---|:---|:---|:---|
| `StateUpdated` | `run` | `sync` | `handleRun` → full refetch | `trigger_resync()` |
| `Outcome` | `run` | `outcome` | `handleRun` → full refetch | `trigger_resync()` + finalize |
| `Resync` | `run` | `resync` | `handleRun` → full refetch | `trigger_resync()` |
| `ContextUsage` | `run` | `context_usage` | `handleRun` → update context | update `last_context_tokens` |
| `SubagentSpawned` | `run` | `subagent_spawned` | `handleRun` → add tree entry | add to `active_subagents` |
| `SubagentResult` | `run` | `subagent_result` | `handleRun` → mark done | mark done, finalize group |
| `PlanUpdate` | `run` | `plan_update` | `handleRun` → set plan | render plan block |
| `ChangeReport` | `run` | `change_report` | `handleRun` → refetch files | render change report |
| `Message` | `message` | — | `handleMessage` | dedup + display |
| `AgentStatus` | `activity` | `doing`/`done` | `handleActivity` | update status bar / tool groups |
| `IdlePromptTriggered` | `activity` | `doing` | `handleActivity` | — |
| `QueueUpdated` | `queue` | — | `handleQueue` | — |
| `Token` | `token` | —/`done` | `handleToken` | streaming buffer |
| `TextSegment` | `text_segment` | — | `handleTextSegment` | push agent message |
| `AskUser` | `ask_user` | — | `handleAskUser` | interactive prompt |
| `ModelFallback` | `model_fallback` | — | `handleModelFallback` | system message |
| `ToolProgress` | `tool_progress` | — | no-op | update `status_tool` |

### Agent status lifecycle

`model_loading` → `thinking` → `calling_tool` → `working` → `idle`

## ChatMessage model

The frontend `ChatMessage` is the core data structure for rendered messages:

```typescript
interface ChatMessage {
  role: 'user' | 'agent';
  from?: string;              // agent ID
  to?: string;                // target agent ID
  text: string;               // primary text content
  timestamp: string;
  timestampMs?: number;

  // Lifecycle
  isGenerating?: boolean;     // true while agent is producing this message
  isThinking?: boolean;       // true during thinking phase

  // Activity (tool use)
  activitySummary?: string;   // headline: "Reading file.ts"
  activityEntries?: string[]; // tool call entries: "Read file.ts", "Edit main.rs"
  toolCount?: number;         // total tool calls in this turn

  // Context
  contextTokens?: number;
  messageCount?: number;
  durationMs?: number;

  // Rich content
  images?: string[];
  subagentTree?: SubagentTreeEntry[];
  content?: ContentBlock[];   // structured content blocks (target model)

  // Legacy (being phased out)
  segments?: MessageSegment[];
  liveText?: string;
}
```

### Message phases

The frontend derives a rendering phase from ChatMessage state:

```
getMessagePhase(msg) → 'thinking' | 'working' | 'streaming' | 'done'
```

| Phase | Condition | Rendering |
|:------|:----------|:----------|
| `thinking` | `isGenerating && isThinking` | Animated ThinkingIndicator |
| `working` | `isGenerating && has non-transient activity` | GroupedToolActivity (live) |
| `streaming` | `isGenerating && no activity` | Markdown text + cursor |
| `done` | `!isGenerating` | Collapsible summary + final text |

Transient statuses filtered out: "Thinking...", "Model loading...", "Running".

## Rendering pipeline

```
ChatMessage
  → AgentMessage component
    → SubagentTreeView (if delegation)
    → ActivitySection (tool calls: grouped, collapsible)
    → MessageBody (text: thinking indicator, special blocks, markdown)
```

### Special block types

MessageBody checks for JSON-structured content and renders specialized widgets:

| JSON `type` | Component | Purpose |
|:------------|:----------|:--------|
| `plan` | PlanBlock | Interactive plan approval UI |
| `finalize_task` | FinalizeTaskBlock | Task completion summary |
| `change_report` | ChangeReportBlock | File changes summary |
| `ask_user` | AskUserCard | Agent question with options |

Non-JSON text renders as Markdown via `MarkdownContent`.

## Chat reducer

`useChatMessages` hook manages message state via a reducer with these action categories:

### Token streaming
- `APPEND_TOKEN` — append streaming token to current message
- `SET_THINKING` — toggle thinking state

### Message lifecycle
- `PUSH_MESSAGE` — add new message (user or agent)
- `FINALIZE_MESSAGE` — mark current message as done (isGenerating=false)
- `CLEAR_MESSAGES` — reset all messages

### Activity (tool calls)
- `SET_ACTIVITY` — update activity summary/entries on current message
- `ACTIVITY_DONE` — mark activity complete

### Run events
- `SET_CONTEXT_USAGE` — update token/message counts
- `ADD_SUBAGENT` — add entry to subagent tree
- `MARK_SUBAGENT_DONE` — mark subagent complete
- `SET_PLAN` — set plan data on message

### Special
- `SET_ASK_USER` — store pending AskUser question
- `SET_MODEL_FALLBACK` — record model fallback event
- `PUSH_TEXT_SEGMENT` — append finalized text segment

## Message queue

When a user sends a message to a busy agent:

1. Message is added to per-agent queue (`queued_chats` HashMap).
2. `QueueUpdated` SSE event emitted so UI can show queued state.
3. At next loop iteration, engine checks queue and injects into context.
4. Model sees the message and decides how to respond.

This enables the "AI interrupt" pattern — users can redirect, cancel, or query a running agent without waiting for it to finish.

**Implementation**: `server/chat_api.rs` → `queued_chats`

## Event flow: single agent turn

```
User sends message via POST /api/chat
  ↓
1. Server → Message event (kind: "message")
   UI: PUSH_MESSAGE (user msg) + PUSH_MESSAGE (empty agent msg, isGenerating=true)

2. Server → Token events (kind: "token", thinking=true)
   UI: SET_THINKING + APPEND_TOKEN → phase: "thinking"

3. Server → AgentStatus (kind: "activity", phase: "doing", detail: "Reading file.ts")
   UI: SET_ACTIVITY → phase: "working"

4. Server → Token events (kind: "token", thinking=false)
   UI: APPEND_TOKEN → phase: "streaming"

5. Server → TextSegment (kind: "text_segment")
   UI: PUSH_TEXT_SEGMENT (finalized text chunk)

6. Server → Outcome (kind: "run", phase: "outcome")
   UI: FINALIZE_MESSAGE → phase: "done"
```

## API contract

### Chat

- `POST /api/chat` — send message to agent (queues if busy).

### Agent runs

- `GET /api/agent-runs?project_root=...&session_id=...` — list runs.
- `GET /api/agent-children?run_id=...` — list child runs.
- `GET /api/agent-context?run_id=...&view=summary|raw` — run context/messages.
- `POST /api/agent-cancel` with `{ run_id }` — cancel run tree.
- `POST /api/ask-user-response` with `{ question_id, answers }` — respond to AskUser question.

### Sessions & projects

- `GET/POST /api/settings` — configuration.
- `GET /api/workspace/tree` — file tree.

### Missions

- `GET /api/mission?project_root=...` — active mission.
- `GET /api/missions?project_root=...` — mission history.
- `POST /api/mission` — set mission.
- `DELETE /api/mission` — clear mission.
- `GET /api/agent-override?project_root=...&agent_id=...` — agent override.
- `POST /api/agent-override` — set agent override.

### Events

- `GET /api/events` — SSE stream.
- `GET /api/health` — health check (`{"ok": true}`).
