use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Component, Path};

pub(super) fn build_globset(globs: Option<&[String]>) -> Result<Option<GlobSet>> {
    let Some(globs) = globs else {
        return Ok(None);
    };
    if globs.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for g in globs {
        builder.add(Glob::new(g)?);
    }
    Ok(Some(builder.build()?))
}

pub(super) fn sanitize_rel_path(root: &Path, path: &str) -> Result<String> {
    if path.is_empty() {
        anyhow::bail!("empty path");
    }
    let raw = Path::new(path);
    let rel_path = if raw.is_absolute() {
        raw.strip_prefix(root)
            .map_err(|_| anyhow::anyhow!("absolute path must be inside workspace root"))?
            .to_path_buf()
    } else {
        raw.to_path_buf()
    };

    if rel_path.as_os_str().is_empty() {
        anyhow::bail!("empty path");
    }
    if rel_path
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        anyhow::bail!("path traversal not allowed");
    }
    if rel_path
        .components()
        .any(|c| matches!(c, Component::RootDir | Component::Prefix(_)))
    {
        anyhow::bail!("path must resolve inside workspace root");
    }

    Ok(rel_path.to_string_lossy().to_string())
}

pub(super) fn to_rel_string(root: &Path, path: &Path) -> Result<String> {
    let rel = path.strip_prefix(root)?;
    Ok(rel.to_string_lossy().to_string())
}

pub(crate) fn summarize_tool_args(tool: &str, args: &Value) -> String {
    let mut safe_args = args.clone();
    if let Some(obj) = safe_args.as_object_mut() {
        match tool {
            "Write" => {
                if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
                    let byte_len = content.len();
                    let line_count = content.lines().count();
                    obj.insert(
                        "content".to_string(),
                        serde_json::json!(format!(
                            "<omitted:{} bytes, {} lines>",
                            byte_len, line_count
                        )),
                    );
                }
            }
            "Edit" => {
                for key in ["old_string", "new_string", "old", "new", "old_text", "new_text", "oldText", "newText", "search", "replace", "from", "to"] {
                    if let Some(content) = obj.get(key).and_then(|v| v.as_str()) {
                        let byte_len = content.len();
                        let line_count = content.lines().count();
                        obj.insert(
                            key.to_string(),
                            serde_json::json!(format!(
                                "<omitted:{} bytes, {} lines>",
                                byte_len, line_count
                            )),
                        );
                    }
                }
            }
            "Bash" => {
                if let Some(cmd) = obj.get("cmd").and_then(|v| v.as_str()) {
                    let preview = if cmd.len() > 160 {
                        // Find a char boundary at or before 160 to avoid UTF-8 panic.
                        let end = cmd
                            .char_indices()
                            .map(|(i, _)| i)
                            .take_while(|&i| i <= 160)
                            .last()
                            .unwrap_or(0);
                        format!("{}... (truncated, {} chars)", &cmd[..end], cmd.len())
                    } else {
                        cmd.to_string()
                    };
                    obj.insert("cmd".to_string(), serde_json::json!(preview));
                }
            }
            _ => {}
        }
    }
    safe_args.to_string()
}

pub(crate) fn normalize_tool_args(tool: &str, args: &Value) -> Value {
    let mut normalized = args.clone();
    if let Some(obj) = normalized.as_object_mut() {
        if matches!(tool, "Bash") && !obj.contains_key("cmd") {
            if let Some(command) = obj.get("command").cloned() {
                obj.insert("cmd".to_string(), command);
            }
        }

        if matches!(tool, "Read" | "Write" | "Edit") && !obj.contains_key("path") {
            if let Some(fp) = obj.get("filepath").cloned() {
                obj.insert("path".to_string(), fp);
            } else if let Some(file) = obj.get("file").cloned() {
                obj.insert("path".to_string(), file);
            }
        }

        if matches!(tool, "Edit") {
            if !obj.contains_key("old_string") {
                if let Some(v) = obj.get("old").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("old_text").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("oldText").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("search").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("from").cloned() {
                    obj.insert("old_string".to_string(), v);
                }
            }
            if !obj.contains_key("new_string") {
                if let Some(v) = obj.get("new").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("new_text").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("newText").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("replace").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("to").cloned() {
                    obj.insert("new_string".to_string(), v);
                }
            }
            if !obj.contains_key("replace_all") {
                if let Some(v) = obj.get("all").cloned() {
                    obj.insert("replace_all".to_string(), v);
                }
            }
        }

        if matches!(tool, "Grep") && !obj.contains_key("query") {
            if let Some(path) = obj.get("path").cloned() {
                obj.insert("query".to_string(), path);
            } else if let Some(fp) = obj.get("filepath").cloned() {
                obj.insert("query".to_string(), fp);
            } else if let Some(file) = obj.get("file").cloned() {
                obj.insert("query".to_string(), file);
            }
        }

        if matches!(tool, "Grep" | "Glob")
            && obj.get("globs").map(|v| v.is_string()).unwrap_or(false)
        {
            if let Some(glob) = obj.get("globs").and_then(|v| v.as_str()) {
                obj.insert("globs".to_string(), serde_json::json!([glob]));
            }
        }
    }
    normalized
}

