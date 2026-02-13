use crate::engine::AgentOutcome;
use crate::server::{ServerEvent, ServerState};
use std::sync::Arc;
use tokio::sync::broadcast;

pub(crate) fn queue_key(project_root: &str, session_id: &str, agent_id: &str) -> String {
    format!("{project_root}|{session_id}|{agent_id}")
}

pub(crate) fn queue_preview(message: &str) -> String {
    const LIMIT: usize = 100;
    let trimmed = message.trim();
    if trimmed.len() <= LIMIT {
        trimmed.to_string()
    } else {
        format!("{}...", &trimmed[..LIMIT])
    }
}

pub(crate) fn sanitize_tool_args_for_display(
    tool: &str,
    args: &serde_json::Value,
) -> serde_json::Value {
    let mut safe = args.clone();
    if let Some(obj) = safe.as_object_mut() {
        if matches!(tool, "Write") {
            if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
                let bytes = content.len();
                let lines = content.lines().count();
                obj.insert(
                    "content".to_string(),
                    serde_json::json!(format!("<omitted:{} bytes, {} lines>", bytes, lines)),
                );
            }
        } else if matches!(tool, "Edit") {
            for key in ["old_string", "new_string", "old", "new", "old_text", "new_text", "oldText", "newText", "search", "replace", "from", "to"] {
                if let Some(content) = obj.get(key).and_then(|v| v.as_str()) {
                    let bytes = content.len();
                    let lines = content.lines().count();
                    obj.insert(
                        key.to_string(),
                        serde_json::json!(format!("<omitted:{} bytes, {} lines>", bytes, lines)),
                    );
                }
            }
        }
    }
    safe
}

pub(crate) fn extract_tool_path_arg(args: &serde_json::Value) -> Option<String> {
    args.get("path")
        .or_else(|| args.get("file"))
        .or_else(|| args.get("filepath"))
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

#[derive(Debug, Clone)]
struct ToolCallForUi {
    name: String,
    args: Option<serde_json::Value>,
}

fn parse_tool_call_from_json_line(line: &str) -> Option<ToolCallForUi> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    if parsed.get("type")?.as_str()? == "tool" {
        let tool = parsed
            .get("tool")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())?;
        return Some(ToolCallForUi {
            name: tool,
            args: parsed.get("args").cloned(),
        });
    }
    let kind = parsed.get("type").and_then(|v| v.as_str())?;
    if matches!(kind, "finalize_task") {
        return None;
    }
    if parsed.get("args").and_then(|v| v.as_object()).is_some() {
        return Some(ToolCallForUi {
            name: kind.to_string(),
            args: parsed.get("args").cloned(),
        });
    }
    None
}

fn parse_tool_name_from_result_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("tool ") {
        return None;
    }
    let rest = trimmed.get(5..)?.trim();
    let (name, _) = rest.split_once(':')?;
    let clean = name.trim();
    if clean.is_empty() {
        None
    } else {
        Some(clean.to_string())
    }
}

fn preview_value(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let preview: String = value.chars().take(max_chars).collect();
        format!("{preview}... ({len} chars)", len = value.chars().count())
    }
}

fn basename(value: &str) -> String {
    let normalized = value.trim().replace('\\', "/");
    normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .last()
        .map(|v| v.to_string())
        .unwrap_or_else(|| normalized.to_string())
}

