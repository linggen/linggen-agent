# Linggen-Agent Framework

Technical runtime design for `linggen-agent` (Rust): skill loading, agent management, model routing, tool contract, and safety rules.

## Related docs

- `doc/product-spec.md`: product goals, design principles, UX.
- `doc/multi-agents.md`: multi-agent runtime/events/API contract.

## Runtime shape

- CLI entry point:
  - `linggen` starts the backend server and TUI simultaneously.
  - WebUI and TUI connect to the same server and share session state.
  - Current: `linggen-agent agent` (TUI only), `linggen-agent serve` (server only). Target: unified `linggen` command.
- Workspace root:
  - If `--root` is passed, use it.
  - Otherwise, walk upward to find `.git`; if not found, use current directory.
- Interaction modes:
  - `chat` → `PromptMode::Chat` (human in the loop; Claude Code-style agentic chat)
  - `auto` → `PromptMode::Structured` (agents in the loop; autonomous JSON action loop)
- Iteration budget:
  - `agent.max_iters` in `linggen-agent.toml` is the loop budget used by both:
    - structured autonomous loop (`auto` / `PromptMode::Structured`)
    - chat-mode tool loop (`chat` / `PromptMode::Chat`)

## Skill loading

Skills are the primary extension mechanism. Format follows the [Agent Skills](https://agentskills.io) open standard, aligned with [Claude Code skills](https://code.claude.com/docs/en/skills).

- Each skill is a directory with `SKILL.md` as entrypoint plus optional supporting files.
- `SKILL.md` has YAML frontmatter and markdown instructions.

**Frontmatter fields**: `name`, `description`, `argument-hint`, `disable-model-invocation`, `user-invocable`, `allowed-tools`, `model`, `context`, `agent`, `trigger`.

**Discovery order** (higher priority wins):

| Level    | Path                                            | Scope                    |
|:---------|:------------------------------------------------|:-------------------------|
| Personal | `~/.linggen/skills/<name>/SKILL.md`             | All projects             |
| Project  | `.linggen/skills/<name>/SKILL.md`               | This project only        |
| Compat   | `~/.claude/skills/`, `~/.codex/skills/`         | Cross-tool compatibility |

- Skills are discovered at startup and on file change (live reload).
- Skill descriptions are loaded into agent context so the model knows what is available.
- Full skill content loads only when invoked (by user via `/name` or by model).
- Skills with `disable-model-invocation: true` are only invocable by the user.
- Skills with `user-invocable: false` are only invocable by the model.

## Trigger symbol parsing

The runtime parses trigger symbols from raw user input before sending to the model. Model responses are not parsed for triggers.

**System triggers** (reserved):
- `/` — built-in commands (`/help`, `/clear`, `/settings`) and skill invocation (`/deploy`, `/translate`).
- `@` — mentions. Routes to skills registered for the named target (e.g. `@tom hello` → social chat skill).

**User-defined triggers** — skills declare custom trigger prefixes in frontmatter (`trigger: "!!"`, `trigger: "%%"`, etc.).

**Matching order**: system triggers first, then user-defined triggers, then pass-through to model.

## Agent loading model

- Agent specs are markdown files under `agents/*.md` with YAML frontmatter.
- Runtime discovers these files dynamically at startup (no hardcoded agent list).
- Users can also create and manage agents via WebUI (create, edit, delete, assign skills/models).
- The framework ships with default agents (`ling`, `coder`). Users add more as needed.
- Each agent has its own context, skill set, and model preference.
- Agent behavior is read from markdown metadata and prompt body.
- All agents are equal — any agent can be chatted with directly or delegated to.
- Gate policy is read from markdown frontmatter `policy`.
  - Shorthand list: `policy: [Patch, Finalize, Delegate]`.
  - Object form for delegate target constraints:
    - `policy.allow`: `[Patch, Finalize, Delegate]`
    - `policy.delegate_targets`: `["coder"]`

**Agent frontmatter fields**: `name`, `description`, `tools`, `model`, `work_globs`, `policy`.

## Model routing

Users can configure multiple model providers: local (Ollama), OpenAI API, Claude API, AWS Bedrock.

**Built-in policies**:
- `local-first` — prefer local models, fall back to cloud.
- `cloud-first` — prefer cloud models, fall back to local.

**Custom policies**: Users define named policies with per-model priority and conditions in `linggen-agent.toml`:

```toml
[routing]
default_policy = "balanced"

[[routing.policies]]
name = "balanced"
rules = [
  { model = "qwen3:32b", priority = 1, max_complexity = "medium" },
  { model = "claude-sonnet-4-6", priority = 2 },
  { model = "claude-opus-4-6", priority = 3, min_complexity = "high" },
]
```

**Complexity signal**: estimated from prompt length, tool call depth, and skill metadata (`model` hint in skill frontmatter). Skills can declare `model: cloud` or `model: local` to influence routing.

**Per-agent model**: agents can specify a `model` in frontmatter to override the routing policy.

## Shared sessions

- The server owns all session state (DB-backed).
- WebUI and TUI connect to the same HTTP/SSE API.
- Chat messages from any client are broadcast to all connected clients in real-time via SSE.
- Agent switching (`/agent <name>`) and tab views (WebUI) operate within the shared session.

## Loop behavior

- Structured loop (`PromptMode: structured`):
  - Runs a JSON action loop (`tool` / `patch` / `finalize_task`) up to `agent.max_iters`.
- Chat loop (`PromptMode: chat`):
  - Runs an agentic loop: model output -> optional tool call -> observation -> model follow-up.
  - One tool call is accepted per model turn; chaining happens across turns.
  - Stops when model returns plain-text answer, on error, cancellation, or when `agent.max_iters` is reached.

## Core modules

- `engine`: prompt loop, tool execution, outcomes.
- `server`: chat/run APIs, SSE events, session/project routes, shared session broadcast.
- `agent_manager`: agent lifecycle, run records, cancellation, model routing.
- `skills`: skill discovery, loading, live reload, trigger matching.
- `db`: persistent project/session/chat/run state.

## Tool contract

Model-facing tools:

- `get_repo_info()`
- `Glob({ globs?, max_results? })`
- `Read({ path, max_bytes?, line_range? })`
- `Grep({ query, globs?, max_results? })`
- `Write({ path, content })`
- `Edit({ path, old_string, new_string, replace_all? })`
- `Bash({ cmd, timeout_ms? })`
- `capture_screenshot({ url, delay_ms? })`
- `lock_paths({ globs, ttl_ms? })`
- `unlock_paths({ tokens })`
- `delegate_to_agent({ target_agent_id, task })`

Notes:

- `Write` overwrites entire file content. `Edit` performs exact string replacement within a file.
- `Edit` accepts aliases for its fields: `old_string` aliases `old`/`old_text`/`oldText`/`search`/`from`; `new_string` aliases `new`/`new_text`/`newText`/`replace`/`to`.
- `Bash` does not accept `cwd`; it runs under workspace root.
- `Read`/`Write`/`Edit` accept aliases `file` / `filepath` for `path`.
- `Read` `line_range` is a two-element array with 1-based inclusive indexing: `[start, end]`.
- `lock_paths` / `unlock_paths` provide workspace-level file locking for multi-agent coordination.
- Tool calls are hard-gated per agent using the loaded agent spec (`spec.tools`).
  - Frontmatter wildcard `tools: ["*"]` means unrestricted tool access.
- Action gates are hard-enforced by policy:
  - `Patch` is required for `{"type":"patch",...}`
  - `Finalize` is required for `{"type":"finalize_task",...}`
  - `Delegate` is required for `delegate_to_agent(...)`
- `Glob` is used for filename/path discovery.
- `Grep` is used for grep-style content search.
- Runtime enforces canonical Claude-style tool names directly (`Read`, `Write`, `Edit`, `Bash`, `Glob`, `Grep`).

## Command safety (`Bash`)

- Commands are validated before execution.
- Disallowed patterns include `$(`, backticks, and newline injection.
- Shell separators are parsed (`|`, `;`, `&&`, `||`); each segment's first token must be allowlisted.
- Commands execute with timeout (default 30 s) and output capture.

## File safety

- Paths are sanitized to stay inside workspace root.
- Parent traversal (`..`) and invalid absolute paths are rejected.
- File listing/search use ignore-aware walking.

## Multi-agent delegation rules

- All agents are equal — any agent can chat directly or be delegated to.
- Delegation depth is configurable via `agent.max_delegation_depth` in `linggen-agent.toml` (default 2).
- Delegation is blocked when the current depth reaches the configured maximum.
- `delegate_to_agent` requires the `Delegate` policy capability on the calling agent.
- `policy.delegate_targets` constrains which agents the caller can delegate to.
- Run lifecycle is persisted (`running`, `completed`, `failed`, `cancelled`).
- Cancellation cascades through the run tree.

## Event integration

Server publishes SSE events consumed by WebUI and TUI:

- `StateUpdated`
- `Message { from, to, content }`
- `SubagentSpawned { parent_id, subagent_id, task }`
- `SubagentResult { parent_id, subagent_id, outcome }`
- `AgentStatus { agent_id, status, detail?, status_id?, lifecycle? }`
- `SettingsUpdated { project_root, mode }`
- `QueueUpdated { project_root, session_id, agent_id, items }`
- `ContextUsage { agent_id, stage, message_count, char_count, estimated_tokens, token_limit?, compressed, summary_count }`
- `Token { agent_id, token, done, thinking }`
- `Outcome { agent_id, outcome }`
- `ChangeReport { agent_id, files, truncated_count }`

## Run inspection APIs

Run and context inspection APIs used by WebUI and TUI:

- `GET /api/agent-runs?project_root=...&session_id=...`
- `GET /api/agent-children?run_id=...`
- `GET /api/agent-context?run_id=...&view=summary|raw`
- `POST /api/agent-cancel` with `{ run_id }`

Notes:

- `agent-context` returns run metadata + message summary; `view=raw` includes run-scoped messages.
- Run cancellation is tree-based (`parent + active descendants`).

## Design direction

- Agent framework — skills, agents, models managed from WebUI and TUI.
- Keep tool contracts explicit and constrained.
- Cross-tool skill compatibility (Agent Skills standard).
- Keep docs aligned with implemented behavior; mark planned features clearly.
