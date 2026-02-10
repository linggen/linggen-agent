## Product Spec: Linggen Agent

### Human Intent

Linggen Agent is basically an extended web version of Claude Code with a local-model runtime.
I will try other like socical agent in thie framework.

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

- `chat`: plain-text conversation, explanations, and guided iteration.
- `auto`: structured, tool-driven planning/execution behavior.

Mode controls response behavior; safety is enforced by policy/tool constraints.

### Agent model

- Main agents: long-lived coordinators/workers (`lead`, `coder`; `operator` is planned).
- Subagents: short-lived workers owned by one parent main agent.
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

### Safety requirements

- Repo/workspace scoped file operations.
- Allowlisted command execution via `run_command`.
- Persisted chat/run records for traceability.
- Cancellation support for active run trees.

### Non-goals (early stage)

- Multi-tenant hosted SaaS.
- Unbounded autonomous production deployment.
- Removing policy gates from tools.
