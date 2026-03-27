---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Storage

Filesystem layout for all persistent state. No database — everything is files.

## Related docs

- `agent-spec.md`: agent lifecycle, delegation.
- `mission-spec.md`: cron-based mission system.
- `agentic-loop.md`: plan persistence, context management.
- `chat-spec.md`: API endpoints that read/write this state.
- `skill-spec.md`: skill discovery paths.

## Root directories

Two-tier layout, aligned with Claude Code's `~/.claude/` + `{repo}/.claude/` convention.

| Directory | Purpose |
|-----------|---------|
| `~/.linggen/` | Global home (override with `$LINGGEN_HOME`) |
| `~/.linggen/sessions/` | All sessions (flat, creator/project tracked in metadata) |
| `~/.linggen/runs/` | All agent run records |
| `~/.linggen/memory/` | Global memory |
| `~/.linggen/missions/` | Mission definitions and run history |
| `~/.linggen/skills/{name}/` | Per-skill state |
| `{repo}/.linggen/` | Project-local settings (permissions) |

Project path encoding: `/Users/foo/project` → `-Users-foo-project` (same convention as Claude Code).

## Global state (`~/.linggen/`)

```
~/.linggen/
├── config/
│   └── linggen.runtime.toml    # Runtime config overrides (TOML)
├── logs/
│   └── linggen-{YYYY-MM-DD}.log  # Daily rolling logs (text)
├── agents/
│   └── {name}.md                     # Global agent specs (markdown + YAML frontmatter)
├── skills/
│   ├── {name}.md                     # Flat skill files
│   └── {name}/
│       ├── SKILL.md                  # Skill definition
│       └── scripts/                  # Skill assets (optional)
├── sessions/                         # All sessions (user, skill, mission — flat)
│   └── {session_id}/
│       ├── session.yaml              # Session metadata (includes creator, cwd, project)
│       └── messages.jsonl            # Chat messages, append-only (JSONL)
├── runs/                             # All agent run records
│   └── {run_id}.json
├── memory/                           # Global memory
│   └── ...
├── credentials.json                  # API keys for model providers (JSON)
├── permissions.json                  # Global tool permissions (home mode)
├── missions/
│   └── {mission_id}/
│       ├── mission.md                # Mission definition (markdown + YAML frontmatter)
│       └── runs.jsonl                # Mission run history (JSONL)
├── ling.pid                          # Daemon PID
└── ling.log                          # Daemon stdout
```

All sessions live in one flat directory. The `creator`, `project`, and `skill` fields in `session.yaml` provide the metadata for filtering, grouping, and display. No session files under `missions/` or `skills/` — those directories hold definitions only.

## Project-local state (`{repo}/.linggen/`)

Project-specific settings live inside the repo itself, not in `~/.linggen/`. Created when the agent grants project-scoped permissions.

```
{repo}/.linggen/
└── permissions.json                  # Project-scoped tool permissions (JSON)
```

Can be gitignored. Same pattern as Claude Code's `{repo}/.claude/settings.local.json`.


## Data formats

### Credentials (`credentials.json`)

```json
{
  "gemini-flash": { "api_key": "AIza..." },
  "groq-llama": { "api_key": "gsk_..." }
}
```

Stored at `~/.linggen/credentials.json`. Keyed by model `id` from `linggen.toml`. Never committed to git. See `models.md` → Credentials.

### Tool permissions (`permissions.json`)

```json
{ "tool_allows": ["Write", "Edit"] }
```

Two locations:
- `~/.linggen/permissions.json` — global permissions (home mode)
- `{repo}/.linggen/permissions.json` — project-scoped permissions (loaded when agent enters a git repo)

Created when user selects "Allow all {tool}". See `tool-spec.md` → Tool permission mode.

### Session metadata (`session.yaml`)

```yaml
id: sess-1700000000-abc12345
title: "Fix login bug"
created_at: 1700000000
creator: user
cwd: /Users/foo/workspace/myproject
project: /Users/foo/workspace/myproject
project_name: myproject
```

