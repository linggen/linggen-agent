# Multi-Agent Runtime Spec

Current runtime contract for main agents, subagents, and UI-facing events/APIs.

## Related docs

- `doc/product-spec.md`: product goals and mode behavior (`chat` / `auto`).
- `doc/framework.md`: tool/router/safety implementation details.

## Agent taxonomy

- Main agents:
  - Long-lived, can receive user tasks, can message other main agents.
  - Current examples: `lead`, `coder`.
  - Planned: `operator`.
- Subagents:
  - Ephemeral child workers owned by one main agent.
  - Typical IDs: `search`, `plan`, `review`.

## Hard invariants

1. `main -> main` messaging is allowed.
2. `main -> subagent` delegation is allowed.
3. `subagent -> parent main` return is allowed.
4. `subagent -> subagent` is denied.
5. `subagent -> spawn(*)` is denied.
6. Delegation depth is fixed at 1.
7. Parent cancellation cascades to active children.

## Run model

Each execution is recorded as an `AgentRunRecord` with:

- `run_id`
- `repo_path`
- `session_id`
- `agent_id`
- `agent_kind` (`main` | `subagent`)
- `parent_run_id` (optional)
- `status` (`running` | `completed` | `failed` | `cancelled`)
- `detail` (optional)
- `started_at`, `ended_at`

## Live status model (UI)

The UI consumes `AgentStatus` events with these commonly used states:

- `model_loading`
- `thinking`
- `calling_tool`
- `working`
- `idle`

`detail` is free text (for example: "Model loading", "Calling read_file").

## SSE event contract (current)

`/api/events` emits:

- `StateUpdated`
- `Message`
- `SubagentSpawned`
- `SubagentResult`
- `AgentStatus`
- `SettingsUpdated`
- `QueueUpdated`
- `Token`
- `Outcome`

Events are wrapped with an increasing `seq` value to help UI dedupe/recovery.

## API contract (current)

- `GET /api/agent-runs?project_root=...&session_id=...`
- `GET /api/agent-children?run_id=...`
- `GET /api/agent-context?run_id=...&view=summary|raw`
- `POST /api/agent-cancel` with `{ run_id }`

Supporting routes used by the same UI flow:

- `POST /api/chat`
- `GET /api/events`
- `GET/POST /api/settings`
- `GET /api/workspace/tree`

## UI behavior guidance

- Keep one in-progress message per agent while streaming tokens/status.
- Append status/tool activity into that message instead of adding many transient bubbles.
- Render tool activity as a compact summary row with expandable details.
- On final assistant message, replace the temporary in-progress body but preserve activity summary.

## Near-term improvements

- Add richer per-run context slices (messages/tools/artifacts) without increasing chat noise.
- Extend status detail normalization for clearer human-readable phases.
