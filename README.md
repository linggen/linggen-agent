<p align="center">
  <img src="logo.svg" width="120" alt="Linggen" />
</p>

<h1 align="center">Linggen</h1>

<p align="center">
  Open-source AI agent system. Run any model. Add skills by dropping files. Access from anywhere.
</p>

<p align="center">
  <a href="https://linggen.dev">linggen.dev</a> &middot;
  <a href="https://linggen.dev/docs">Docs</a> &middot;
  <a href="https://linggen.dev/skills">Skills</a>
</p>

---

Linggen runs on your machine and gives you a fully-featured AI coding agent — with the models you choose, the skills you need, and access from any device.

## Why Linggen

**You own everything.** Linggen runs locally. Your code, your keys, your data — nothing leaves your machine unless you tell it to.

**Any model.** Ollama, OpenAI, Claude, Gemini, DeepSeek, Groq, OpenRouter — use one or all of them.

**Skills, not plugins.** Drop a `SKILL.md` file into your project and the agent gains new capabilities instantly. Skills follow the open [Agent Skills](https://agentskills.io) standard, compatible with Claude Code and Codex.

**Access from anywhere.** Built-in WebRTC transport lets you use your linggen from your phone, laptop, or any browser — no VPN or port forwarding needed.

**Web UI + Terminal.** Full web interface and a terminal TUI, both connected to the same backend in real-time.

## Install

```bash
curl -fsSL https://linggen.dev/install.sh | bash
```

Then:

```bash
ling init    # set up ~/.linggen/ with default config
ling         # start (TUI + Web UI at localhost:9898)
```

## Key Features

### Skills Marketplace

Search and install community skills in one click — or write your own. Skills can add tools, knowledge, interactive apps, and more. Browse the [marketplace](https://linggen.dev/skills).

### Multi-Agent Delegation

Agents can delegate tasks to other agents. Each agent has its own context, tools, and model. Delegation depth is configurable — like `fork()` for AI.

### Mission System

Schedule recurring tasks with cron expressions. Agents self-initiate work on a schedule — code reviews, dependency updates, monitoring, whatever you define.

### Remote Access

Link your linggen to [linggen.dev](https://linggen.dev) for remote access via WebRTC. One command to set up:

```bash
ling login
```

Then connect from any browser at `linggen.dev/app`.

### Plan Mode

For complex tasks, the agent proposes a plan before acting. Review, edit, or approve — then it executes. Keeps you in control on high-stakes changes.

## Adding Skills

```
~/.linggen/skills/my-skill/SKILL.md
```

```markdown
---
name: my-skill
description: Does something useful.
allowed-tools: [Bash, Read]
---

Instructions for the agent when this skill is invoked.
```

Invoke via `/my-skill` in chat. Skills are also triggered automatically based on context.

## Adding Agents

Drop a markdown file in `~/.linggen/agents/`:

```markdown
---
name: reviewer
description: Code review specialist.
tools: ["Read", "Glob", "Grep"]
model: claude-sonnet-4-20250514
---

You review code for bugs, style issues, and security vulnerabilities.
```

Available immediately — no restart needed.

## Documentation

Design docs: [`doc/`](doc/) | Full docs: [linggen.dev/docs](https://linggen.dev/docs)

## License

MIT
