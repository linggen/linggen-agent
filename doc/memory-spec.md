---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Memory

Persistent knowledge extracted from conversations. The agent remembers who the user is and what it has done — across all sessions, all projects.

## Related docs

- `session-spec.md`: system prompt assembly (layer 7 = memory).
- `storage-spec.md`: filesystem layout (`~/.linggen/memory/`).
- `skill-spec.md`: skill format, progressive disclosure pattern.
- `mission-spec.md`: missions trigger the memory skill nightly.

## Core concept

Memory has two parts:

1. **Loading** (built-in) — the engine reads memory file frontmatters at server start and injects descriptions into the system prompt. Same pattern as skill discovery.
2. **Writing** (skill + mission) — a nightly mission runs the `memory` skill, which reads the day's session files, sends them to the model, and the model updates the memory files directly.

All memory is **global** (`~/.linggen/memory/`). No project-scoped memory. If a user says "I prefer Rust over Java" in one project, every future session should know that.

Two units are tracked: **user** (what the user said, claims, prefers) and **agent** (what the agent did, decided, observed).

## Built-in vs skill

The split follows the same pattern as skills: **engine loads metadata, skill does the work.**

| Concern | Who | How |
|:--------|:----|:----|
| Read frontmatter descriptions at server start | **Engine (built-in)** | Scan `~/.linggen/memory/*.md`, parse YAML frontmatter |
| Inject descriptions into system prompt | **Engine (built-in)** | Layer 7, same as skill descriptions in layer 3 |
| Tell agent where memory files live | **Engine (built-in)** | One line in prompt with path hint |
| Extract facts from conversations | **Skill + Mission** | Nightly cron, model reads sessions and updates memory files |
| Compress week → month → year | **Skill + Mission** | Same nightly run handles compression |
| Real-time "remember this" writes | **Agent** | Uses `Write` tool directly, no special support |
| Update frontmatter descriptions after writes | **Skill / Agent** | Writer updates `description` field after any change |

The engine's built-in part is minimal: read files' frontmatter, inject descriptions into prompt. All extraction intelligence lives in the skill.

### Comparison with skill loading

| Aspect | Skills | Memory |
|:-------|:-------|:-------|
| Discovery path | `~/.linggen/skills/*/SKILL.md` | `~/.linggen/memory/*.md` |
| Metadata loaded at startup | `name` + `description` | `name` + `description` |
| Injected into system prompt | Layer 3 (available skills) | Layer 7 (memory summaries) |
| Full content loaded | On skill invocation | On agent `Read` (on demand) |
| Who writes the files | Skill author (human) | Memory skill (agent) |

### What the engine replaces

The current built-in memory system (`system-prompt.toml` memory blocks, `MEMORY.md` index, project-scoped memory, "how to save" instructions in prompt) is replaced by:

- **Delete**: `memory_block`, `memory_block_empty`, `global_memory_block`, `global_memory_block_empty` templates
- **Delete**: `MEMORY.md` index file pattern
- **Delete**: project-scoped memory (`~/.linggen/projects/{encoded}/memory/`)
- **Delete**: `memory_dir` per-project wiring in `agent_manager`
- **Add**: simple frontmatter scanner for `~/.linggen/memory/*.md`
- **Add**: one prompt template that lists memory descriptions + path hint

## Fixed memory files (v1)

Five well-known files. The memory skill only writes to these — no ad-hoc file creation in v1.

| File | Unit | Purpose |
|:-----|:-----|:--------|
| `user_info.md` | user | Identity, preferences, hobbies, claims — everything the user has ever told the agent |
| `user_feedback.md` | user | How the user wants the agent to behave — corrections, confirmed approaches, style rules |
| `agent_done_week.md` | agent | What the agent did this week — detailed, rolling 7 days |
| `agent_done_month.md` | agent | Compressed monthly summary — key actions and outcomes |
| `agent_done_year.md` | agent | High-level yearly summary — major milestones only |

### Time-decay model

Like human memory: recent events are vivid, older ones fade to what mattered.

```
This week          → agent_done_week.md   (detailed: files changed, commands run, decisions made)
  ↓ compress
Past months        → agent_done_month.md  (summarized: features built, bugs fixed, deploys)
  ↓ compress
Past years         → agent_done_year.md   (highlights: major milestones, architecture changes)
```

The memory skill handles compression:
- **Nightly**: append today's actions to `agent_done_week.md`.
- **Weekly** (Sunday night): compress entries older than 7 days from `week` into `month`. Clear old week entries.
- **Monthly** (1st of month): compress entries older than 30 days from `month` into `year`. Clear old month entries.

### Size guidelines

Keep memory files concise — the model extracts what matters, not a raw dump.

