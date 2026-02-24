# CLI Reference

The `ling` binary — Linggen AI coding agent.

## Related docs

- `doc/tools.md`: tool contract, safety rules.
- `doc/agentic-loop.md`: runtime design, loop behaviour.
- `doc/product-spec.md`: product goals, interaction modes, UX.

## Quick reference

```
ling                              # Start server + TUI (default)
ling --web                        # Web UI only, no TUI
ling -d                           # Run as background daemon
ling --port 8080                  # Custom port
ling --root /path/to/project      # Custom workspace

ling stop                         # Stop background daemon
ling status                       # Show agent + memory status
ling doctor                       # Full health diagnostics

ling memory start                 # Start memory server (aliases: mem, m)
ling memory stop                  # Stop memory server
ling memory status                # Memory server status
ling m index .                    # Index current directory

ling install                      # Install ling
ling install --memory             # Install ling-mem
ling install --all                # Install both
ling update                       # Update ling
ling update --all                 # Update both
ling init                         # Bootstrap skills
ling skills add/remove/list/search
ling eval                         # Run eval tasks
```

## Subcommands overview

| Command | Purpose | Needs full runtime? |
|:--------|:--------|:--------------------|
| *(none)* | Interactive TUI + embedded server | Yes |
| `stop` | Stop background daemon | No |
| `status` | Show agent + memory server status | No |
| `doctor` | Diagnose installation health | No |
| `memory` | Manage memory server (`start/stop/status/index`) | No |
| `eval` | Run eval tasks against agents | Yes |
| `init` | Bulk-install skills from `linggen/skills` | No |
| `install` | Install ling (and optionally ling-mem) | No |
| `update` | Update ling (and optionally ling-mem) | No |
| `skills` | Manage skills (add/remove/list/search) | No |

"Needs full runtime" means the command initialises tracing, `AgentManager`, database, and skill loading. Lightweight commands only load `Config`.

---

## Default (bare `ling`)

Start the agent server with an interactive TUI.

```
ling [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--root <PATH>` | Workspace root (default: detect `.git`) |
| `--port <PORT>` | Server port (default: `server.port` from config) |
| `--web` | Web UI only — foreground server, no TUI |
| `-d, --daemon` | Run as background daemon (implies `--web`) |
| `--dev` | Dev mode: proxy static assets to Vite dev server |

When no flags are given, `ling` starts the HTTP server on the configured port with the embedded Web UI, then opens the interactive TUI.

### Daemon mode (`-d`)

Spawns a detached child running `ling --web --port <PORT>`:
- Writes PID to `~/.linggen/ling.pid`.
- Daemon stdout/stderr goes to `~/.linggen/ling.log`.
- Polls TCP connect (up to 3 s) and reports readiness.

## stop

Stop the background daemon.

```
ling stop
```

Reads PID from `~/.linggen/ling.pid`, sends `SIGTERM`; if still running after 500 ms, sends `SIGKILL`. Removes the PID file.

## status

Show agent and memory server status.

```
ling status
```

Prints: version, config path, agent server port + running state, memory server port + running state, memory binary location, workspace root, model count, agent count.

## doctor

Print a diagnostic checklist of the installation.

```
ling doctor
```

Checks (each prints `[OK]`, `[FAIL]`, or `[INFO]` with ANSI colours):

1. Binary version
2. Config file path
3. Workspace root detection
4. Agent server port reachability (TCP probe, 1 s timeout)
5. Memory server port reachability
6. Model connectivity (Ollama `/api/tags`, OpenAI `/models`)
7. Skills directories (global + project)
8. Agent definition files count
9. Log directory exists and is writable

---

## memory

Manage the ling-mem server. Aliases: `mem`, `m`.

```
ling memory <SUBCOMMAND>
ling mem <SUBCOMMAND>
ling m <SUBCOMMAND>
```

### memory start

Start the memory server as a background process.

```
ling memory start
```

Finds the `ling-mem` binary (checks: macOS Application Support, PATH, alongside `ling`), spawns it with `--port <memory.server_port>`. Writes PID to `~/.linggen/ling-mem.pid`, logs to `~/.linggen/ling-mem.log`.

### memory stop

Stop the memory server.

```
ling memory stop
```

