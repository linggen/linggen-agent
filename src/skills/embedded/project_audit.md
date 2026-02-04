---
name: project_audit
description: Scans the workspace to summarize current progress, user stories, and project status.
---

# Project Audit Skill

You are now in **Audit Mode**. Your goal is to provide a comprehensive "State of the Project" report for the current workspace.

### Your Workflow:
1.  **Inventory**: Use `Glob` to find all documentation (`.md` files), user stories, and source code.
2.  **Status Check**: 
    - Read the `.linggen-agent/` directory to see current tasks and their states.
    - Grep for `TODO` or `FIXME` in the codebase to identify pending technical debt.
    - Read existing `user-stories.md` or `tasks/` files to count "Done" vs "Pending" items.
3.  **Summarize**: Produce a report with:
    - **Project Essence**: A 1-2 sentence summary of what this application does.
    - **Progress**: A summary of completed vs pending user stories/tasks.
    - **Current Focus**: What is currently being worked on?
    - **Next Steps**: Recommended next 3 actionable tasks to move the project forward.

### Constraints:
- Do not make any changes to files.
- If you find conflicting information between documentation and code, highlight it.
- Be concise but thorough.
- Never assume a file exists. Always `Glob` first, and only `Read` paths that were listed by `Glob`.
- Limit file reads: prefer reading at most 3-5 key docs before summarizing.

### Example Output Format:

**Project Essence**: Linggen Agent is a multi-agent development workspace that uses Rust and React to provide autonomous coding capabilities.

**Progress**:
- ✅ 5 User Stories completed (Project management, DB persistence, SSE streaming).
- ⏳ 2 User Stories pending (Swarm mode orchestration, Advanced tool delegation).
- 🛠️ 12 TODOs found in `src/engine/mod.rs` and `src/server/mod.rs`.

**Current Focus**: Implementing the `project_audit` skill and integrating it into the Lead agent's workflow.

**Next Steps**:
1. Refactor `AgentManager` to support dynamic agent scaling.
2. Implement `get_agent_status` tool for better swarm observability.
3. Add comprehensive test suite for Redb persistence layer.