pub(crate) fn validate_shell_command(cmd: &str) -> Result<()> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty command");
    }

    // Disallow common shell injection patterns.
    for banned in ["$(", "`", "\n", "\r", "<(", ">("] {
        if trimmed.contains(banned) {
            anyhow::bail!("command contains disallowed shell construct: {}", banned);
        }
    }
    // Block output redirection (but allow `>` inside grep patterns etc. via `--`).
    // We only block bare `>` or `>>` that appear as shell operators.
    for op in [" > ", " >> ", "\t>\t", "\t>>\t", " >|"] {
        if trimmed.contains(op) {
            anyhow::bail!("command contains disallowed shell redirection");
        }
    }
    // Block input redirection `< file`.
    if trimmed.contains(" < ") {
        anyhow::bail!("command contains disallowed shell redirection");
    }

    let allowed: HashSet<&str> = [
        "ls", "pwd", "cat", "head", "tail", "wc", "cut", "sort", "uniq", "tr", "sed", "awk",
        "find", "fd", "rg", "grep", "git", "cargo", "rustc", "npm", "pnpm", "yarn", "node",
        "python", "python3", "pip", "pip3", "pytest", "go", "make", "just",
        "bash", "sh", "curl", "jq",
    ]
    .into_iter()
    .collect();

    for segment in split_shell_segments(trimmed) {
        let token = first_segment_token(segment)
            .ok_or_else(|| anyhow::anyhow!("invalid command segment: '{}'", segment))?;
        if !allowed.contains(token) {
            anyhow::bail!(
                "Command not allowed: {} (allowed tools are code/search/build/test commands)",
                token
            );
        }
    }

    Ok(())
}

fn split_shell_segments(cmd: &str) -> Vec<&str> {
    cmd.split(|c| c == '|' || c == ';')
        .flat_map(|part| part.split("&&"))
        .flat_map(|part| part.split("||"))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

fn first_segment_token(segment: &str) -> Option<&str> {
    segment
        .split_whitespace()
        .next()
        .map(|token| token.trim_start_matches('('))
}

pub fn canonical_tool_name(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "Glob" => "Glob",
        "Read" => "Read",
        "Grep" => "Grep",
        "Write" => "Write",
        "Edit" => "Edit",
        "Bash" => "Bash",
        "capture_screenshot" => "capture_screenshot",
        "lock_paths" => "lock_paths",
        "unlock_paths" => "unlock_paths",
        "Task" | "delegate_to_agent" => "Task",
        "WebSearch" | "web_search" => "WebSearch",
        "WebFetch" | "web_fetch" => "WebFetch",
        "Skill" | "skill" => "Skill",
        "AskUser" | "ask_user" => "AskUser",
        "ExitPlanMode" | "exit_plan_mode" => "ExitPlanMode",
        _ => return None,
    })
}

pub(crate) fn full_tool_schema_entries() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": "Glob",
            "args": {"globs": "string[]?", "max_results": "number?"},
            "returns": "string[]"
        }),
        serde_json::json!({
            "name": "Read",
            "args": {"path": "string", "max_bytes": "number?", "line_range": "[number,number]?"},
            "returns": "{path,content,truncated}",
            "notes": "Path aliases accepted: path, file, filepath."
        }),
        serde_json::json!({
            "name": "Grep",
            "args": {"query": "string", "globs": "string[]?", "max_results": "number?"},
            "returns": "{matches:[{path,line,snippet}]}",
            "notes": "Query aliases accepted: query, path, file, filepath."
        }),
        serde_json::json!({
            "name": "Write",
            "args": {"path": "string", "content": "string"},
            "returns": "success",
            "notes": "Path aliases accepted: path, file, filepath."
        }),
        serde_json::json!({
            "name": "Edit",
            "args": {"path": "string", "old_string": "string", "new_string": "string", "replace_all": "boolean?"},
            "returns": "success",
            "notes": "Applies an exact string replacement. Path aliases accepted: path, file, filepath."
        }),
        serde_json::json!({
            "name": "Bash",
            "args": {"cmd": "string", "timeout_ms": "number?"},
            "returns": "{exit_code,stdout,stderr}",
            "notes": "Runs allowlisted dev/search/build shell commands with per-segment validation. Command alias accepted: command."
        }),
        serde_json::json!({
            "name": "capture_screenshot",
            "args": {"url": "string", "delay_ms": "number?"},
            "returns": "{url,base64}"
        }),
        serde_json::json!({
            "name": "Task",
            "args": {"target_agent_id": "string", "task": "string"},
            "returns": "{agent_outcome}",
            "notes": "Delegates a task to another agent. Subject to max delegation depth."
        }),
        serde_json::json!({
            "name": "WebSearch",
            "args": {"query": "string", "max_results": "number?"},
            "returns": "{results:[{title,url,snippet}]}",
            "notes": "Search the web via DuckDuckGo. Default 5 results, max 10."
        }),
        serde_json::json!({
            "name": "WebFetch",
            "args": {"url": "string", "max_bytes": "number?"},
            "returns": "{url,content,content_type,truncated}",
            "notes": "Fetch a URL and return its content as text. HTML is stripped of tags. Default max 100KB."
        }),
        serde_json::json!({
            "name": "Skill",
            "args": {"skill": "string", "args": "string?"},
            "returns": "string",
            "notes": "Invoke a skill by name. Returns the skill's full instructions. Pass optional args for the skill."
        }),
        serde_json::json!({
            "name": "AskUser",
            "args": {
                "questions": "[{question: string, header: string, options: [{label: string, description?: string, preview?: string}], multi_select?: boolean}]"
            },
            "returns": "{answers: [{question_index: number, selected: string[], custom_text?: string}]}",
            "notes": "Ask user 1-4 structured questions with 2-4 options each. User can always type custom text via 'Other'. Blocks until response (5 min timeout). Not available in sub-agents."
        }),
        serde_json::json!({
            "name": "ExitPlanMode",
            "args": {},
            "returns": "success",
            "notes": "Signal that your plan is complete and ready for user review. Call this after researching the codebase and writing your plan in your response text."
        }),
    ]
}
