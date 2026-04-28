---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Memory

Persistent knowledge that travels with the user across sessions — about
who they are, how they want to work, the decisions they've made, and the
people and projects in their life. Memory must help every kind of user
(software engineer, musician, language learner, cook), not just coders.
It is **append-mostly**: the store grows when there's signal, and shrinks
only when a fixed rule fires or the user agrees during a live session.

## 1. What is memory

Memory is the **user's biography across sessions**, not the agent's
notebook of what it did.

**What it is:**

- The user's identity, role, language, life context.
- How the user wants the agent to work — preferences, commitments, do/don't rules.
- Decisions the user made and the reasoning behind them.
- Cross-project gotchas the user genuinely re-encounters.

**What it isn't:**

- An activity log of what the agent worked on (git history records that).
- A snapshot of the codebase (the files are the source of truth).
- A conversation transcript (the session store records that).

Two places knowledge can live, picked by how the agent needs to see it:

- **Core memory** — universals about the person, always inlined into
  every session's system prompt. Tiny by design.
- **RAG memory** — everything else durable, queried on demand by the
  agent or surfaced at turn start.

If a candidate doesn't earn a place in either layer, **drop it.**
Memory does not write to project files (`<project>/AGENTS.md`,
`CLAUDE.md`, source code, docs). Those are user-curated; the agent
will read the file directly when it needs the content. Mixing
auto-extracted facts into version-controlled files creates churn the
user didn't author and conflates two distinct authorities.

The split between the two layers matters because *how a fact is loaded*
determines *whether the agent can use it*. Universals go in core
(deterministic, every session). Things the agent needs sometimes go in
RAG (probabilistic search). Anything that doesn't justify either is
not memory — drop.

## 2. How memory works

### Three components

- **Core layer.** Two markdown files (`identity.md`, `style.md`) on
  disk under `~/.linggen/memory/`. Engine-inlined into every session's
  stable system prompt. The user, the agent, and the extraction
  pipeline all write the same way — by editing markdown — and every
  writer goes through normal file-permission plumbing. High bar for
  entry; the whole pair stays ~30–50 lines combined. No activity logs,
  no project-specific rules, no meta-feedback about the memory system
  itself.

