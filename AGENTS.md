---
name: linggen
description: "[Deprecated] Use the 'memory' skill for RAG/search and 'skiller' skill for marketplace. This skill is a compatibility pointer."
argument-hint: "[query]"
---

# Linggen Skill (Deprecated)

This skill has been split into two focused skills:

- **`memory`** — Semantic memory, RAG, code search, prompt enhancement, codebase indexing, and server management.
- **`skiller`** — Skill marketplace: search, install, browse library packs.

## Migration

The old `linggen` scripts still work but are no longer maintained. Use the new skills instead:

| Old (linggen) | New (memory / skiller) |
|---|---|
| `search_codebase.sh` | `memory/scripts/search_codebase.sh` |
| `query_codebase.sh` | `memory/scripts/query_codebase.sh` |
| `enhance_prompt.sh` | `memory/scripts/enhance_prompt.sh` |
| `memory_search_semantic.sh` | `memory/scripts/search_memory.sh` |
| `memory_fetch_by_meta.sh` | `memory/scripts/fetch_memory.sh` |
| `list_sources.sh` | `memory/scripts/list_sources.sh` |
| `get_status.sh` | `memory/scripts/server_status.sh` |
| `start_linggen_server.sh` | `memory/scripts/start_server.sh` |
| `lookup_skills.sh` | `skiller/scripts/lookup_skills.sh` |
| `install_online_skill.sh` | `skiller/scripts/install_skill.sh` |
| `list_library_packs.sh` | `skiller/scripts/list_library_packs.sh` |
| `get_library_pack.sh` | `skiller/scripts/get_library_pack.sh` |
