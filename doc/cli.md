# CLI Reference

All subcommands for the `linggen-agent` binary.

## Related docs

- `doc/framework.md`: runtime design, tool contract, safety rules.
- `doc/product-spec.md`: product goals, interaction modes, UX.

## Subcommands overview

| Command | Purpose | Needs full runtime? |
|:--------|:--------|:--------------------|
| `agent` | Interactive TUI (REPL) | Yes |
| `serve` | Start HTTP server + Web UI (foreground) | Yes |
| `eval` | Run eval tasks against agents | Yes |
| `doctor` | Diagnose installation health | No |
| `start` | Start server as background daemon | No |
| `stop` | Stop the background daemon | No |
| `status` | Show server/daemon status | No |
| `init` | Bulk-install skills from `linggen/skills` | No |
| `install` | Self-update the binary (alias: `update`) | No |
| `skills` | Manage skills (add/remove/list/search) | No |

"Needs full runtime" means the command initialises tracing, `AgentManager`, database, and skill loading. Lightweight commands only load `Config`.

---

## agent

Interactive multi-agent TUI (ratatui REPL).

```
linggen-agent agent [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--ollama-url <URL>` | Ollama base URL (overrides config) |
| `--model <NAME>` | Model name (overrides config) |
| `--root <PATH>` | Workspace root (default: detect `.git`) |
| `--max-iters <N>` | Max tool iterations per run |
| `--no-stream` | Disable streaming output |

## serve

Start the HTTP API server with embedded Web UI (foreground process).

```
linggen-agent serve [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--port <PORT>` | Listen port (default: `server.port` from config) |
| `--ollama-url <URL>` | Ollama base URL |
| `--model <NAME>` | Model name |
| `--root <PATH>` | Workspace root (default: detect `.git`) |
| `--dev` | Dev mode: skip embedded static assets, proxy to Vite |

## eval

Run evaluation tasks against agents.

```
linggen-agent eval [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--root <PATH>` | Workspace root |
| `--filter <SUBSTRING>` | Filter tasks by name |
| `--max-iters <N>` | Override max iterations per task |
| `--timeout <SECS>` | Per-task timeout (default: 300) |
| `--agent <ID>` | Override agent for all tasks |
| `--verbose` | Print agent messages during execution |

Exits with code 1 if any task fails.

---

## doctor

Print a diagnostic checklist of the installation.

```
linggen-agent doctor
```

Checks (each prints `[OK]`, `[FAIL]`, or `[INFO]` with ANSI colours):

1. Binary version
2. Config file path
3. Workspace root detection
4. Server port reachability (TCP probe, 1 s timeout)
5. Model connectivity (Ollama `/api/tags`, OpenAI `/models`)
6. Skills directories (global + project)
7. Agent definition files count
8. Log directory exists and is writable

## start

Start the server as a background daemon.

```
linggen-agent start [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--port <PORT>` | Listen port (default: `server.port` from config) |
| `--root <PATH>` | Workspace root |

Behaviour:
- Checks if the port is already listening; if so, prints a message and exits.
- Spawns a detached child running `linggen-agent serve --port <PORT>`.
- Writes PID to `~/.linggen/linggen-agent.pid`.
- Daemon stdout/stderr goes to `~/.linggen/linggen-agent.log`.
- Polls TCP connect (up to 3 s) and reports readiness.

## stop

Stop the background daemon.

```
linggen-agent stop
```

Behaviour:
- Reads PID from `~/.linggen/linggen-agent.pid`.
- Sends `SIGTERM`; if still running after 500 ms, sends `SIGKILL`.
- Removes the PID file.

## status

Show server and daemon status.

```
linggen-agent status
```

Prints: config path, server port, running state (with PID if available), workspace root, model count, agent count.

---

## init

Bulk-install all skills from the `linggen/skills` GitHub repository.

```
linggen-agent init [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--global` | Install to `~/.linggen/skills/` (default: project `.linggen/skills/`) |
| `--root <PATH>` | Workspace root for project-scoped install |

Downloads the repository as a ZIP, scans for all `SKILL.md` files, and extracts each skill directory.

## install

Self-update the binary to the latest release.

```
linggen-agent install
linggen-agent update      # alias
```

Behaviour:
- Fetches `manifest.json` from the latest GitHub release.
- Compares versions; if up to date, exits early.
- Downloads the platform-specific binary and atomically replaces the current executable.
- Gracefully handles "no releases available yet".

---

## skills

Manage skills (install, remove, list, search).

```
linggen-agent skills <SUBCOMMAND>
```

### skills add

```
linggen-agent skills add <NAME> [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--repo <URL>` | GitHub repository (default: `linggen/skills`) |
| `--ref <REF>` | Git ref / branch / tag (default: `main`) |
| `--global` | Install to `~/.linggen/skills/` |
| `--force` | Overwrite existing installation |

### skills remove

```
linggen-agent skills remove <NAME> [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--global` | Remove from global scope |

### skills list

```
linggen-agent skills list
```

Scans `~/.linggen/skills/`, `.linggen/skills/`, `~/.claude/skills/`, `~/.codex/skills/` and prints installed skill names with their source.

### skills search

```
linggen-agent skills search <QUERY>
```

Searches the Linggen marketplace (and skills.sh fallback) and prints results.

---

## State files

| File | Purpose |
|:-----|:--------|
| `~/.linggen/linggen-agent.pid` | Daemon PID (created by `start`, removed by `stop`) |
| `~/.linggen/linggen-agent.log` | Daemon stdout/stderr log |

## Configuration

All commands load config from `linggen-agent.toml` (see `doc/framework.md` for search order). Lightweight commands (`doctor`, `start`, `stop`, `status`, `init`, `install`, `skills`) only need the config file; they do not initialise the full agent runtime.

### Model context_window override

For cloud/remote models where the provider API does not report context size (e.g. Ollama `qwen3.5:cloud`), set `context_window` manually:

```toml
[[models]]
id = "ollama1"
provider = "ollama"
url = "http://127.0.0.1:11434"
model = "qwen3.5:cloud"
context_window = 131072
```

## Health endpoint

The server exposes `GET /api/health` returning `{"ok": true}`. Used by `start` for readiness polling.