- **Skill layer.** A pluggable memory skill exposes two verb-dispatched
  tools: `Memory_query` (verbs: `get`, `search`, `list`) and
  `Memory_write` (verbs: `add`, `update`, `delete`). The default skill
  ships a local RAG store; the contract is engine-owned and
  **host-agnostic**, so the same skill works inside Linggen, inside
  Claude Code, and inside any agent that honors the `provides:
  [memory]` declaration. Swapping skills changes where data lives, not
  what tools the model sees. Bulk forget is deliberately not on the
  model surface — it's user-initiated via the dashboard or `ling-mem
  forget` CLI.

- **Mission layer.** An offline extraction agent that reads recent
  transcripts, applies the durability rules in §4, and appends rows.
  Runs on a cron in Linggen via the mission scheduler. Runs on user
  demand in Claude Code — no scheduler, invoked via the skill's slash
  command or chat trigger. Same prompt, same rules, different runtimes.

### Three flows

- **In-session writes.** When the user explicitly says *"remember X"* or
  commits to a behavior the agent should keep doing, the agent writes —
  `Edit` to the core files for universals, `Memory_write` for scoped
  facts. A periodic mid-session nudge asks the model whether the recent
  exchange produced anything durable, so opportunities aren't missed.
  Cadence is configurable; nudge wording is owned by the active skill.

- **Offline mission — append + mechanical maintenance.** The mission
  appends new rows and runs *mechanical* cleanup: dedup near-rephrases
  to clearer wording, extend a row's contexts/tags when new evidence
  fits, add `supersedes` links between rows, retire rows that meet a
  fixed obsolescence rule (session-arc leak, hard TTL, completed
  supersedes chain). It does **not** synthesize, generalize, or resolve
  contradictions — those judgments are reserved for live retrieval (see
  rule 5 in §3).

- **Retrieval doubles as maintenance.** Push at turn start (the user's
  message is searched silently and matches prefix the user turn), pull
  on demand (the model calls `Memory_query`). When matching rows
  surface, the agent does three jobs at once:

  1. **Synthesize.** Multiple rows on the same theme reconcile in
     prose — *"From memory: …"*. Live and visible.
  2. **Detect drift.** Notice rows that contradict newer info,
     contradict what the user just said, or have aged into staleness.
  3. **Propose action.** Surface the drift to the user. On confirm, the
     agent issues `Memory_write` (verb=update to merge, or verb=delete
     to forget a single row). Bulk forget is not on the model tool
     surface — it's user-initiated via the dashboard or CLI.

  Maintenance is encountered, not scheduled. Reconciliation, conflict
  resolution, and expiration all fire when retrieval pulls the relevant
  rows and the user is right there to confirm or correct.

### Routing

Two destinations for candidates that earn a place; everything else is
dropped. **Memory never writes to project files** (`<project>/AGENTS.md`,
`CLAUDE.md`, source code, docs).

- **Universal-about-person** (true in any project, any time) → core
  markdown (`identity.md` or `style.md`).
- **Everything else durable** (cross-project intent, decision,
  preference, life context, cross-project learning) → RAG via
  `Memory_write({verb: "add", ...})`.
- **Anything else, including project-internal implementation detail**
  → drop. The agent reads the project's own files (code, AGENTS.md
  authored by the user) when it needs to know.

Most candidates drop — the durability rules in §4 reject more than they
accept.

## 3. Design rules

Six anchors. When a design decision is unclear, they break the tie.

**1. Human in the loop for destructive operations.**

Forgets, merges of distinct facts, and any rewrite that changes the
meaning of a row require explicit user confirmation. Append and
mechanical maintenance are the safe defaults. Never silently overwrite
a user-stated fact based on inference.

**2. The user is the source of truth.**

Memory records what the user said and what the agent observed in the
user's presence. The agent does not invent details — names, dates,
breeds, project terms — to make an entry feel complete. Fabricated
specifics mislead every future retrieval. If the user said *"a cat,"*
the row says *"a cat,"* not *"a cat named [made-up name]."*

**3. The file beats the memory.**

Anything re-derivable from workspace files (code, configs, project docs,
the project's own `AGENTS.md` / `CLAUDE.md`) doesn't belong in the memory
store. The file is the source of truth; memory storing the same content
creates a stale copy that rots on every refactor. Project-internal
architecture and conventions stay in those user-curated files. Memory
neither duplicates them nor writes back to them — drop the candidate.

**4. Curate, don't accumulate.**

The store grows with genuinely durable signal — the user's life, work,
and decisions accumulate over years, and that growth is the whole point.
What it must not do is drift: expired facts get retired, conflicting
rows get reconciled, noisy duplicates get merged. Net value goes up over
time, not row count alone.

**5. Live for synthesis, offline for mechanics.**

Maintenance happens in two places, split by the kind of judgment it
needs.

*Mechanical maintenance* — exact-rephrase dedup, extending contexts/tags,
adding `supersedes` links, retiring rows that meet a fixed obsolescence
rule — runs offline in the mission. The decisions are mechanical: a
rule fires, the action follows.

*Semantic maintenance* — merging facts into a story, generalizing
patterns into rules, choosing between contradicting rows — runs only in
the live session. The user is there to see the synthesis and correct it
before anything commits.

Bulk forget is user-initiated only, regardless of where it would run.

| Operation | Where | Why |
|:--|:--|:--|
| Append a new row | Offline mission OR live session | Pure additive |
| Exact-rephrase dedup (clearer wording on a near-duplicate) | Offline OR live | Mechanical — pick the better string |
| Extend `contexts[]` / `tags[]` from new evidence | Offline OR live | Mechanical — array union |
| Add a `supersedes` link between two rows | Offline OR live | Metadata only |
| Retire by fixed obsolescence rule (session-arc leak, hard TTL, completed supersedes chain) | Offline OR live | Mechanical — the criterion is a fixed rule |
| Merge distinct facts into a synthesized story | **Live only** | Content rewrite — hallucination risk |
| Generalize utterances into a "user always X" rule | **Live only** | Over-fit risk |
| Resolve a contradiction between rows | **Live only** | Needs context the offline run may lack |
| Bulk delete by filter | **User-initiated only — not a model tool** | The whole point is intentional cleanup; users invoke via the dashboard or `ling-mem forget` CLI. The model can iterate `Memory_query` → `Memory_write({verb: "delete"})` for small sets when explicitly asked. |

**6. Never store secrets, at any layer.**

Credentials, API keys, tokens, passwords, embedded auth in URLs — out
of memory entirely. The credential never enters any memory layer.
Memory does not write to project files, so there is no secondary
destination to consider. If the user wants to record a *gotcha* about
the credential (*"don't copy this URL into cloud configs"*), that's a
hand-edit they make to their own project file; memory does not author
those.

## 4. What's worth remembering

Memory's value is signal density, not row count. Three rules decide
whether a candidate earns its place. Routing (core markdown vs RAG) is
the §2 concern — these rules answer only the binary question:
**should this be saved at all?** Memory never writes to project files;
candidates that don't fit core or RAG are dropped.

**1. Don't memorize what lives in workspace files.**

Code, configs, READMEs, project docs, the user's own `AGENTS.md` /
`CLAUDE.md` — the agent reads them when it needs them. Putting the
same content in memory creates a second copy that rots the moment the
file changes. The file is the source of truth; memory stays out of its
way.

> *"In repo1, the planner module exposes a facade that returns a
> context object per tick"* — **skip.** The agent will read the planner
> sources next time it matters. Memory does not auto-write to the
> project's `AGENTS.md` either — that file is user-curated. If the
> architectural intent is load-bearing for future work, the user can
> hand-edit it themselves.

This rule kills most "the codebase has X" candidates from offline scans.
If a fact can be re-derived by reading one or two files, it doesn't
belong in memory.

**2. User-stated preferences need a confidence gate.**

Not every *"the user said …"* line is durable. Distinguish three cases:

- **Save** — the user is correcting how the *agent* should work, with
  commitment language and cross-project reach:
  > *"I want the agent to always keep UI and server aligned, don't leave
  > one half-done into the next task."*

  This shapes agent behavior beyond a single repo. Record as
  `preference`.

- **Skip** — the user is making a single architectural call, true today
  and possibly reversed next month:
  > *"We should decouple layer 1 from the core engine."*

  Rot-prone. Belongs in design notes or the PR description — not memory.
  Memory does not auto-write to the project's `AGENTS.md` either; if
  the user wants it captured there, that's their hand-edit.

- **Record utterances; synthesize at retrieval, not at extraction.**
  When a pattern emerges across many sessions — repeated *"split this
  module"*, *"factor out Y"*, *"decouple X"* — the extractor still
  appends each one as its own row. It does **not** try to mint a
  higher-order preference like *"user prefers continual decoupling."*

  Synthesis happens live: when retrieval pulls several rows on the same
  theme, the agent reconciles them in prose — *"From memory: you've
  raised decoupling concerns in 5 sessions; pattern is X."* The user
  sees the generalization the moment it's made and can correct it.

  Why not synthesize offline: generalizing scattered utterances into a
  permanent rule is exactly where the agent over-fits — one strong rant
  can mint a "user always wants Y" claim that misrepresents them
  forever. Append-and-reconcile keeps the raw evidence and forces
  synthesis to happen in the user's presence.

  The proactive case — surfacing a pattern the user wouldn't have
  queried for — belongs in the dashboard, not the extractor. The
  dashboard can run cluster analysis on demand and offer *"we see N
  similar utterances about X — promote to a preference?"*, with the
  user confirming before any new typed row is written.

**3. User-only knowledge — record, then maintain.**

Facts only the user can supply: life context, history, relationships,
dates, equipment, the people and animals around them. The agent has no
other path to learn these, so when the user volunteers one, save it.
But every such fact ages, so:

- **Stamp ages relative to a date, not to "now".**
  > *"I have a 3-year-old cat"* → save as *"User has a cat, age 3 as of
  > 2026-04-27"*, not *"the cat is 3 years old."* Without the as-of
  > date, "3 years old" silently rots into "still 3 years old" forever.

  Record only what the user said. Don't invent a name, breed, or any
  other detail to make the entry feel complete — fabricated specifics
  will mislead every future retrieval.

- **Append at extraction; reconcile at retrieval.** When the user
  revises a fact, the offline extractor adds a new timestamped row — it
  does **not** overwrite the existing one. Reconciliation happens at
  read time: when multiple matching rows surface, the agent merges them
  in the response, ordered by timestamp, and the user sees the synthesis
  live (and can correct it on the spot).
  > Stored: *"User has a cat"* (2024). Later: *"When I relocated, I
  > left the cat with a friend"* (2026). Retrieval surfaces both;
  > the agent renders *"From memory: you had a cat that you left with a
  > friend during your 2026 relocation."*

  Why append rather than merge-at-write: a bad merge from the offline
  pipeline silently corrupts good data, and the user isn't there to
  catch it. Append-only keeps every original utterance recoverable; the
  agent's live synthesis is correctable in the same conversation. The
  raw timestamps also let the agent answer *"when did I get the cat?"* /
  *"how long did I have it?"* — questions a flattened row destroys.

  Optional hint: when the extractor is highly confident the new row
  supersedes an earlier one, it can tag the new row with a `supersedes:
  <id>` link. That's metadata for retrieval ranking, not a destructive
  edit.

  Destructive consolidation (actually deleting the old row) is
  user-initiated only — *"clean up my cat memory"* or a dashboard
  review. The agent proposes the merged version; the user approves
  before any write.

The extractor should pre-flag candidates that semantically overlap with
existing rows so the live agent (and the dashboard) sees the cluster at
retrieval time and can act on it with the user.

## Implementation pointers

This spec stays out of implementation. Where the wires actually run:

- **Tool dispatch and capability routing** — `tool-spec.md`,
  `skill-spec.md`.
- **Filesystem layout under `~/.linggen/`** — `storage-spec.md`.
- **Permission tiers and path scopes** — `permission-spec.md`.
- **Default RAG engine schema (locked shape)** —
  [linggen-memory/DESIGN.md](../../linggen-memory/DESIGN.md).
- **Session prompt assembly and the `include_memory` flag** —
  `session-spec.md`.

What Linggen assumes from any provider, regardless of implementation:
stable opaque row identity (Linggen never parses ids); free-form
many-to-many `contexts[]`; closed-enum `type`; provider-internal ranking
and embedding. Schema-versioned rows with explicit migrations. Daemons
bind localhost-only and are never exposed to remote consumers.

The default skill (`ling-mem`) ships two surfaces with separate
responsibilities: a **data UI** for row-level browsing (read-only on
open, every change explicit) and a **skill dashboard** for higher-level
summaries, extraction controls, and the on-demand cluster-analysis
described in §4 rule 2. The split is responsibility, not packaging.

## Known limitations

This v1 design is opinionated and ships with intentional gaps. Worth
naming so the next pass knows what to fix:

- **No row-level confidence calibration.** Old rows from early-version
  extractors sit equal to fresh user-confirmed rows. A `confidence` or
  `last_verified` field would let retrieval prioritize correctly.
- **No scale story past ~10⁴ rows.** Append-mostly works at small scale;
  at large scale the embedding index needs re-tiering and retrieval
  needs cluster ranking.
- **Privacy isolation is by convention, not enforcement.** A `work/` vs
  `personal/` context tag relies on the agent filtering correctly. No
  hard cross-context boundary.
- **Live maintenance is a property the implementation must earn.** "The
  agent will detect drift and propose forgets" depends on prompting and
  evals — it's not a guaranteed property of the design alone.
- **Cold-start has no importer.** New users start empty; there's no
  path to bootstrap from existing notes or prior tools.
- **Proactive synthesis lives entirely in the dashboard.** Users who
  don't open it never get pattern-surfacing value.

These are deferred deliberately, not overlooked.

## Future

- **Cross-device sync** — exports + git is v1; real sync is P2P via
  Linggen's WebRTC transport.
- **Temporal reasoning** — entity-time graph queries inspired by Zep;
  `supersedes` is the structural foothold.
- **Multi-provider** — a local fast skill + a cloud/persistent skill
  running simultaneously with merged results.
- **`Memory_archive`** — soft-forget (hidden from default search but
  recoverable). Eighth tool to add when the default skill supports it.
- **Confidence calibration** — surface row freshness/age in the UI;
  use it to rank retrieval.
