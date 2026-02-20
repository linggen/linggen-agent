# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Please read `.claude/skills/memory/SKILL.md` on load to understand the Linggen Memory skill and context management system.

## Doc and Spec

Read files under `doc/` and follow them. If you find wrong content in any doc file, confirm with the user.

- `doc/product-spec.md` — product goals, interaction modes, UX surface
- `doc/framework.md` — runtime design, tool contract, safety rules
- `doc/multi-agents.md` — multi-agent runtime, events, API contract
- `doc/code-style.md` — code style rules (flat logic, small files/functions, clean code)
- `doc/log-spec.md` — logging levels, throttling, output targets

## Build and Run

### Rust Backend

```bash
# Build
cargo build

# Run CLI interactive mode
cargo run -- agent

# Run web UI server (default port from linggen-agent.toml)
cargo run -- serve

# Run server in dev mode (proxies static assets from Vite)
cargo run -- serve --dev

# Run with specific workspace root
cargo run -- serve --root /path/to/project
```

### Web UI (React + Vite + Tailwind v4)

```bash
cd ui

# Install dependencies
npm install

# Dev server (proxies /api to backend port from linggen-agent.toml)
npm run dev

# Production build (output to ui/dist/, embedded by Rust via rust-embed)
npm run build

# Lint
npm run lint
npm run lint:fix
```

### Development Workflow

For full-stack dev, run both in parallel:
1. `cargo run -- serve --dev` (backend, serves API only)
2. `cd ui && npm run dev` (Vite dev server with HMR, proxies API to backend)

For production: `npm run build` in `ui/`, then `cargo run -- serve` (backend embeds `ui/dist/` via rust-embed).

## Architecture

Linggen Agent is a local-first, multi-agent coding assistant. Two entry points: `linggen-agent agent` (interactive CLI/REPL) and `linggen-agent serve` (HTTP API + web UI).

### Rust Backend (`src/`)

- **`main.rs`** — CLI entry point (clap). Parses subcommands `agent` and `serve`, loads config, sets up logging.
- **`config.rs`** — Config loading from `linggen-agent.toml` (TOML). Defines `Config`, `ModelConfig`, `AgentSpec` (parsed from markdown frontmatter), `AgentPolicy`.
- **`engine/`** — Core agent execution engine. Prompt loop, tool dispatch, structured/chat mode execution, action parsing, patch application, output rendering. `engine/tools.rs` implements all model-facing tools (`Read`, `Write`, `Edit`, `Bash`, `Glob`, `Grep`, `capture_screenshot`, `lock_paths`, `unlock_paths`, `delegate_to_agent`, `get_repo_info`).
- **`server/`** — Axum HTTP server. `mod.rs` sets up routes and static asset serving. `chat_api.rs` handles chat/run endpoints and SSE streaming. `chat_helpers.rs` has shared chat logic. `agent_api.rs` has run inspection APIs. `projects_api.rs` handles project/session CRUD. `workspace_api.rs` serves workspace file tree.
- **`agent_manager/`** — Agent lifecycle management, run records, cancellation, model routing. `models.rs` handles multi-model provider dispatch (Ollama, OpenAI-compatible).
- **`ollama.rs`** — Ollama API client (streaming and non-streaming chat completions).
- **`db/`** — Persistent state using redb (embedded key-value store). Projects, sessions, chat messages, run records.
- **`skills/`** — Skill manager for loading embedded and project-local skill definitions.
- **`state_fs/`** — Filesystem-backed project state (`.linggen-agent/` directory).
- **`repl.rs`** — Interactive CLI REPL mode (ratatui TUI).
- **`check.rs`** — Command safety validation for `Bash` tool (allowlist enforcement).
- **`workspace.rs`** — Workspace root detection (walks up to find `.git`).
- **`logging.rs`** — Tracing setup with file rotation and retention.

### Web UI (`ui/src/`)

React 19 + TypeScript + Tailwind CSS v4 + Vite. Key files:

- **`App.tsx`** — Main app component. Manages projects, sessions, agents, SSE event handling, settings.
- **`components/ChatPanel.tsx`** — Chat interface with message rendering, tool activity display, markdown/code rendering.
- **`components/AgentsCard.tsx`** — Agent status cards with run history and timeline badges.
- **`components/AgentTree.tsx`** — Agent hierarchy visualization.
- **`components/HeaderBar.tsx`** — Top navigation bar.
- **`components/ModelsCard.tsx`** — Model configuration display.
- **`types.ts`** — Shared TypeScript type definitions.

### Agent Definitions (`agents/`)

Agent specs are markdown files with YAML frontmatter. Discovered dynamically at startup — adding a new `.md` file registers a new agent without code changes.

Frontmatter fields: `name`, `description`, `tools`, `model`, `work_globs`, `policy`.

Current agents: `ling` (general-purpose assistant), `coder` (implementation).

### Configuration (`linggen-agent.toml`)

Config search order: `$LINGGEN_CONFIG` env var → `./linggen-agent.toml` → `~/.config/linggen-agent/` → `~/.local/share/linggen-agent/`.

Key sections: `[[models]]` (LLM providers), `[server]` (port), `[agent]` (max_iters, safety mode), `[logging]`, `[[agents]]` (agent spec references).

## Code Style

Follow `doc/code-style.md`:
- Prefer guard clauses and early returns over deep nesting
- Keep files and functions small and focused; refactor when complexity grows
- Remove unused code — no compatibility shims or dead feature flags
- Keep async control flow explicit and traceable

## Key Design Patterns

- **Tool names are Claude Code-style**: `Read`, `Write`, `Edit`, `Bash`, `Glob`, `Grep` (capitalized).
- **Workspace-scoped file operations**: all paths are sandboxed to workspace root; parent traversal (`..`) is rejected.
- **Bash command safety**: commands are validated against an allowlist; shell injection patterns are blocked.
- **Agent policy enforcement**: tools and actions (Patch, Finalize, Delegate) are hard-gated per agent via frontmatter policy, not just prompt guidance.
- **SSE events**: server publishes real-time events (`Token`, `Message`, `AgentStatus`, `SubagentSpawned`, etc.) consumed by the web UI.
- **Delegation depth**: configurable via `max_delegation_depth` (default 2). All agents are equal — any agent can delegate to any other agent within the depth limit.

Please read `.claude/skills/linggen/SKILL.md` on load to understand the Linggen project structure and context management system.
