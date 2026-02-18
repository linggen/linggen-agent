<p align="center">
  <img src="logo.svg" width="120" alt="Linggen Agent" />
</p>

<h1 align="center">Linggen Agent</h1>

<p align="center">A local-first, skill-driven agent framework. Manage agents, skills, and models from a Web UI or terminal.</p>

Linggen Agent is the successor of [Linggen](https://github.com/linggen/linggen). It provides a multi-agent runtime where users can add agents and skills by dropping markdown files — no code changes needed.

## Features

- **Skills-first architecture** — add capabilities by creating a `SKILL.md` file. Skills follow the [Agent Skills](https://agentskills.io) open standard, compatible with Claude Code and Codex.
- **Multi-agent management** — create, configure, and switch between agents. Each agent has its own context, skill set, and model preference.
- **Multi-model routing** — connect local models (Ollama), OpenAI API, Claude API, or AWS Bedrock. Define routing policies like `local-first`, `cloud-first`, or custom priority rules.
- **Skills Marketplace** — search, install, and manage community skills from the built-in marketplace UI or via `/skill` chat command.
- **Web UI + TUI** — both interfaces connect to the same backend and share session state in real-time via SSE.
- **Two interaction modes** — `chat` (human-in-the-loop, Claude Code style) and `auto` (autonomous agent execution).
- **Workspace safety** — file operations are scoped to the workspace root. Bash commands are validated against an allowlist. Agent actions are policy-gated per agent.

## Quick Start

### Prerequisites

- Rust toolchain (1.75+)
- Node.js (18+) for the Web UI
- An LLM provider: [Ollama](https://ollama.com) for local models, or an OpenAI-compatible API key

### Build

```bash
# Backend
cargo build

# Web UI
cd ui && npm install && npm run build && cd ..
```

### Configure

Create `linggen-agent.toml` in the project root:

```toml
[[models]]
id = "local"
provider = "ollama"
url = "http://127.0.0.1:11434"
model = "qwen3:32b"
keep_alive = "20m"

# [[models]]
# id = "cloud"
# provider = "openai"
# url = "https://api.openai.com/v1"
# model = "gpt-4o"
# api_key = "sk-..."

[server]
port = 6666

[agent]
max_iters = 100

[[agents]]
id = "lead"
spec_path = "agents/lead.md"
model = "local"
```

Config search order: `$LINGGEN_CONFIG` env var, `./linggen-agent.toml`, `~/.config/linggen-agent/`, `~/.local/share/linggen-agent/`.

### Run

```bash
# Start the server (Web UI at http://localhost:6666)
cargo run -- serve

# Or start the interactive TUI
cargo run -- agent

# Dev mode (backend + Vite HMR)
cargo run -- serve --dev   # terminal 1
cd ui && npm run dev       # terminal 2
```

## Adding Agents

Drop a markdown file in `agents/` with YAML frontmatter:

```markdown
---
name: coder
description: Implementation agent that writes and edits code.
kind: main
tools: ["Read", "Write", "Edit", "Bash", "Glob", "Grep"]
policy: [Patch, Finalize]
---

You are a coding agent. Write clean, tested code.
```

Frontmatter fields: `name`, `description`, `tools`, `model`, `kind` (`main`/`subagent`), `work_globs`, `policy`.

The agent is available immediately on the next startup — no code changes needed.

## Adding Skills

Create a directory with a `SKILL.md` file:

```
.linggen/skills/my-skill/SKILL.md    # project-scoped
~/.linggen/skills/my-skill/SKILL.md  # global
```

```markdown
---
name: my-skill
description: Does something useful.
allowed-tools: [Bash, Read]
---

Instructions for the agent when this skill is invoked.
```

Invoke skills via `/my-skill` in chat, or the model invokes them automatically based on context.

### Skills Marketplace

Install community skills from the [marketplace](https://github.com/linggen/skills):

- **Web UI**: Settings > Skills > Marketplace tab — search, install, and uninstall with one click.
- **Chat**: `/skill find <query>`, `/skill add <name>`, `/skill delete <name>`, `/skill list`.

Skills are compatible across Linggen, Claude Code, and Codex (shared [Agent Skills](https://agentskills.io) standard).

## Architecture

```
linggen-agent
├── src/
│   ├── main.rs              # CLI entry (clap): `agent` and `serve` subcommands
│   ├── config.rs             # TOML config, model/agent spec parsing
│   ├── engine/               # Core agent loop, tool dispatch, action parsing
│   ├── server/               # Axum HTTP server, SSE events, REST API
│   ├── agent_manager/        # Agent lifecycle, run records, model routing
│   ├── skills/               # Skill discovery, loading, marketplace
│   ├── db/                   # Persistent state (redb key-value store)
│   └── check.rs              # Bash command safety validation
├── agents/                   # Agent spec markdown files
├── ui/                       # React 19 + Vite + Tailwind v4
└── linggen-agent.toml        # Configuration
```

### Tool Contract

Agents interact with the workspace through a fixed set of Claude Code-style tools:

| Tool | Description |
|---|---|
| `Read` | Read file contents (with optional line range) |
| `Write` | Write/overwrite file |
| `Edit` | Exact string replacement within a file |
| `Bash` | Execute shell commands (allowlisted, with timeout) |
| `Glob` | Find files by pattern |
| `Grep` | Search file contents by regex |
| `delegate_to_agent` | Delegate a task to a subagent |
| `capture_screenshot` | Take a screenshot of a URL |

### Multi-Agent Runtime

- **Main agents** are long-lived and can receive user tasks.
- **Subagents** are ephemeral workers spawned by main agents via `delegate_to_agent`.
- Delegation depth is fixed at 1: main agent can delegate to subagents, but subagents cannot spawn further subagents.
- All actions are policy-gated per agent: `Patch`, `Finalize`, `Delegate` capabilities are declared in frontmatter.
- Run lifecycle is persisted and cancellation cascades through the run tree.

### Real-time Events

The server publishes SSE events consumed by both Web UI and TUI:

`StateUpdated`, `Message`, `Token`, `AgentStatus`, `SubagentSpawned`, `SubagentResult`, `Outcome`, `ContextUsage`, `ChangeReport`, `QueueUpdated`, `SettingsUpdated`.

## API Endpoints

| Route | Method | Description |
|---|---|---|
| `/api/chat` | POST | Send a chat message |
| `/api/events` | GET | SSE event stream |
| `/api/agents` | GET | List agents |
| `/api/skills` | GET | List loaded skills |
| `/api/models` | GET | List configured models |
| `/api/projects` | GET/POST/DELETE | Manage projects |
| `/api/sessions` | GET/POST/DELETE | Manage sessions |
| `/api/settings` | GET/POST | Get/update settings |
| `/api/config` | GET/POST | Get/update server config |
| `/api/marketplace/search` | GET | Search marketplace skills |
| `/api/marketplace/list` | GET | List popular marketplace skills |
| `/api/marketplace/install` | POST | Install a marketplace skill |
| `/api/marketplace/uninstall` | DELETE | Uninstall a marketplace skill |
| `/api/agent-runs` | GET | List agent runs |
| `/api/agent-context` | GET | Inspect run context |
| `/api/agent-cancel` | POST | Cancel an active run |

## License

MIT
