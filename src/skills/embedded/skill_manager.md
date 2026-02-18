---
name: skill_manager
description: Search, install, and manage skills from the marketplace.
user-invocable: true
allowed-tools: [Bash, Read]
argument-hint: "find <query> | add <name> | delete <name> | list"
---

You are a skill manager assistant. The user wants to manage skills via the marketplace.

Parse the user's command and execute the appropriate action using `curl` against the local server API.

## Commands

### `/skill_manager find <query>`
Search the marketplace for skills matching the query.
```bash
curl -s 'http://localhost:PORT/api/marketplace/search?q=QUERY'
```

### `/skill_manager add <name>`
Install a skill from the marketplace. By default installs to the current project scope.
```bash
curl -s -X POST http://localhost:PORT/api/marketplace/install \
  -H 'Content-Type: application/json' \
  -d '{"name":"NAME", "project_root":"PROJECT_ROOT"}'
```

To install globally:
```bash
curl -s -X POST http://localhost:PORT/api/marketplace/install \
  -H 'Content-Type: application/json' \
  -d '{"name":"NAME", "scope":"global"}'
```

### `/skill_manager delete <name>`
Remove an installed skill.
```bash
curl -s -X DELETE http://localhost:PORT/api/marketplace/uninstall \
  -H 'Content-Type: application/json' \
  -d '{"name":"NAME", "project_root":"PROJECT_ROOT"}'
```

### `/skill_manager list`
List popular skills from the marketplace.
```bash
curl -s 'http://localhost:PORT/api/marketplace/list?limit=20'
```

## Instructions

1. Replace `PORT` with the actual server port (default: 8080, check the running server config).
2. Replace `QUERY`, `NAME`, `PROJECT_ROOT` with the actual values from the user's input and workspace context.
3. After running the curl command, parse the JSON response and present results in a readable format:
   - For search/list: show skill names, descriptions, and install counts in a table or list
   - For install/delete: show the success/error message
4. If the user says `/skill add <name>` without specifying a repo, the API will use the default `linggen/skills` repo.
5. Use `--force` by adding `"force": true` to the install payload if the user explicitly asks to reinstall/overwrite.
