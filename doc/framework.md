# Linggen-Agent Framework

Technical runtime design for `linggen-agent` (Rust): workspace scoping, tool contract, and safety rules.

## Related docs

- `doc/product-spec.md`: product goals and UX.
- `doc/multi-agents.md`: multi-agent runtime/events/API contract.

## Runtime shape

- Binary/subcommands:
  - `linggen-agent agent` (interactive CLI)
  - `linggen-agent serve` (HTTP API + web UI)
- Workspace root:
  - If `--root` is passed, use it.
  - Otherwise, walk upward to find `.git`; if not found, use current directory.
- Interaction modes:
  - `chat`
  - `auto`

## Core modules

- `engine`: prompt loop, tool execution, outcomes.
- `server`: chat/run APIs, SSE events, session/project routes.
- `agent_manager`: agent lifecycle, run records, cancellation.
- `db`: persistent project/session/chat/run state.

## Tool contract (current)

Model-facing tools today:

- `get_repo_info()`
- `list_files({ globs?, max_results? })`
- `read_file({ path, max_bytes?, line_range? })`
- `search_rg({ query, globs?, max_results? })`
- `write_file({ path, content })`
- `run_command({ cmd, timeout_ms? })`
- `capture_screenshot({ url, delay_ms? })`
- `delegate_to_agent({ target_agent_id, task })`

Notes:

- `write_file` is supported in current runtime (not patch-only).
- `run_command` does not accept `cwd`; it runs under workspace root.
- `read_file`/`write_file` accept aliases `file` / `filepath` for `path`.
- Tool calls are hard-gated per agent using the loaded agent spec (`spec.tools`).
- Tool aliases are normalized to canonical names before allowlist enforcement (for example: `Read` -> `read_file`, `Grep` -> `search_rg`, `Bash` -> `run_command`).

## Command safety (`run_command`)

- Commands are validated before execution.
- Disallowed patterns include `$(`, backticks, and newline injection.
- Shell separators are parsed (`|`, `;`, `&&`, `||`); each segment's first token must be allowlisted.
- Commands execute with timeout and output capture.

## File safety

- Paths are sanitized to stay inside workspace root.
- Parent traversal (`..`) and invalid absolute paths are rejected.
- File listing/search use ignore-aware walking.

## Multi-agent delegation rules

- Main agents can delegate to main agents or spawn subagents.
- Subagents cannot spawn subagents.
- Run lifecycle is persisted (`running`, `completed`, `failed`, `cancelled`).
- Cancellation cascades through the run tree.

## Event integration

Server publishes SSE events used by UI and sync flows:

- `StateUpdated`
- `Message`
- `SubagentSpawned`
- `SubagentResult`
- `AgentStatus`
- `SettingsUpdated`
- `QueueUpdated`
- `Token`
- `Outcome`

## Run inspection APIs

Run and context inspection APIs used by the web UI:

- `GET /api/agent-runs?project_root=...&session_id=...`
- `GET /api/agent-children?run_id=...`
- `GET /api/agent-context?run_id=...&view=summary|raw`
- `POST /api/agent-cancel` with `{ run_id }`

Notes:

- `agent-context` returns run metadata + message summary; `view=raw` includes run-scoped messages.
- Run cancellation is tree-based (`parent + active descendants`).

## Design direction

- Keep local-first defaults.
- Keep tool contracts explicit and constrained.
- Keep docs aligned with implemented behavior; mark future features as planned.
