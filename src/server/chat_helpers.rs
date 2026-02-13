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

fn preview_value(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let preview: String = value.chars().take(max_chars).collect();
        format!("{preview}... ({len} chars)", len = value.chars().count())
    }
}

fn status_line_for_tool_call(tool_call: Option<&ToolCallForUi>) -> String {
    let Some(tool_call) = tool_call else {
        return "Calling tool...".to_string();
    };

    let name = tool_call.name.trim().to_lowercase();
    let args = tool_call.args.as_ref();
    match name.as_str() {
        "read" => "Reading file...".to_string(),
        "write" => "Writing file...".to_string(),
        "bash" => {
            let cmd = args
                .and_then(|v| v.get("cmd"))
                .and_then(|v| v.as_str())
                .map(|v| v.trim())
                .filter(|v| !v.is_empty());
            if let Some(cmd) = cmd {
                format!("Running command: {}", preview_value(cmd, 120))
            } else {
                "Running command...".to_string()
            }
        }
        "grep" => "Searching...".to_string(),
        "glob" => "Listing files...".to_string(),
        "delegate_to_agent" => {
            let target = args
                .and_then(|v| v.get("target_agent_id"))
                .and_then(|v| v.as_str())
                .map(|v| v.trim())
                .filter(|v| !v.is_empty());
            if let Some(target) = target {
                format!("Delegating to subagent: {target}")
            } else {
                "Delegating...".to_string()
            }
        }
        "" => "Calling tool...".to_string(),
        _ => format!("Calling tool: {}", tool_call.name.trim()),
    }
}

fn looks_like_code_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//")
        || t.starts_with("use ")
        || t.starts_with("import ")
        || t.starts_with("fn ")
        || t.starts_with("pub ")
        || t.starts_with("let ")
        || t.starts_with("const ")
        || t.starts_with("struct ")
        || t.starts_with("impl ")
        || t.starts_with("#include")
        || t.starts_with('{')
        || t.starts_with('}')
        || t.contains("::")
}

pub(crate) fn sanitize_message_for_ui(from: &str, content: &str) -> Option<String> {
    if from == "user" {
        return Some(content.to_string());
    }

    let mut cleaned_lines: Vec<String> = Vec::new();
    let mut saw_tool = false;
    let mut last_tool: Option<ToolCallForUi> = None;
    let mut saw_read_result = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            if cleaned_lines.last().map(|v| !v.is_empty()).unwrap_or(false) {
                cleaned_lines.push(String::new());
            }
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
            if lower.starts_with("tool read:") {
                saw_read_result = true;
                last_tool = Some(ToolCallForUi {
                    name: "Read".to_string(),
                    args: None,
                });
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
