---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Mission System

Cron-based scheduled agent work. A project can have **multiple active missions** — like a crontab with multiple entries.

## Related docs

- `agents.md`: agent types, lifecycle, delegation.
- `product-spec.md`: mission system overview, OS analogy.
- `storage.md`: mission JSON format, filesystem layout.

## Core concepts

A **mission** is a cron job:

| Field | Required | Description |
|:------|:---------|:------------|
| `id` | yes | Unique identifier (generated) |
| `schedule` | yes | Cron expression (5-field standard) |
| `prompt` | yes | The instruction sent to the agent |
| `model` | no | Model override for this mission |
| `enabled` | yes | Whether this mission is active |
| `created_at` | yes | Timestamp |

### Mission agent

All missions run the **`mission` agent** (`agents/mission.md`). This is not user-configurable — the UI shows "mission" as a readonly label.

The mission agent is designed for **autonomous execution** — no human in the loop:

- **No interactive tools**: no `AskUser`, `UpdatePlan`, `EnterPlanMode`.
- **Work tools**: `Read`, `Write`, `Edit`, `Bash`, `Glob`, `Grep`, `WebSearch`, `WebFetch`.
- **Delegation**: can use `Task` to spawn `coder`, `explorer`, `debugger` for heavy work.
- **Self-documenting**: the agent's final message serves as the run report.

Users can view the mission agent definition (readonly) in the mission UI.

### Cron syntax

Standard 5-field cron: `minute hour day-of-month month day-of-week`.

```
*/30 * * * *        → every 30 minutes
0 9 * * 1-5         → weekdays at 9am
0 0 * * 0           → every Sunday at midnight
0 */2 * * *         → every 2 hours
```

No seconds field. No `@reboot` or non-standard extensions.

## Multiple missions

A project has a **list of missions**, each independently scheduled. Like a crontab file with multiple entries:

```
# Mission 1: Architecture review — daily at 9am
0 9 * * *   "Review code changes and update dependency graphs"

# Mission 2: Disk cleanup — every Sunday
0 0 * * 0   "Analyze disk usage and suggest cleanup"

# Mission 3: Status check — every 30 minutes
*/30 * * * * "Check CI/CD status and report issues"
```

Each mission is independent — its own schedule, prompt, and optional model. Missions can be enabled/disabled individually.

## Session per run

Each mission trigger creates a **new session**. This is the key design difference from interactive chat — the session is the run log.

- **Session title**: `"Mission: {prompt_preview} — {timestamp}"` (prompt truncated to ~50 chars).
- **All tool calls, messages, observations** are recorded in the session — same as a user chat session.
- **Viewable in UI**: mission run sessions appear in the session list and can be opened read-only.
- **Run entry links to session**: `MissionRunEntry` includes `session_id` so the UI can navigate from run history to the session.

## Scheduler behavior

Background task evaluates all enabled missions against their cron schedules:

1. **Tick**: scheduler wakes periodically (every ~10s) and checks all enabled missions.
2. **Match**: for each mission whose cron expression matches the current time window, fire the prompt.
3. **Create session**: create a new session for this run.
4. **Spawn mission agent**: run the `mission` agent in that session with the mission prompt.
5. **Busy skip**: if the mission agent is already running, skip this trigger and log it.
6. **Run record**: each trigger creates a standard `AgentRunRecord` (see `agents.md`) + a `MissionRunEntry`.

### Deduplication

The scheduler tracks the last fire time per mission. A cron match only fires if the current minute differs from the last fire minute — prevents double-firing within the same tick window.

## Run history

Each mission trigger creates:
- A **session** containing the full conversation (tool calls, agent messages).
- An `AgentRunRecord` in `runs/` (standard format).
- A `mission_run` entry in `missions/{id}/runs.jsonl` linking the run to the mission and session.

```json
{ "run_id": "run-mission-1700000000-123456", "session_id": "sess-1700000000-abcd1234", "triggered_at": 1700000000, "status": "completed", "skipped": false }
```

Skipped triggers (agent busy) are also logged with `"skipped": true` and no `session_id`.

## Safety

| Guard | Value | Rationale |
|:------|:------|:----------|
| Minimum interval | 1 minute | Cron can't express sub-minute; prevents runaway |
| Max triggers per mission | 100 per day | Caps runaway missions |
| Max concurrent missions | No hard limit | Busy-skip naturally throttles |
| `max_iters` | Per agent config | Bounds each triggered run |
| No mission = no triggers | — | Missions must be explicitly created |
| Disabled missions | Skip silently | `enabled: false` stops all triggers |
| No interactive tools | — | Mission agent cannot block waiting for human input |

## Lifecycle

```
create → enabled → (triggers run on schedule, each run creates a session) → disabled → delete
```

- **Create**: user defines schedule + prompt via Web UI or API.
- **Enable/Disable**: toggle without deleting. Disabled missions keep their config and history.
- **Delete**: removes the mission. Run history and sessions are preserved.
- **Edit**: update schedule, prompt, or model. Takes effect on next tick.

## UI

### Mission editor

- **Schedule**: cron expression with presets.
- **Prompt**: the instruction text.
- **Model override**: optional.
- **Agent**: shown as readonly "mission" label — not editable.
- **View agent**: link/button to view `mission.md` content (readonly).

### Run history

Each run entry shows:
- Timestamp, status (completed/failed/skipped).
- Link to the session — opens the full conversation log.

## API operations

| Operation | Description |
|:----------|:------------|
| List missions | All missions for a project (with status, last run) |
| Create mission | New mission with schedule + prompt |
| Update mission | Edit schedule, prompt, model, or enabled flag |
| Delete mission | Remove mission (history preserved) |
| Mission runs | Run history for a specific mission (with session links) |

## Implementation

| Module | Responsibility |
|:-------|:---------------|
| `agents/mission.md` | Mission agent definition (autonomous, no AskUser) |
| `mission_scheduler.rs` | Cron evaluation, tick loop, session creation, trigger firing |
| `project_store/missions.rs` | Mission CRUD, run history persistence |
| `server/missions_api.rs` | HTTP endpoints for mission management |