All sessions live at `~/.linggen/sessions/{id}/` — no path reconstruction needed. `cwd`, `project`, and `project_name` are updated dynamically as the agent changes directories. `project` and `project_name` are null when in home mode (no git repo detected).

### Chat messages (`messages.jsonl`)

One JSON object per line, append-only.

```json
{ "agent_id": "ling", "from_id": "user", "to_id": "ling", "content": "...", "timestamp": 1700000000, "is_observation": false }
```

### Agent run record (`{run_id}.json`)

```json
{
  "run_id": "run-ling-1700000000-123456",
  "repo_path": "/abs/path",
  "session_id": "sess-...",
  "agent_id": "ling",
  "agent_kind": "main",
  "parent_run_id": null,
  "status": "completed",
  "detail": "chat:structured-loop",
  "started_at": 1700000000,
  "ended_at": 1700000060
}
```

Status values: `running`, `completed`, `failed`, `cancelled`.

### Mission (`mission.md`)

Markdown file with YAML frontmatter. Stored at `~/.linggen/missions/{id}/mission.md`.

```markdown
---
schedule: '*/30 * * * *'
enabled: true
project: /path/to/project
permission_tier: standard
created_at: 1700000000
---

Check CI/CD status and report issues
```

Frontmatter fields: `schedule` (5-field cron), `enabled`, `model` (optional), `project` (optional), `permission_tier` ("readonly", "standard", "full"), `created_at`.

The markdown body is the mission prompt. Multiple missions can be active simultaneously — each in its own directory. See [`mission-spec.md`](mission-spec.md) for full details.

### Mission sessions

Mission sessions are stored in `~/.linggen/sessions/` alongside all other sessions. The `creator: mission` and `mission_id` fields in `session.yaml` identify them as mission-created. Mission definitions and run history remain in `~/.linggen/missions/{mission_id}/`.

### Mission run history (`runs.jsonl`)

```json
{ "run_id": "run-mission-1700000000-123456", "session_id": "sess-1700000000-abc12345", "triggered_at": 1700000000, "status": "completed", "skipped": false }
```

Append-only. Skipped triggers (agent busy) logged with `"skipped": true`.

### Plan messages (in `messages.jsonl`)

Plans are persisted as JSON messages inlined in the session's `messages.jsonl` — no separate plan files.

```json
{"type":"plan","plan":{"summary":"Refactor auth module","status":"planned","plan_text":"# Refactor auth module\n\n1. Read existing auth code\n2. Implement JWT validation\n3. Add tests","items":[]}}
```

Status values: `planned`, `approved`, `executing`, `completed`, `rejected`.
The UI renders these as PlanBlock components via `tryRenderSpecialBlock`.

## Configuration

Search order for `linggen.toml`:
1. `$LINGGEN_CONFIG` env var
2. `./linggen.toml` (working directory)
3. `~/.config/linggen/`
4. `~/.local/share/linggen/`

## Implementation

| Module | Responsibility |
|--------|---------------|
| `paths.rs` | All `~/.linggen/` path constants |
| `project_store/mod.rs` | Projects, agent overrides |
| `project_store/missions.rs` | Global mission store (CRUD, cron, run history) |
| `project_store/runs.rs` | Run records (CRUD) |
| `project_store/path_encoding.rs` | Path → directory name encoding |
| `state_fs/sessions.rs` | Sessions, chat messages (CRUD) |
| `state_fs/mod.rs` | Workspace-level state files |
| `engine/plan.rs` | Plan lifecycle (finalize, emit SSE) |
| `config.rs` | Config loading/saving |
| `logging.rs` | Log file rotation |

## Safety

- **Path traversal**: Session IDs and state file names reject `..`, `/`, `\\`.
- **Append-only messages**: Chat history uses JSONL — concurrent appends are safe.
- **No overwrites on history**: Missions and runs create new files; never overwrite old ones.
