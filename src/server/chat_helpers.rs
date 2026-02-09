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
        if matches!(tool, "write_file" | "Write") {
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
        AgentOutcome::Ask(question) => {
            let _ = events_tx.send(ServerEvent::Message {
                from: from_id.to_string(),
                to: "user".to_string(),
                content: serde_json::json!({
                    "type": "ask",
                    "question": question
                })
                .to_string(),
            });
        }
        _ => {}
    }
}
