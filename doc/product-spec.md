# Product Spec: Linggen Agent

## Vision

Linggen Agent is an **AI operating system**. The core runtime manages agent processes, communication, and execution — everything else is userspace, built on top as skills.

Users manage agents, skills, and models through WebUI and TUI. Each agent has its own context and skill set — from a social chat skill to a coding agent. Skills follow the [Agent Skills](https://agentskills.io) open standard (aligned with Claude Code) and work across AI tools.

### OS analogy

| OS Concept | Linggen Equivalent |
|:-----------|:-------------------|
| Process | Agentic loop — one running agent |
| Interrupt | User message queue — checked each iteration, model decides |
| Thread/Fork | Subagent delegation — concurrent child execution |
| Syscall | Tool call — built-in tools are the kernel API |
| Dynamic library | Skill — loaded at runtime, no code changes |
| Shell/scripting | Code execution (PTC) — model outputs code, engine runs it |
| Signal | Cancel, pause — delivered via message queue |
| IPC | Event bus + delegation for inter-agent communication |

### Related docs

| Doc | OS Layer |
|:----|:---------|
| `agentic-loop.md` | Kernel — loop, interrupts, PTC, cancellation |
| `agents.md` | Process management — lifecycle, delegation, scheduling |
| `skills.md` | Dynamic extensions — format, discovery, triggers |
| `tools.md` | Syscall interface — built-in tools, safety |
| `chat-spec.md` | Chat system — SSE events, message model, rendering, APIs |
| `models.md` | Hardware abstraction — providers, routing |
| `cli.md` | Shell — CLI reference |

---

## Design Principles

### 1. Skills-first architecture

Skills are the primary extension mechanism. Built-in tools only for core operations needing safety/state/performance. Everything else (web search, APIs, git workflows) is a skill.

- Each skill is a directory with `SKILL.md` entrypoint and optional supporting files.
- Adding a skill = dropping a directory. No code changes needed.
- Skill format aligns with Claude Code and the Agent Skills open standard.

### 2. Multi-agent management

- Framework ships with default agents; users create more by dropping `agents/*.md` files.
- Each agent has its own context, skill set, model preference, and policy.
- Agents range from general-purpose to specialized (coding, social chat, research).
- Switch via `/agent <name>` or tab views in web UI.

### 3. Unified CLI and shared sessions

- Single command `linggen` starts backend server + TUI.
- Web UI and TUI share session state via same HTTP/SSE API.
- Chat messages from any client appear in real-time on all clients.

### 4. Trigger symbols

Parsed from user input only (not model output):

- `/` — built-in commands and skill invocation.
- `@` — mentions, routed to skills for the named target.
- User-defined triggers via skill frontmatter.

### 5. Multi-model routing

Users configure multiple providers (Ollama, OpenAI, Claude, Bedrock). Named routing policies (`local-first`, `cloud-first`, custom) control which model handles each request. See `models.md`.

### 6. Cross-tool skill ecosystem

- Skills written for Linggen work in Claude Code and Codex (shared standard).
- The `linggen` skill lets other AI tools dispatch tasks to Linggen.
- Users manage all agents, skills, and models from one place.

---

## Product Goals

- **Agent OS** — robust core runtime (agentic loop, message queue, code execution, agent lifecycle, event bus).
- **Skills-first** — add capabilities by dropping a `SKILL.md`. Built-in tools only for core ops.
- **User interrupt** — users can message a running agent; model sees it and adapts.
- **Code execution (PTC)** — model outputs code, engine executes it. Works with any model.
- **Multi-agent concurrency** — multiple agents running simultaneously.
- **Unified CLI** — serves both TUI and web UI from shared backend.
- **Multi-model routing** — named policies (local-first, cloud-first, custom).
- **Cross-tool compatibility** — Agent Skills standard.

## Mission System

Mission-based autonomy instead of explicit mode switching.

- **No mission** → agents are purely reactive (human-in-the-loop).
- **Active mission** → agents with `idle_prompt` self-initiate work periodically.

A mission is a project-level goal. Each mission defines text + per-agent idle configuration. See `agents.md` for idle scheduler details.

## UX Surface

- **CLI**: `linggen` starts server + TUI.
- **Web UI**: agent/skill management, session chat, agent tab views, mission page, settings, memory.
- **Agent switching**: `/agent <name>` or tab views.

## Safety Requirements

- Workspace-scoped file operations.
- Allowlisted command execution.
- Persisted chat/run records.
- Cancellation cascades through run trees.
- See `tools.md` for details.

## Non-goals (early stage)

- Multi-tenant hosted SaaS.
- Unbounded autonomous production deployment.
- Container/VM sandboxing (local trust model).
