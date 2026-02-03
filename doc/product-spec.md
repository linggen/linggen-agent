## Product Spec: Linggen Agent (v1 → vNext)

### Summary

Linggen Agent is a local-first, multi-agent system that can plan, code, test, and release software with configurable autonomy. Humans primarily interact with a PM agent via CLI/Web UI; the PM agent dispatches work to specialist agents (Coder, Operator, etc.).

### Related docs

- **Technical design**: see `doc/framework.md` (architecture, tool protocol, safety enforcement).

### Target users

- **Solo developers** who want a reliable “hands-off” coding + testing loop.
- **Small teams** who want an internal autonomous assistant that reduces repetitive DevOps and QA work.
- **Power users** who want long-running automation with remote monitoring (phone/web).

### Product goals

- **Autonomy control**: explicit Autonomy Levels from fully human-in-the-loop to largely autonomous operation.
- **Local-first**: use local Ollama models by default (e.g. `qwen3-coder` 30B).
- **Cloud escalation**: optional Claude API for hard tasks, with strict cost controls.
- **Safety by design**: policy-gated tools, allowlists, audits, and rollbacks.
- **Remote visibility**: web UI to monitor progress, read diffs/logs, and intervene if needed.
- **UI visibility**: allow humans to view the **running app UI** (dev server/staging) and review UI artifacts (screenshots/recordings).

### Non-goals (early)

- Running as a hosted SaaS (multi-tenant service).
- Unbounded autonomous production changes without auditability and rollback.
- Full “agent writes anything anywhere” behavior (must remain policy-driven).

### Autonomy Levels (FSD-style, L1–L5)

We use an FSD-inspired definition: **L1 is most human-in-the-loop**, **L5 is full autonomy**.
Autonomy Level controls permissions, tool availability, approvals, and cloud spend.

- **L1 — Driver Assistance (Human-driven)**
  - Human is actively driving the work; agent assists with analysis and suggestions.
  - Typical actions: read/search, propose plans/diffs, suggest checks; no workspace mutation by default.
- **L2 — Partial Automation (Human supervising)**
  - Agent can execute bounded steps (e.g. run allowlisted checks, apply small patches), while the human supervises.
  - Deploy/release actions remain approval-gated by default.
- **L3 — Conditional Automation (Human on-call)**
  - Agent can complete an end-to-end workflow in defined environments (dev/staging) and asks for takeover when uncertain.
  - Production actions require explicit gates and escalation.
- **L4 — High Automation (Unattended within ODD)**
  - Agent operates unattended for long periods within a well-defined operational design domain (ODD): repo + environments + policies.
  - Can deploy with automated rollback + health checks + audit trail.
- **L5 — Full Automation (General autonomy)**
  - Agent can operate across projects/environments with minimal human involvement.
  - Reserved as a long-term goal; requires mature safety, governance, and cost controls.

### Core agents

- **PM agent (Planner/Manager)**
  - Primary chat interface.
  - Converts goals → user stories + acceptance criteria.
  - Dispatches tasks and tracks progress.
- **Coder agent**
  - Implements tasks and produces diffs/patches.
  - Uses local Ollama by default.
- **Operator agent**
  - Runs tests/builds, deploys, monitors, and rolls back (as permitted).
- **Extension agents (later)**
  - Reviewer, Social/Notify, Refactor, etc.

### UX requirements

- **CLI MVP**
  - `linggen coder` for interactive coding.
  - Planned: `linggen pm`, `linggen operator`.
- **Web UI (vNext)**
  - Status dashboard: task stage, confidence, failures, last actions.
  - Artifacts: user stories, diffs, `/check` logs, deployment reports.
  - Chat with PM agent (PM dispatches to others).

### UI workflow (PM → Coder → Operator → PM)

When the task involves UI/UX, the system should converge quickly with minimal back-and-forth.

- **PM agent (design intent + confirmation)**
  - Produces: user stories + acceptance criteria + **wireframe** (Mermaid) and/or a lightweight **HTML prototype**.
  - Requests a quick user confirmation at L1/L2 before full implementation.
- **Coder agent (implementation)**
  - Implements UI based on the PM’s spec/prototype and generates a patch.
  - Suggests verification steps (unit tests + UI checks where applicable).
- **Operator agent (runtime validation)**
  - Runs the app and exposes a preview URL (dev server or staging).
  - Captures **screenshots (and optionally short recordings)** as reviewable artifacts.
  - Runs automated UI checks (Playwright-style) when configured.
- **PM agent (review loop)**
  - Summarizes whether the runtime UI matches acceptance criteria and asks for approval if required by autonomy level.

### Safety & policy requirements

- **Tool allowlists** (especially for `run_command`) with default-deny.
- **Workspace boundaries**: repo-root restriction + ignore rules.
- **Approval gates**: required confirmations for high-risk actions (especially L3–L5).
- **Audit trail**: command history, diffs produced/applied, deploy reports.
- **Rollback strategy**: required for any deploy-enabled autonomy level.

### Model strategy & cost control

- **Local default**: Ollama `qwen3-coder` (configurable).
- **Cloud escalation** (optional):
  - Used only when triggered by policy (e.g. repeated failures, high-risk refactor, complex planning).
  - Enforced by budgets: per-task and per-day spend limits.
  - Prompt packaging: send minimal context (task packet + relevant diffs/logs), not entire repos.
- **Optional vision-language (VL) model** (local preferred):
  - Used to describe and sanity-check UI from screenshots/recordings (e.g. detect missing buttons/text/layout regressions).
  - Keeps the Coder model text-first; UI “perception” is handled by the VL model/tooling.

### MVP scope (recommended)

- L1 Patch-only Coder agent with `/check` allowlisted verification.
- PM agent that converts user intent into structured tasks and dispatches to Coder.
- Basic Operator agent to run tests and report results (no deploy yet, or staging-only).
- For UI tasks: PM produces Mermaid wireframes (and optionally HTML prototypes) + Operator captures screenshots for review.
