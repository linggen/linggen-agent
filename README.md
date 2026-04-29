<p align="center">
  <img src="logo.svg" width="120" alt="Linggen" />
</p>

<h1 align="center">Linggen</h1>

<p align="center">
  <strong>Local AI coding agent you can access from anywhere.</strong><br>
  Open-source. Any model. WebRTC remote access. Skills you can share.
</p>

<p align="center">
  <a href="https://linggen.dev">Website</a> &middot;
  <a href="https://linggen.dev">Demo Video</a> &middot;
  <a href="https://linggen.dev/docs">Docs</a> &middot;
  <a href="https://linggen.dev/skills">Skills Marketplace</a> &middot;
  <a href="https://discord.gg/linggen">Discord</a>
</p>

<p align="center">
  <a href="https://github.com/linggen/linggen/releases"><img src="https://img.shields.io/github/v/release/linggen/linggen?style=flat-square" alt="Release" /></a>
  <a href="https://github.com/linggen/linggen/blob/main/LICENSE"><img src="https://img.shields.io/github/license/linggen/linggen?style=flat-square" alt="MIT License" /></a>
  <a href="https://github.com/linggen/linggen/stargazers"><img src="https://img.shields.io/github/stars/linggen/linggen?style=flat-square" alt="Stars" /></a>
</p>

---

## What is Linggen?

Linggen is an AI coding agent that runs on your machine — and lets you access it from any device via WebRTC. Start a task on your desktop, check on it from your phone. No cloud hosting, no subscriptions, your models and your data.

```bash
curl -fsSL https://linggen.dev/install.sh | bash
ling
```

That's it. Opens a web UI at `localhost:9898`.

## Why Linggen over Claude Code / Cursor / Codex?

| | Linggen | Claude Code | Cursor | Codex |
|---|---|---|---|---|
| **Runs locally** | Yes | Yes | No (cloud) | Cloud-only |
| **Any model** | Ollama, Claude, GPT, Gemini, DeepSeek, Groq, OpenRouter | Claude only | Multi-model | GPT only |
| **Remote access** | P2P WebRTC — your machine, any device | Cloud-hosted web app | No | Cloud-only |
| **Share models with others** | Yes — proxy rooms over P2P | No | No | No |
| **Open source** | MIT | No | No | CLI only |
| **Skills/extensions** | Drop-in SKILL.md files ([Agent Skills](https://agentskills.io) standard) | Custom slash commands | Plugins | No |
| **Web UI** | Full web interface with streaming | Terminal + web app | IDE-embedded | Web (cloud) |
| **Data stays local** | Always — P2P, no relay | Code sent to cloud for web app | Cloud-processed | Cloud-processed |
| **Cost** | Free + your model costs | $20/mo or API costs | $20/mo | API costs |

## Key Features

### Remote Access via WebRTC

Start a coding task on your desktop, monitor it from your phone. No VPN, no port forwarding — peer-to-peer encrypted connection.

```bash
ling login   # link to linggen.dev
```

Then open `linggen.dev/app` from any browser, anywhere.

### Any Model, Your Choice

Use local models via Ollama, or cloud APIs — Claude, GPT, Gemini, DeepSeek, Groq, OpenRouter. Switch models mid-conversation. Configure fallback chains so work never stops.

### Proxy Rooms — Share Your Models

Open a private or public room and let friends, teammates, or the community talk to your models — over the same P2P WebRTC link, no cloud middleman. Pick which models are shared, which tools and skills consumers can use, and set daily token budgets per room and per consumer. Disable the room and everyone is kicked instantly.

```bash
# Owner: enable a room in Settings → Sharing
# Consumer: open linggen.dev/app and connect to the room name
```

Room config lives at `~/.linggen/room_config.toml`; persistent token usage at `~/.linggen/token_usage.json` (auto-resets at midnight UTC).

### Semantic Memory

The `ling-mem` skill gives the agent a LanceDB-backed semantic memory: typed facts (preference / decision / learned / fact), 384-dim embeddings, filter-and-search, first-class forgetting. It remembers across sessions, projects, and tools — and works the same from Linggen, Claude Code, or any agent that can shell out.

### Skills, Not Plugins

Drop a `SKILL.md` into your project and the agent gains new capabilities instantly. Skills follow the open [Agent Skills](https://agentskills.io) standard, compatible with Claude Code and Codex.

```
~/.linggen/skills/my-skill/SKILL.md
```

Browse and install community skills from the [marketplace](https://linggen.dev/skills).

### Multi-Agent Delegation

Agents delegate tasks to other agents — each with its own context, tools, and model. Like `fork()` for AI.

### Plan Mode

For complex tasks, the agent proposes a plan before acting. Review, edit, or approve — then it executes. Stay in control on high-stakes changes.

### Mission System

Schedule recurring tasks with cron expressions. Three run modes — `agent` (full agent loop), `app` (open a URL), or `script` (run a shell command) — so missions cover everything from code reviews and dependency updates to dashboards and one-shot scripts. Each mission carries its own permission scope, allowed tools, and allowed skills.

## Quick Start

```bash
# Install
curl -fsSL https://linggen.dev/install.sh | bash

# First-time setup
ling init

# Start (opens browser)
ling

# Optional: enable remote access
ling login
```

## Adding Agents

Drop a markdown file in `~/.linggen/agents/` — available immediately, no restart:

```markdown
---
name: reviewer
description: Code review specialist.
tools: ["Read", "Glob", "Grep"]
model: claude-sonnet-4-20250514
---

You review code for bugs, style issues, and security vulnerabilities.
```

## Documentation

- [Design docs](doc/) — architecture, specs, and internals
- [Full docs](https://linggen.dev/docs) — guides and reference
- [Skill spec](doc/skill-spec.md) — how to write skills

## Contributing

Contributions welcome. See the [design docs](doc/) for architecture context.

## License

MIT
