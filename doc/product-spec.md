## Product Spec: Linggen Agent

### Human Intent

Linggen Agent is basically an extended web version of Claude Code with a local-model runtime.
The framework should support additional specialized agents over time.

### Summary

Linggen Agent is a local-first, multi-agent coding assistant with two explicit modes: `chat` and `auto`.

### Related docs

- `doc/framework.md`: runtime/tooling/safety design.
- `doc/multi-agents.md`: main/subagent runtime contract, events, and APIs.

### Product goals

- Keep daily coding workflows local-first (Ollama by default).
- Provide a web UI with live agent status and chat history.
- Keep execution safe through tool constraints, workspace boundaries, and auditability.
- Support optional cloud escalation only when configured.

### Interaction modes

- `chat`: human-in-the-loop mode, basically Claude Code mode. The user guides iteration and can intervene between steps.
  - Chat behavior is still agentic: tools can chain across turns (one tool call per turn) until a final plain-text answer.
- `auto`: human-not-in-the-loop mode. Agents stay in the loop and continue execution until completion, failure, or cancellation.
  - Both modes are bounded by `agent.max_iters` from config.

Mode controls response behavior; safety is enforced by policy/tool constraints.

### Agent model

- Agent definitions are loaded dynamically from markdown files in `agents/*.md`.
- No agent list is hardcoded in runtime code or static config.
- Adding a new `agents/*.md` file creates a new available agent (after runtime reload/startup).
- Agent action gates are configured dynamically from frontmatter `policy`.
  - Shorthand syntax: `policy: [Patch, Finalize, Delegate]`.
  - Optional object syntax for scoped delegation:
    - `policy.allow: [Patch, Delegate]`
    - `policy.delegate_targets: [search, plan]`
- Main agents: long-lived coordinators/workers (`kind: main` in agent markdown).
- Subagents: short-lived workers owned by one parent main agent (`kind: subagent` in agent markdown).
- Delegation depth is fixed to one level (`main -> subagent` only).

### Current UX surface

- CLI:
  - `linggen-agent agent` for interactive chat.
  - `linggen-agent serve` for API + web UI.
  - Future: merge CLI entry points into `linggen` / `linggen-cli`.
- Web UI:
  - Session-based chat.
  - Agent status (`model_loading`, `thinking`, `calling_tool`, `working`, `idle`).
  - Agent hierarchy and context inspection.
  - Per-agent run history selector (main + subagent).
  - Run pin/unpin to keep a specific context stable while new runs arrive.
  - Run timeline panel (start/end, subagent spawn/return, tool/task milestones).
  - Context message filtering.
  - Right-side agent cards show compact run/timeline/subagent badges.

### Safety requirements

- Repo/workspace scoped file operations.
- Canonical tool contract uses Claude Code-style names (`Read`, `Write`, `Bash`, `Glob`, `Grep`).
- Allowlisted command execution via `Bash`.
- Persisted chat/run records for traceability.
- Cancellation support for active run trees.

### Non-goals (early stage)

- Multi-tenant hosted SaaS.
- Unbounded autonomous production deployment.
- Removing policy gates from tools.