fn first_string_arg(args: Option<&serde_json::Value>, keys: &[&str]) -> Option<String> {
    let obj = args.and_then(|v| v.as_object())?;
    for key in keys {
        if let Some(value) = obj.get(*key).and_then(|v| v.as_str()) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolStatusPhase {
    Start,
    Done,
    Failed,
}

pub(crate) fn tool_status_line(
    tool: &str,
    args: Option<&serde_json::Value>,
    phase: ToolStatusPhase,
) -> String {
    let name = tool.trim().to_lowercase();
    let read_path = first_string_arg(args, &["path", "file", "filepath"])
        .map(|path| preview_value(&basename(&path), 140));
    let bash_cmd = first_string_arg(args, &["cmd", "command"]).map(|cmd| preview_value(&cmd, 140));
    let grep_query =
        first_string_arg(args, &["query", "pattern", "q"]).map(|query| preview_value(&query, 140));
    let delegate_target = first_string_arg(args, &["target_agent_id"]);
    let glob_preview = args
        .and_then(|v| v.get("globs"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str())
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .map(|values| preview_value(&values.join(", "), 140));

    match name.as_str() {
        "read" => match phase {
            ToolStatusPhase::Start => read_path
                .map(|path| format!("Reading file: {path}"))
                .unwrap_or_else(|| "Reading file...".to_string()),
            ToolStatusPhase::Done => read_path
                .map(|path| format!("Read file: {path}"))
                .unwrap_or_else(|| "Read file".to_string()),
            ToolStatusPhase::Failed => read_path
                .map(|path| format!("Read failed: {path}"))
                .unwrap_or_else(|| "Read failed".to_string()),
        },
        "write" => match phase {
            ToolStatusPhase::Start => read_path
                .map(|path| format!("Writing file: {path}"))
                .unwrap_or_else(|| "Writing file...".to_string()),
            ToolStatusPhase::Done => read_path
                .map(|path| format!("Wrote file: {path}"))
                .unwrap_or_else(|| "Wrote file".to_string()),
            ToolStatusPhase::Failed => read_path
                .map(|path| format!("Write failed: {path}"))
                .unwrap_or_else(|| "Write failed".to_string()),
        },
        "edit" => match phase {
            ToolStatusPhase::Start => read_path
                .map(|path| format!("Editing file: {path}"))
                .unwrap_or_else(|| "Editing file...".to_string()),
            ToolStatusPhase::Done => read_path
                .map(|path| format!("Edited file: {path}"))
                .unwrap_or_else(|| "Edited file".to_string()),
            ToolStatusPhase::Failed => read_path
                .map(|path| format!("Edit failed: {path}"))
                .unwrap_or_else(|| "Edit failed".to_string()),
        },
        "bash" => match phase {
            ToolStatusPhase::Start => bash_cmd
                .map(|cmd| format!("Running command: {cmd}"))
                .unwrap_or_else(|| "Running command...".to_string()),
            ToolStatusPhase::Done => bash_cmd
                .map(|cmd| format!("Ran command: {cmd}"))
                .unwrap_or_else(|| "Ran command".to_string()),
            ToolStatusPhase::Failed => bash_cmd
                .map(|cmd| format!("Command failed: {cmd}"))
                .unwrap_or_else(|| "Command failed".to_string()),
        },
        "grep" => match phase {
            ToolStatusPhase::Start => grep_query
                .map(|query| format!("Searching: {query}"))
                .unwrap_or_else(|| "Searching...".to_string()),
            ToolStatusPhase::Done => grep_query
                .map(|query| format!("Searched: {query}"))
                .unwrap_or_else(|| "Searched".to_string()),
            ToolStatusPhase::Failed => grep_query
                .map(|query| format!("Search failed: {query}"))
                .unwrap_or_else(|| "Search failed".to_string()),
        },
        "glob" => match phase {
            ToolStatusPhase::Start => glob_preview
                .map(|globs| format!("Listing files: {globs}"))
                .unwrap_or_else(|| "Listing files...".to_string()),
            ToolStatusPhase::Done => glob_preview
                .map(|globs| format!("Listed files: {globs}"))
                .unwrap_or_else(|| "Listed files".to_string()),
            ToolStatusPhase::Failed => glob_preview
                .map(|globs| format!("List files failed: {globs}"))
                .unwrap_or_else(|| "List files failed".to_string()),
        },
        "delegate_to_agent" => match phase {
            ToolStatusPhase::Start => delegate_target
                .map(|target| format!("Delegating to subagent: {target}"))
                .unwrap_or_else(|| "Delegating...".to_string()),
            ToolStatusPhase::Done => delegate_target
                .map(|target| format!("Delegated to subagent: {target}"))
                .unwrap_or_else(|| "Delegated to subagent".to_string()),
            ToolStatusPhase::Failed => delegate_target
                .map(|target| format!("Delegation failed: {target}"))
                .unwrap_or_else(|| "Delegation failed".to_string()),
        },
        "" => match phase {
            ToolStatusPhase::Start => "Calling tool...".to_string(),
            ToolStatusPhase::Done => "Used tool".to_string(),
            ToolStatusPhase::Failed => "Tool failed".to_string(),
        },
        _ => match phase {
            ToolStatusPhase::Start => format!("Calling tool: {}", tool.trim()),
            ToolStatusPhase::Done => format!("Used tool: {}", tool.trim()),
            ToolStatusPhase::Failed => format!("Tool failed: {}", tool.trim()),
        },
    }
}

fn status_line_for_tool_call(tool_call: Option<&ToolCallForUi>) -> String {
    let Some(tool_call) = tool_call else {
        return "Calling tool...".to_string();
    };

    tool_status_line(
        &tool_call.name,
        tool_call.args.as_ref(),
        ToolStatusPhase::Start,
    )
}

pub(crate) fn sanitize_message_for_ui(from: &str, content: &str) -> Option<String> {
    if from == "user" {
        return Some(content.to_string());
    }

    let mut cleaned_lines: Vec<String> = Vec::new();
    let mut saw_tool = false;
    let mut last_tool: Option<ToolCallForUi> = None;
    let mut saw_read_result = false;
    let mut saw_tool_result_block = false;
    let mut drop_remainder_as_tool_result = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            if cleaned_lines.last().map(|v| !v.is_empty()).unwrap_or(false) {
                cleaned_lines.push(String::new());
            }
            continue;
        }
        if drop_remainder_as_tool_result {
            continue;
        }

        if let Some(tool_call) = parse_tool_call_from_json_line(line) {
            saw_tool = true;
            if !tool_call.name.is_empty() {
                last_tool = Some(tool_call);
            }
            continue;
        }

        let lower = line.to_lowercase();
        if lower.starts_with("tool ")
            || lower.starts_with("tool_error:")
            || lower.starts_with("tool_not_allowed:")
        {
            saw_tool = true;
            saw_tool_result_block = true;
            drop_remainder_as_tool_result = true;
            if let Some(name) = parse_tool_name_from_result_line(line) {
                last_tool = Some(ToolCallForUi {
                    name,
                    args: None,
                });
            }
            if lower.starts_with("tool read:") {
                saw_read_result = true;
                if last_tool.is_none() {
                    last_tool = Some(ToolCallForUi {
                        name: "Read".to_string(),
                        args: None,
                    });
                }
            }
            continue;
        }
        if lower.starts_with("starting autonomous loop for task:") {
            continue;
        }
        if lower == "(content omitted in chat; open the file viewer for full text)" {
            continue;
        }
        // Never show Read output content in chat UI. After a Read tool result,
        // many lines are TOML/JSON/etc and don't trip `looks_like_code_line`, which causes
        // full file dumps to appear in chat. Instead, collapse the entire tool result into
        // a single progress/status line ("Reading file...").
        if saw_read_result {
            continue;
        }
        cleaned_lines.push(raw_line.to_string());
    }

    let cleaned = cleaned_lines
        .join("\n")
        .replace("\n\n\n", "\n\n")
        .trim()
        .to_string();
    if saw_tool_result_block {
        // Tool call status lines are emitted from the call event itself; suppress
        // tool-result duplicates in chat UI.
        return None;
    }
    if cleaned.is_empty() {
        if saw_tool {
            return Some(status_line_for_tool_call(last_tool.as_ref()));
        }
        return None;
    }
    Some(cleaned)
}

pub(crate) fn sanitize_server_event_for_ui(event: ServerEvent) -> Option<ServerEvent> {
    match event {
        ServerEvent::Message { from, to, content } => {
            let cleaned = sanitize_message_for_ui(&from, &content)?;
            Some(ServerEvent::Message {
                from,
                to,
                content: cleaned,
            })
        }
        // Status + final Message are enough for chat UI; dropping token chunks prevents
        // leaking raw tool payload fragments while keeping raw context in DB.
        ServerEvent::Token { .. } => None,
        other => Some(other),
    }
}

pub(crate) async fn emit_queue_updated(
    state: &Arc<ServerState>,
    project_root: &str,
    session_id: &str,
    agent_id: &str,
) {
    let key = queue_key(project_root, session_id, agent_id);
    let items = {
        let guard = state.queued_chats.lock().await;
        guard.get(&key).cloned().unwrap_or_default()
    };
    let _ = state.events_tx.send(ServerEvent::QueueUpdated {
        project_root: project_root.to_string(),
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        items,
    });
}

pub(crate) fn emit_outcome_event(
    outcome: &AgentOutcome,
    events_tx: &broadcast::Sender<ServerEvent>,
    from_id: &str,
) {
    match outcome {
        AgentOutcome::Task(packet) => {
            let _ = events_tx.send(ServerEvent::Message {
                from: from_id.to_string(),
                to: "user".to_string(),
                content: serde_json::json!({
                    "type": "finalize_task",
                    "packet": packet
                })
                .to_string(),
            });
        }
        AgentOutcome::Patch(diff) => {
            let _ = events_tx.send(ServerEvent::Message {
                from: from_id.to_string(),
                to: "user".to_string(),
                content: serde_json::json!({
                    "type": "patch",
                    "diff": diff
                })
                .to_string(),
            });
        }
        _ => {}
    }
}