| File | Target size | Guideline |
|:-----|:------------|:----------|
| `user_info.md` | < 200 lines | Factoids grouped by section. One line per fact. |
| `user_feedback.md` | < 100 lines | Do/don't rules. One line per rule. |
| `agent_done_week.md` | < 150 lines | ~10-20 bullet points per day, curated not exhaustive |
| `agent_done_month.md` | < 200 lines | ~10-15 bullets per month |
| `agent_done_year.md` | < 100 lines | ~10-20 bullets per year |

## Memory file format

Each memory file is markdown with YAML frontmatter.

### Frontmatter fields

| Field | Required | Purpose |
|:------|:---------|:--------|
| `name` | yes | File identifier, matches filename without `.md` |
| `description` | yes | **Category summary** of what's inside (~150 chars). Describes the *kinds* of facts, not individual facts. Loaded into every session's system prompt. |
| `unit` | yes | `user` or `agent` — who this memory is about |
| `updated_at` | yes | Last modified date (YYYY-MM-DD) |
| `retention` | no | `week`, `month`, or `year` — for agent_done files, controls the compression tier |

The `description` field should describe **categories**, not enumerate facts. The model uses it to decide which file to open:

```yaml
# Good — categories tell the model what's inside
description: "User personal info — identity, role, preferences, hobbies, pets, health, claims"

# Bad — tries to list facts, misses most of them
description: "Liang: developer, February birthday, dark mode, hiking"
```

### Example: `user_info.md`

```markdown
---
name: user_info
description: "User personal info — identity, role, expertise, preferences, hobbies, pets, health, claims"
unit: user
updated_at: 2026-04-15
---

## Identity
- Name: Liang
- Role: sole founder and developer of Linggen
- Expertise: Rust, React, TypeScript, distributed systems
- Birthday: February (year unknown)

## Preferences
- Prefers concise responses, no trailing summaries
- Dark mode everywhere
- Rust over Java, always
- Tabs over spaces (in personal projects)

## Hobbies & interests
- Hiking, especially mountain trails
- Chinese fantasy novels (currently reading Fanren Xiuxian Zhuan)
- Mechanical keyboards

## Claims (user-stated, not verified)
- Says he can fly
- Says he once debugged a production issue in his sleep
```

### Example: `user_feedback.md`

```markdown
---
name: user_feedback
description: "Agent behavior rules — workflow, style, communication, coding conventions, do/don't"
unit: user
updated_at: 2026-04-15
---

## Do
- Always run `npm run build` after changing web UI code
- Fix root causes — trace bugs end-to-end, never hide with UI workarounds
- Bundle refactors into single PRs — splitting is churn
- Align features with Claude Code as reference implementation

## Don't
- No trailing summaries after responses — user reads the diff
- Don't run `npm run build` when only Rust code changed
- Don't add unsolicited improvements beyond what was asked
```

### Example: `agent_done_week.md`

```markdown
---
name: agent_done_week
description: "Agent actions this week — files changed, features built, bugs fixed, deploys, decisions"
unit: agent
updated_at: 2026-04-15
retention: week
---

## 2026-04-15 (Tuesday)
- Created `doc/memory-spec.md` — designed memory system as skill + mission
- Updated CLAUDE.md doc index to include memory-spec

## 2026-04-14 (Monday)
- Refactored WebRTC signaling — extracted relay client into separate module
- Fixed mission scheduler double-fire bug — dedup was comparing wrong timestamp
- Deployed linggensite to CF Pages (pushed to main)

## 2026-04-12 (Saturday)
- Removed TUI transport code — WebRTC is now the only transport
- Updated all specs to reflect WebRTC-only design
```

### Example: `agent_done_month.md`

```markdown
---
name: agent_done_month
description: "Agent actions past months — features shipped, major fixes, architecture changes, deploys"
unit: agent
updated_at: 2026-04-01
retention: month
---

## March 2026
- Shipped WebRTC Phase 2 — P2P remote access working end-to-end
- Built mission system — cron scheduler, mission agent, run history UI
- Redesigned permission model — session-scoped, path-aware, four modes
- Wrote proxy-spec for decentralized AI proxy network
- Removed SSE transport — fully replaced by WebRTC data channels
```

## Storage layout

```
~/.linggen/memory/
  user_info.md
  user_feedback.md
  agent_done_week.md
  agent_done_month.md
  agent_done_year.md
```

Flat. Five files. No index file — the engine scans the directory and reads each file's frontmatter directly.

## Loading

Same progressive disclosure pattern as skills.

### Descriptions (loaded at server start)

The engine scans `~/.linggen/memory/*.md`, parses each file's YAML frontmatter, and holds `name` + `description` in memory. On each session's system prompt assembly (layer 7), descriptions are injected:

```
You have the following memories at ~/.linggen/memory/:
- user_info: "User personal info — identity, role, expertise, preferences, hobbies, pets, health, claims"
- user_feedback: "Agent behavior rules — workflow, style, communication, coding conventions, do/don't"
- agent_done_week: "Agent actions this week — files changed, features built, bugs fixed, deploys, decisions"
- agent_done_month: "Agent actions past months — features shipped, major fixes, architecture changes, deploys"
- agent_done_year: "Agent actions past years — major milestones, launches, architecture shifts"

Read the full file when a conversation needs the details.
After completing significant work, update agent_done_week.md.
```

