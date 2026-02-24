# Events

IPC: SSE event bus, message queue, inter-agent communication, and API contract.

## Related docs

- `agentic-loop.md`: how interrupts work in the loop.
- `agents.md`: agent lifecycle events.

## SSE event bus

`GET /api/events` streams real-time events to all connected clients (WebUI + TUI).

Events are wrapped with increasing `seq` for dedup/recovery.

### Event types

| Event | Purpose |
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
| `SettingsUpdated` | Config changed (project_root, mode) |
| `IdlePromptTriggered` | Idle scheduler fired (agent_id, project_root) |

### Agent status values

`model_loading` → `thinking` → `calling_tool` → `working` → `idle`

## Message queue

When a user sends a message to a busy agent:

1. Message is added to per-agent queue (`queued_chats` HashMap).
2. `QueueUpdated` SSE event emitted so UI can show queued state.
3. At next loop iteration, engine checks queue and injects into context.
4. Model sees the message and decides how to respond.

This enables the "AI interrupt" pattern — users can redirect, cancel, or query a running agent without waiting for it to finish.

**Implementation**: `server/chat_api.rs` → `queued_chats`

## API contract

### Chat

- `POST /api/chat` — send message to agent (queues if busy).

### Agent runs

- `GET /api/agent-runs?project_root=...&session_id=...` — list runs.
- `GET /api/agent-children?run_id=...` — list child runs.
- `GET /api/agent-context?run_id=...&view=summary|raw` — run context/messages.
- `POST /api/agent-cancel` with `{ run_id }` — cancel run tree.

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
