---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Plan Spec

Redesign of the plan feature, aligned with Claude Code. Separates planning (read-only research) from execution, with user approval gates. Multi-model compatible.

## Related docs

- `agentic-loop.md`: kernel loop, termination.
- `tools.md`: syscall interface, permissions.
- `chat-spec.md`: SSE events, APIs.
- `storage.md`: plan file persistence.

## Design principles

1. **Enforce at infrastructure, not model level.** Remove tools from the API call — don't trust the model to not call them.
2. **Plans are free-form markdown, not structured JSON.** Any model can write text. No JSON parsing for plan creation.
3. **Progress tracking is separate from planning.** `update_plan` is for execution-time task tracking, decoupled from plan mode.
4. **Per-session persistence.** Plans persist to `~/.linggen/projects/{enc}/sessions/{sid}/plan.md`. Session resume replaces plan resume.
5. **Self-contained plans.** Plans must include file paths, line numbers, and code snippets so they work as the sole context after clearing.
6. **Unified flow.** No `PlanOrigin` enum — all plans use the same approval flow. Plan mode requires user initiation and approval.

## Changes from previous design

### Dropped

- `PlanOrigin` enum — single unified flow, no `ModelManaged` vs `UserRequested` distinction.
- Global plan persistence (`~/.linggen/plans/`) — now per-session.
- Plan auto-resume (`load_latest_plan()`) — session resume replaces it.
- `plan_file` / `plans_dir_override` fields — replaced by `session_plan_dir`.
- `slugify_summary()`, `unique_plan_path()`, `parse_plan_file()` — no longer needed.
- Sidecar `.meta.json` files — simplified to single `plan.md`.
- Stale `Planned` → `Executing` auto-promotion — no silent approval bypass.
- Bash in plan mode — blocked entirely for safety.

### Kept

- SSE `PlanUpdate` events.
- `update_plan` action for execution-time progress (separate feature).
- `plan_mode: bool` on engine.
- Status lifecycle: `Planned` → `Approved` → `Executing` → `Completed`.
- `auto_complete_plan()` marking remaining items as `Skipped`.

### Added

- **`ExitPlanMode` tool** — model calls this when plan is ready, triggers approval.
- **Fallback detection** — if model emits `done` in plan mode without calling `ExitPlanMode`, treat as implicit exit.
- **Direct plan editing** — user can edit plan markdown before approving (TUI: `$EDITOR`, Web UI: inline editor).
- **Rich approval UI** — shows context usage percentage, offers "Approve, clear context" / "Approve, keep context" / "Reject".
- **Self-contained plan prompts** — plan-mode prompt emphasizes code snippets and line numbers.
- **Per-session plan dir** — `session_plan_dir` field on engine, set from `ProjectStore.project_dir()`.

## Lifecycle

### Entry

User-initiated only:
1. `/plan <task>` command in Web UI or TUI.
2. Model emits `enter_plan_mode` → engine asks user for consent first.

### Research phase

Tools available: `Read`, `Glob`, `Grep`, `WebSearch`, `WebFetch`, `AskUser`, `ExitPlanMode`.

Tools blocked (removed from API call): `Write`, `Edit`, `Bash`, `Task`, `lock_paths`, `unlock_paths`, `Skill`.

Model researches the codebase, then writes a self-contained markdown plan and calls `ExitPlanMode`.

### Approval

User options:
- **Approve, clear context (X% used)** — clears conversation history; plan is sole context.
- **Approve, keep context** — retains full conversation; plan appended.
- **Give feedback** — user sends a message; model refines plan.
- **Reject** — plan discarded.

### Execution

Plan text injected into system prompt. Model executes with full tools. Progress tracking via `update_plan` is optional and independent.

### Completion

On `done`: remaining tracked items marked `Skipped` (not `Done`), plan status → `Completed`, file updated, SSE event sent.

## Progress tracking

`update_plan` remains as an independent feature — not coupled to plan mode. Used during execution or spontaneously for complex tasks. Drives the sidebar task list UI.

## Storage

Plan file: `~/.linggen/projects/{encoded_path}/sessions/{session_id}/plan.md`

Single file, no sidecar metadata. Written on every plan update, overwritten in place.

## Model capability tiers

| Tier | Models | Support |
|------|--------|---------|
| **Tier 1** | Claude, GPT-4, Gemini Pro | Full: `ExitPlanMode` tool, progress tracking, `AskUser` |
| **Tier 2** | Qwen-72B, DeepSeek, Llama-70B+ | Core: tool calling with fallback detection |
| **Tier 3** | Small Ollama (7B-14B) | Basic: fallback only, manual approval trigger |

Tier configured via `capability_tier` in model config. Default: Tier 1. Tier 3 auto-treats `done` in plan mode as `ExitPlanMode`.