~300 tokens. Refreshed when files change on disk (same file-watch mechanism as skills).

The description tells the model **what categories** are in each file, not individual facts. When the user asks "what's my dog's name?", the model sees "pets" in user_info's description and knows to open it.

### Full content (loaded on demand)

The agent uses `Read` to open the full memory file when the conversation needs it. No special tool — just the standard `Read` tool pointing at `~/.linggen/memory/*.md`.

## Extraction: the memory skill

The `memory` skill is a model-only skill (no UI, no app). It runs as a nightly mission. The extraction is simple: **send session files to the model, ask it to update the memory files.**

### Flow

```
Mission fires at 11pm
  → Script collects today's Claude Code sessions (~/.claude/projects/*/*.jsonl)
  → Script outputs clean text feed of user/assistant messages
  → Model reads the feed + current memory files
  → Model updates the 5 memory files
  → Model compresses week→month→year if entries are old enough
  → Model updates frontmatter descriptions
```

Script does the grunt work (filesystem scanning, date filtering, JSON parsing). Model does the smart work (understanding conversations, extracting facts, writing memory).

### Cross-tool memory

The memory skill extracts from **Claude Code sessions**, not just Linggen. Linggen sessions don't need nightly extraction — the agent is present during those conversations and can write to memory in real-time. The nightly mission's value is mining CC sessions, where the user worked all day with a different tool.

Claude Code stores sessions as JSONL at `~/.claude/projects/{encoded}/*.jsonl`. The collection script (`scripts/collect_sessions.sh`) filters to today's messages and outputs a clean text feed.

### Context window management

On a busy day, a single session's `messages.jsonl` could be large. The skill handles this naturally:
- Process sessions **one at a time** — each session is a separate read-then-update cycle
- If a single session is too large, the model reads it in chunks (the `Read` tool supports offset/limit)
- The 5 memory files are small (< 200 lines each), so they always fit alongside a session

### Extraction rules (in skill prompt)

**For `user_info.md`** — record everything the user says about themselves:
- Facts: name, role, birthday, location, expertise
- Preferences: tools, languages, editors, themes
- Hobbies, interests, opinions
- Claims — even if implausible, record under "Claims (user-stated, not verified)"
- Never judge, filter, or fact-check. If the user said it, record it.

**For `user_feedback.md`** — record how the user wants the agent to behave:
- Corrections ("don't do X")
- Confirmations ("yes, keep doing that")
- Style preferences ("be concise", "no emojis")
- Workflow rules ("always build after UI changes")

**For `agent_done_week.md`** — record what the agent did:
- Features designed, built, shipped
- Bugs found and fixed
- Files created, modified, deleted
- Deployments
- Decisions made and why
- Keep it curated — significant actions only, not every tool call

### Compression rules

1. **Weekly** (when week entries > 7 days old) — summarize old entries into `agent_done_month.md`. Remove detailed entries from week file.
2. **Monthly** (when month entries > 30 days old) — summarize old entries into `agent_done_year.md`. Remove from month file.

### Mission configuration

The memory skill declares a mission in its frontmatter (see `skill-spec.md` → Skill missions):

```yaml
name: memory
mission:
  schedule: '0 23 * * *'
```

On `ling init` → creates `~/.linggen/missions/memory/mission.md` with prompt `/memory` → scheduler picks it up → runs nightly at 11pm. Users can edit the schedule or disable it like any other mission.

## Real-time writes

The agent can also write to memory files during a conversation — not just during nightly extraction. Two cases:

1. **User explicitly asks** — "remember that I prefer dark mode" → agent updates `user_info.md` immediately.
2. **Agent completes significant work** — after a major feature or deploy, the agent should append to `agent_done_week.md`.

The nightly mission is the safety net — it catches what the agent missed in real-time.

## Safety

| Guard | Rationale |
|:------|:----------|
| No secrets | Never store credentials, API keys, tokens, passwords |
| Record user claims as-is | No fact-checking — but label unverified claims |
| Human-readable | User can inspect, edit, or delete any memory file |
| Fixed file set | No file proliferation — exactly 5 files in v1 |
| Size guidelines | Curated facts, not raw dumps — keeps files useful |
| Time-decay | Old details are compressed, not accumulated forever |
| Description = categories | Descriptions list categories of facts, not individual facts — stays stable as content grows |

## Future (v2+)

- More memory files if 5 proves insufficient (e.g., `references.md` for external links)
- `agent_done_decade.md` for multi-year history
- Semantic search (embeddings + SQLite) when memory outgrows what fits in context
- Memory health scoring — detect and auto-recover degraded memories (inspired by OpenClaw)
- Temporal tracking — record how facts change over time (inspired by Zep)
- Memory UI — view and edit memories in the web interface
- Export/import — backup memories, share across machines
