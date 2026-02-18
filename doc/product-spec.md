# Product Spec: Linggen Agent

## Human Intent

Linggen Agent is an agent framework.  
User can manage their agents, skills on WebUI and TUI, from social chat skill to personal AI assistant agent.


### Summary

Linggen Agent is a skill-driven agent framework with two interaction modes: `chat` and `auto`.

Users manage agents and skills through WebUI and TUI. Each agent has its own context and skill set — from a social chat skill that sends messages via `@tom`, to a coding agent that writes and tests code. Skills follow the [Agent Skills](https://agentskills.io) open standard (aligned with Claude Code) and work across AI tools.

### Related docs

- `doc/framework.md`: runtime/tooling/safety design.
- `doc/multi-agents.md`: main/subagent runtime contract, events, and APIs.

---

## Design Principles

### 1. Skills-first architecture

Skills are the primary extension mechanism. The skill format aligns with [Claude Code skills](https://code.claude.com/docs/en/skills) and the Agent Skills open standard.

- Each skill is a directory with a `SKILL.md` entrypoint and optional supporting files (templates, scripts, examples).
- `SKILL.md` has YAML frontmatter (`name`, `description`, `allowed-tools`, `model`, `context`, etc.) and markdown instructions.
- Adding a skill = dropping a directory with `SKILL.md`. No code changes needed.

**Skill discovery paths** (higher priority wins):

| Level    | Path                                            | Scope                        |
|:---------|:------------------------------------------------|:-----------------------------|
| Personal | `~/.linggen/skills/<name>/SKILL.md`             | All projects                 |
| Project  | `.linggen/skills/<name>/SKILL.md`               | This project only            |
| Compat   | `~/.claude/skills/`, `~/.codex/skills/`         | Cross-tool compatibility     |

Linggen reads skills from its own paths first, and optionally discovers skills from Claude Code / Codex directories for cross-tool compatibility. This means a skill written once can be used by Linggen, Claude Code, and Codex.

**Linggen as a skill for other AI tools**: The existing `linggen` skill (in `~/.claude/skills/linggen/`) allows Claude Code, Cursor, or other AI agents to dispatch tasks to Linggen — cross-project search, prompt enhancement, indexed context. In the future, other AI tools can delegate work to Linggen to save tokens and leverage local models.

### 2. Multi-agent management

- The framework ships with a default agent (`jarvis`) but users can create, configure, and manage multiple agents.
- Each agent has its own context, skill set, and model preference.
- Users manage agents via WebUI (create, edit, delete, assign skills/models) or by dropping markdown files in `agents/*.md`.
- Switch agents via `/agent <name>` in chat, or use tab views in web UI to talk to multiple agents simultaneously.
- Agents range from general-purpose assistants to specialized workers (coding, social chat, research, etc.).
- Adding a new `agents/*.md` file registers a new agent. No code changes needed.

**Agent frontmatter fields**: `name`, `description`, `tools`, `model`, `kind` (`main`/`subagent`), `work_globs`, `policy`.

**Policy gates**: Agent actions (Patch, Finalize, Delegate) are configured per agent via frontmatter.

### 3. Unified CLI and shared sessions

- Single command `linggen` starts the backend server and enters the terminal UI simultaneously.
- Web UI and TUI connect to the same backend server and share session state.
- Chat messages sent via web UI appear in real-time on TUI, and vice versa.
- Implementation: both clients use the same HTTP/SSE API. The server owns sessions and broadcasts events to all connected clients.

### 4. Trigger symbols (hybrid: system-reserved + user-defined)

Trigger symbols are parsed from raw user input only — model responses render as markdown normally.

**System triggers** (reserved by the runtime):
- `/` — built-in commands (`/help`, `/clear`, `/settings`) and skill invocation (`/deploy`, `/translate`). Skills register as sub-commands.
- `@` — mentions. Routes to skills that handle the named target (e.g. `@tom hello` → social chat skill dispatches to Tom).

**User-defined triggers** — skills can declare custom trigger prefixes in frontmatter (e.g. `trigger: "!!"`, `trigger: "%%"`).

The runtime matches system triggers first, then user-defined triggers.

### 5. Multi-model routing with named policies

Users can add multiple models: local (Ollama), OpenAI API, Claude API, AWS Bedrock.

**Built-in policies**:
- `local-first` — prefer local models (Ollama), fall back to cloud when local is unavailable or insufficient.
- `cloud-first` — prefer cloud models, fall back to local.

**Custom policies**: Users define named policies with per-model priority and conditions.

```toml
# Example: linggen-agent.toml
[routing]
default_policy = "balanced"

[[routing.policies]]
name = "balanced"
rules = [
  { model = "qwen3:32b", priority = 1, max_complexity = "medium" },
  { model = "claude-sonnet-4-6", priority = 2 },
  { model = "claude-opus-4-6", priority = 3, min_complexity = "high" },
]

[[routing.policies]]
name = "local-only"
rules = [
  { model = "qwen3:32b", priority = 1 },
  { model = "llama3:70b", priority = 2 },
]
```

**Complexity signal**: estimated from prompt length, tool call depth, and skill metadata (`model` hint in skill frontmatter). Skills can declare `model: cloud` or `model: local` to influence routing.

### 6. Cross-tool skill ecosystem

Linggen is a framework that connects AI tools through shared skills:

- Skills written for Linggen work in Claude Code and Codex (shared Agent Skills standard).
- The `linggen` skill lets other AI agents (Claude Code, Cursor) dispatch tasks to Linggen for cross-project search, context lookup, or local-model execution.
- Users manage all their agents, skills, and models from one place (WebUI or TUI).

---

## Product Goals

- Agent framework — users manage agents, skills, and models from WebUI and TUI.
- Skills-first extensibility — add capabilities by dropping a `SKILL.md`.
- Multi-agent support — from social chat to coding to personal assistant.
- Unified CLI (`linggen`) that serves both TUI and web UI from a shared backend.
- Multi-model routing with named policies (local-first, cloud-first, custom).
- Cross-tool skill compatibility (Agent Skills standard).
- Keep execution safe through tool constraints, workspace boundaries, and auditability.

## Interaction Modes

- `chat`: human-in-the-loop mode. The user guides iteration and can intervene between steps.
  - Chat behavior is still agentic: tools can chain across turns until a final plain-text answer.
- `auto`: human-not-in-the-loop mode. Agents continue execution until completion, failure, or cancellation.
  - Both modes are bounded by `agent.max_iters` from config.

Mode controls response behavior; safety is enforced by policy/tool constraints.

## UX Surface

- **CLI**: `linggen` starts server + TUI. Web UI connects to the same server.
- **Web UI**:
  - Agent and skill management — create, configure, assign skills and models.
  - Session-based chat shared with TUI.
  - Agent tab views — talk to multiple agents at once.
  - Agent status (`model_loading`, `thinking`, `calling_tool`, `working`, `idle`).
  - Agent hierarchy and context inspection.
  - Per-agent run history, run pin/unpin, run timeline panel.
- **Agent switching**: `/agent <name>` in chat, or tab views in web UI.

## Safety Requirements

- Repo/workspace scoped file operations.
- Canonical tool contract uses Claude Code-style names (`Read`, `Write`, `Bash`, `Glob`, `Grep`).
- Allowlisted command execution via `Bash`.
- Persisted chat/run records for traceability.
- Cancellation support for active run trees.

## Non-goals (early stage)

- Multi-tenant hosted SaaS.
- Unbounded autonomous production deployment.
- Removing policy gates from tools.
