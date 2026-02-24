# Tools

Syscall interface: built-in tools, safety rules, and the two-tier model.

## Related docs

- `skills.md`: dynamic extensions (skill tools).
- `agentic-loop.md`: how tools are dispatched in the loop.
- `agents.md`: per-agent tool access control.

## Two-tier model

| Tier | What | Why built-in |
|:-----|:-----|:-------------|
| **Built-in tools** | Core coding ops | Need safety enforcement, internal state, or performance |
| **Skills** | Everything else | Extensible, replaceable, no code changes |

Built-in tools are the kernel API. Skills are userspace.

## Built-in tools

| Tool | Args | Purpose |
|:-----|:-----|:--------|
| `Glob` | `globs?, max_results?` | File pattern matching |
| `Read` | `path, max_bytes?, line_range?` | Read file contents |
| `Grep` | `query, globs?, max_results?` | Content search (ripgrep) |
| `Write` | `path, content` | Create/overwrite file |
| `Edit` | `path, old_string, new_string, replace_all?` | String replacement in file |
| `Bash` | `cmd, timeout_ms?` | Shell command execution |
| `capture_screenshot` | `url, delay_ms?` | Web page screenshot |
| `lock_paths` | `globs, ttl_ms?` | Acquire file locks (multi-agent) |
| `unlock_paths` | `tokens` | Release file locks |
| `delegate_to_agent` | `target_agent_id, task` | Spawn subagent |
| `WebSearch` | `query, max_results?` | Web search (DuckDuckGo) |

**Aliases**: `Read`/`Write`/`Edit` accept `file`/`filepath` for `path`. `Edit` accepts `old`/`search`/`from` for `old_string`, `new`/`replace`/`to` for `new_string`.

Tool names follow Claude Code convention (capitalized).

## Code execution 

Model can output `execute_code` action with language and code body.

```json
{"type": "execute_code", "language": "python", "code": "..."}
```

Engine executes via subprocess in workspace root. Same safety as Bash. Result (stdout/stderr/exit_code) fed back as observation. Model-agnostic.

## Tool dispatch

Dispatch order in `ToolRegistry.execute()`:

1. **Builtins** — canonical name match.
2. **Skill tools** — HashMap lookup.
3. **Unknown** — error.

**Implementation**: `engine/tool_registry.rs`, `engine/tools.rs`

## Access control

- Per-agent via `spec.tools` in frontmatter. Wildcard `tools: ["*"]` = unrestricted.
- Action gates via policy: `Patch`, `Finalize`, `Delegate`.
- Write-safety mode: checks that file was Read before Write/Edit.
- Redundancy detection: cache + loop-breaker for repeated calls.

## File safety

- All paths sanitized to workspace root.
- Parent traversal (`..`) rejected.
- Absolute paths outside workspace rejected.
- File listing/search use ignore-aware walking.

## Bash safety

- Commands validated against allowlist before execution.
- Disallowed: `$(`, backticks, newline injection.
- Shell separators (`|`, `;`, `&&`, `||`) parsed; each segment's first token must be allowlisted.
- Timeout enforced (default 30s).
- Output captured (stdout + stderr).