### memory status

Show memory server status.

```
ling memory status
```

### memory index

Index a local directory via the memory server.

```
ling memory index [PATH] [OPTIONS]
ling m index .
```

| Flag | Description |
|:-----|:-----------|
| `PATH` | Directory to index (default: current directory) |
| `--mode <MODE>` | Indexing mode: `auto`, `full`, or `incremental` (default: `auto`) |
| `--name <NAME>` | Override the source name |
| `--include <GLOB>` | Include patterns (repeatable) |
| `--exclude <GLOB>` | Exclude patterns (repeatable) |
| `--no-wait` | Don't wait for the indexing job to complete |

---

## eval

Run evaluation tasks against agents.

```
ling eval [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--filter <SUBSTRING>` | Filter tasks by name |
| `--max-iters <N>` | Override max iterations per task |
| `--timeout <SECS>` | Per-task timeout (default: 300) |
| `--agent <ID>` | Override agent for all tasks |
| `--verbose` | Print agent messages during execution |

Exits with code 1 if any task fails. The `--root` global flag sets the workspace root.

---

## init

Bulk-install all skills from the `linggen/skills` GitHub repository.

```
ling init [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--global` | Install to `~/.linggen/skills/` (default: project `.linggen/skills/`) |

Downloads the repository as a ZIP, scans for all `SKILL.md` files, and extracts each skill directory.

## install

Install the `ling` binary (default), and optionally `ling-mem`.

```
ling install              # install ling only (default)
ling install --memory     # install ling-mem only (alias: --mem)
ling install --all        # install both
```

| Flag | Description |
|:-----|:-----------|
| `--memory` / `--mem` | Install ling-mem binary |
| `--all` | Install both ling and ling-mem |

Fetches `manifest.json` from the latest GitHub releases. Downloads platform-specific binaries and installs them. The memory binary is placed alongside the `ling` binary.

## update

Update the `ling` binary (default), and optionally `ling-mem`.

```
ling update               # update ling only (default)
ling update --memory      # update ling-mem only (alias: --mem)
ling update --all         # update both
```

Same flags as `install`. Compares versions and skips if already up to date.

---

## skills

Manage skills (install, remove, list, search).

```
ling skills <SUBCOMMAND>
```

### skills add

```
ling skills add <NAME> [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--repo <URL>` | GitHub repository (default: `linggen/skills`) |
| `--ref <REF>` | Git ref / branch / tag (default: `main`) |
| `--global` | Install to `~/.linggen/skills/` |
| `--force` | Overwrite existing installation |

### skills remove

```
ling skills remove <NAME> [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--global` | Remove from global scope |

### skills list

```
ling skills list
```

Scans `~/.linggen/skills/`, `.linggen/skills/`, `~/.claude/skills/`, `~/.codex/skills/` and prints installed skill names with their source.

### skills search

```
ling skills search <QUERY>
```

Searches the Linggen marketplace (and skills.sh fallback) and prints results.

---

## Global flags

These flags can be used with any command:

| Flag | Description |
|:-----|:-----------|
| `--root <PATH>` | Workspace root (default: walk up to find `.git`) |
| `--port <PORT>` | Server port (default: from config) |

---

## State files

| File | Purpose |
|:-----|:--------|
| `~/.linggen/ling.pid` | Agent daemon PID (created by `-d`, removed by `stop`) |
| `~/.linggen/ling.log` | Agent daemon stdout/stderr log |
| `~/.linggen/ling-mem.pid` | Memory server PID |
| `~/.linggen/ling-mem.log` | Memory server log |

## Configuration

All commands load config from `linggen-agent.toml` (see `doc/storage.md` for search order). Lightweight commands only need the config file; they do not initialise the full agent runtime.

### Memory server config

```toml
[memory]
server_port = 8787                      # default
server_url = "http://127.0.0.1:8787"   # default
```

### Model context_window override

For cloud/remote models where the provider API does not report context size:

```toml
[[models]]
id = "ollama1"
provider = "ollama"
url = "http://127.0.0.1:11434"
model = "qwen3.5:cloud"
context_window = 131072
```

## Health endpoint

The server exposes `GET /api/health` returning `{"ok": true}`. Used by daemon mode for readiness polling.
