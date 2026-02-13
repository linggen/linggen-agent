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
  - `chat` (human in the loop; basically Claude Code mode)
  - `auto` (agents in the loop; human not required during normal execution)
- Iteration budget:
  - `agent.max_iters` in `linggen-agent.toml` is the loop budget used by both:
    - structured autonomous loop (`auto` mode behavior)
    - chat-mode tool loop (`chat` mode behavior)

## Agent loading model

- Agent specs are source-of-truth markdown files under `agents/*.md`.
- Runtime should discover these files dynamically instead of using a hardcoded agent list.
- Adding a new agent markdown file should register a new agent (after reload/startup), without code or static config edits.
- Agent kind/behavior is read from markdown metadata and prompt body.
- Gate policy is read from markdown frontmatter `policy`.
  - Shorthand list is supported: `policy: [Patch, Finalize, Delegate]`.
  - Object form is supported for delegate target constraints:
    - `policy.allow`: `[Patch, Finalize, Delegate]`
    - `policy.delegate_targets`: `["search", "plan"]`

## Loop behavior (current)

- Structured loop (`PromptMode: structured`):
  - Runs a JSON action loop (`tool` / `patch` / `finalize_task`) up to `agent.max_iters`.
- Chat loop (`PromptMode: chat`):
  - Runs an agentic loop: model output -> optional tool call -> observation -> model follow-up.
  - One tool call is accepted per model turn; chaining happens across turns.
  - Stops when model returns plain-text answer, on error, cancellation, or when `agent.max_iters` is reached.

## Core modules

- `engine`: prompt loop, tool execution, outcomes.
- `server`: chat/run APIs, SSE events, session/project routes.
- `agent_manager`: agent lifecycle, run records, cancellation.
- `db`: persistent project/session/chat/run state.

## Tool contract (current)

Model-facing tools today:

- `get_repo_info()`
- `Glob({ globs?, max_results? })`
- `Read({ path, max_bytes?, line_range? })`
- `Grep({ query, globs?, max_results? })`
- `Write({ path, content })`
- `Bash({ cmd, timeout_ms? })`
- `capture_screenshot({ url, delay_ms? })`
- `delegate_to_agent({ target_agent_id, task })`

Notes:

- `Write` is supported in current runtime (not patch-only).
- `Bash` does not accept `cwd`; it runs under workspace root.
- `Read`/`Write` accept aliases `file` / `filepath` for `path`.
- Tool calls are hard-gated per agent using the loaded agent spec (`spec.tools`).
  - Frontmatter wildcard `tools: ["*"]` means unrestricted tool access.
- Action gates are hard-enforced by policy:
  - `Patch` is required for `{"type":"patch",...}`
  - `Finalize` is required for `{"type":"finalize_task",...}`
  - `Delegate` is required for `delegate_to_agent(...)`
- `Glob` is used for filename/path discovery.
- `Grep` is used for grep-style content search.
- Runtime enforces canonical Claude-style tool names directly (`Read`, `Write`, `Bash`, `Glob`, `Grep`).

## Command safety (`Bash`)

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
