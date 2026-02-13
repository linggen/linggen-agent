use crate::config::{AgentKind, AgentPolicyCapability};
use crate::engine::PromptMode;
use crate::server::chat_helpers::{
    emit_outcome_event, emit_queue_updated, extract_tool_path_arg, queue_key, queue_preview,
    sanitize_tool_args_for_display, tool_status_line, ToolStatusPhase,
};
use crate::server::{QueuedChatItem, ServerEvent, ServerState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio_stream::StreamExt;

#[derive(Deserialize)]
pub(crate) struct ChatRequest {
    project_root: String,
    agent_id: String,
    message: String,
    session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ClearChatRequest {
    project_root: String,
    session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SettingsQuery {
    project_root: String,
}

#[derive(Deserialize)]
pub(crate) struct UpdateSettingsRequest {
    project_root: String,
    mode: String,
}

#[derive(Serialize)]
struct SettingsResponse {
    mode: String,
}

fn prompt_mode_from_string(mode: &str) -> PromptMode {
    if mode.eq_ignore_ascii_case("chat") {
        PromptMode::Chat
    } else {
        PromptMode::Structured
    }
}

fn mode_label(mode: PromptMode) -> &'static str {
    if mode == PromptMode::Chat {
        "chat"
    } else {
        "auto"
    }
}

#[allow(dead_code)]
fn looks_like_file_or_path_request(message: &str) -> bool {
    let msg = message.trim();
    if msg.is_empty() {
        return false;
    }

    // Quick path heuristics
    if msg.contains('/') || msg.contains('\\') || msg.contains("@/") {
        return true;
    }

    // File extension heuristics (no regex/deps)
    const EXTS: &[&str] = &[
        "rs", "toml", "md", "txt", "json", "yaml", "yml", "ts", "tsx", "js", "jsx", "py", "sql",
        "go", "java", "kt", "c", "h", "cpp", "hpp",
    ];

    for raw in msg.split_whitespace() {
        let w = raw.trim_matches(|c: char| {
            !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
        });
        if w.contains('/') {
            return true;
        }
        if let Some(dot) = w.rfind('.') {
            let ext = &w[dot + 1..].to_ascii_lowercase();
            if EXTS.iter().any(|e| e == ext) {
                return true;
            }
        }
    }
    false
}

fn parse_explicit_target_prefix(message: &str) -> Option<(&str, &str)> {
    let rest = message.strip_prefix('@')?;
    let space_idx = rest.find(' ')?;
    let candidate = rest[..space_idx].trim();
    let body = rest[space_idx + 1..].trim_start();
    if candidate.is_empty() {
        return None;
    }
    if !candidate
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some((candidate, body))
}

const CHAT_DUP_TOOL_STREAK_LIMIT: usize = 3;
const CHAT_NO_NEW_READ_STEP_LIMIT: usize = 12;

async fn force_plaintext_summary(
    engine: &mut crate::engine::AgentEngine,
    events_tx: &tokio::sync::broadcast::Sender<ServerEvent>,
    agent_id: &str,
    session_id: Option<&str>,
    original_user_request: &str,
    read_paths: &[String],
    reason: &str,
) -> String {
    let read_files = if read_paths.is_empty() {
        "(none)".to_string()
    } else {
        read_paths.join(", ")
    };
    let prompt = format!(
        "Stop using tools now.\n\
Reason: {reason}\n\
Original user request: {original_user_request}\n\
Files already read: {read_files}\n\n\
Provide a concise final plain-text response to the user based on gathered information. \
Do not output JSON and do not request more tool calls."
    );

    let mut summary = String::new();
    match engine
        .chat_stream(
            &prompt,
            session_id,
            crate::engine::PromptMode::Chat,
        )
        .await
    {
        Ok(mut stream) => {
            while let Some(token_result) = stream.next().await {
                if let Ok(token) = token_result {
                    summary.push_str(&token);
                    let _ = events_tx.send(ServerEvent::Token {
                        agent_id: agent_id.to_string(),
                        token,
                    });
                }
            }
        }
        Err(err) => {
            tracing::warn!("Forced summary stream failed: {}", err);
        }
    }

    let trimmed = summary.trim().to_string();
    if trimmed.is_empty() || extract_chat_tool_call(&trimmed).is_some() {
        if read_paths.is_empty() {
            format!(
                "I stopped because the tool loop was repeating without progress ({reason}). \
Please narrow scope (for example, exact file names) and I can continue."
            )
        } else {
            format!(
                "I stopped because the tool loop was repeating without progress ({reason}). \
I already read: {}. Please tell me which file(s) to focus on next, or ask for a summary of specific files.",
                read_paths.join(", ")
            )
        }
    } else {
        trimmed
    }
}

fn extract_chat_tool_call(text: &str) -> Option<(String, serde_json::Value)> {
    if let Ok(action) = crate::engine::parse_first_action(text) {
        if let crate::engine::ModelAction::Tool { tool, args } = action {
            return Some((tool, args));
        }
    }

    None
}

fn chat_mode_structured_output_error(
    text: &str,
    allow_patch: bool,
    allow_finalize: bool,
) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(action) = crate::engine::parse_first_action(trimmed) {
        return match action {
            crate::engine::ModelAction::Tool { .. } => None,
            crate::engine::ModelAction::Patch { .. } => {
                if !allow_patch {
                    Some(
                        "I couldn't continue because this agent's policy does not allow `patch`. Update frontmatter `policy` to include `Patch` if needed."
                            .to_string(),
                    )
                } else {
                    Some(
                        "I couldn't continue because chat mode expects plain text or a single tool call. Please try again."
                            .to_string(),
                    )
                }
            }
            crate::engine::ModelAction::FinalizeTask { .. } => {
                if !allow_finalize {
                    Some(
                        "I couldn't continue because this agent's policy does not allow `finalize_task`. Update frontmatter `policy` to include `Finalize` if needed."
                            .to_string(),
                    )
                } else {
                    Some(
                        "I couldn't continue because chat mode expects plain text or a single tool call. Please try again."
                            .to_string(),
                    )
                }
            }
        };
    }

    if trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if value.get("type").and_then(|v| v.as_str()).is_some() {
                return Some(
                    "I couldn't continue because chat mode expects plain text or a single tool call."
                        .to_string(),
                );
            }
        }
    }

    None
}

async fn run_loop_with_tracking(
    manager: &Arc<crate::agent_manager::AgentManager>,
    root: &PathBuf,
    engine: &mut crate::engine::AgentEngine,
    agent_id: &str,
    session_id: Option<&str>,
    detail: &str,
) -> Result<crate::engine::AgentOutcome, anyhow::Error> {
    let run_id = manager
        .begin_agent_run(root, session_id, agent_id, None, Some(detail.to_string()))
        .await
        .ok();

    engine.set_run_id(run_id.clone());
    let result = engine.run_agent_loop(session_id).await;
    engine.set_run_id(None);

    if let Some(run_id) = run_id {
        match &result {
            Ok(_) => {
                let _ = manager.finish_agent_run(&run_id, "completed", None).await;
            }
            Err(err) => {
                let msg = err.to_string();
                let status = if msg.to_lowercase().contains("cancel") {
                    "cancelled"
                } else {
                    "failed"
                };
                let _ = manager.finish_agent_run(&run_id, status, Some(msg)).await;
            }
        }
    }

    result
}

pub(crate) async fn get_settings_api(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<SettingsQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&q.project_root)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&q.project_root));
    match state
        .manager
        .db
        .get_project_settings(&root.to_string_lossy())
    {
        Ok(settings) => Json(SettingsResponse {
            mode: settings.mode,
        })
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub(crate) async fn update_settings_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<UpdateSettingsRequest>,
) -> impl IntoResponse {
    let mode = if req.mode.eq_ignore_ascii_case("chat") {
        "chat".to_string()
    } else {
        "auto".to_string()
    };
    let root = PathBuf::from(&req.project_root)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&req.project_root));
    let root_str = root.to_string_lossy().to_string();
    let _ = state.manager.get_or_create_project(root.clone()).await;
    if let Err(e) = state.manager.db.set_project_mode(&root_str, &mode) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let _ = state
        .manager
        .set_project_prompt_mode(&root, prompt_mode_from_string(&mode))
        .await;
    let _ = state.events_tx.send(ServerEvent::SettingsUpdated {
        project_root: root_str,
        mode: mode.clone(),
    });
    Json(SettingsResponse { mode }).into_response()
}

pub(crate) async fn clear_chat_history_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ClearChatRequest>,
) -> impl IntoResponse {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    match state
        .manager
        .db
        .clear_chat_history(&req.project_root, &session_id)
    {
        Ok(removed) => {
            // Also clear in-memory chat history for all agents in this project/session
            if let Ok(root) = PathBuf::from(&req.project_root).canonicalize() {
                if let Ok(ctx) = state.manager.get_or_create_project(root).await {
                    let agents = ctx.agents.lock().await;
                    for agent_mutex in agents.values() {
                        let mut agent = agent_mutex.lock().await;
                        agent.chat_history.clear();
                        agent.observations.clear();
                    }
                }
            }

            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(serde_json::json!({ "removed": removed })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub(crate) async fn chat_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let project_root_str = root.to_string_lossy().to_string();
    let session_id = req.session_id.clone();
    let effective_session_id = session_id.clone().unwrap_or_else(|| "default".to_string());
    let events_tx = state.events_tx.clone();

    // Optional explicit target prefix: "@agent_id <message>".
    // Only reroute when the candidate agent exists in this project.
    let (target_id, clean_msg) =
        if let Some((candidate, body)) = parse_explicit_target_prefix(&req.message) {
            let candidate_id = candidate.to_string();
            if state
                .manager
                .resolve_agent_kind(&root, &candidate_id)
                .await
                .is_some()
            {
                (candidate_id, body.to_string())
            } else {
                (req.agent_id.clone(), req.message.clone())
            }
        } else {
            (req.agent_id.clone(), req.message.clone())
        };
    let trimmed_msg = clean_msg.trim();

    let kind = state
        .manager
        .resolve_agent_kind(&root, &target_id)
        .await
        .unwrap_or(AgentKind::Main);
    if kind == AgentKind::Subagent {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "subagents cannot be chatted with directly; send requests to a main agent"
            })),
        )
            .into_response();
    }

    match state.manager.get_or_create_agent(&root, &target_id).await {
        Ok(agent) => {
            let was_busy = agent.try_lock().is_err();
            let queued_item = if was_busy {
                Some(QueuedChatItem {
                    id: format!(
                        "{}-{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis(),
                        state.queue_seq.fetch_add(1, Ordering::Relaxed)
                    ),
                    agent_id: target_id.clone(),
                    session_id: effective_session_id.clone(),
                    preview: queue_preview(&clean_msg),
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                })
            } else {
                None
            };
            if let Some(item) = &queued_item {
                let key = queue_key(&project_root_str, &effective_session_id, &target_id);
                {
                    let mut guard = state.queued_chats.lock().await;
                    guard.entry(key).or_default().push(item.clone());
                }
                emit_queue_updated(&state, &project_root_str, &effective_session_id, &target_id)
                    .await;
            }

            let events_tx_clone = events_tx.clone();
            let target_id_clone = target_id.clone();
            let clean_msg_clone = clean_msg.clone();
            let root_clone = root.clone();
            let manager = state.manager.clone();
            let state_clone = state.clone();
            let queued_item_id = queued_item.as_ref().map(|q| q.id.clone());
            let session_id_for_queue = effective_session_id.clone();
            let project_root_for_queue = project_root_str.clone();

            // Handle mode switch commands before emitting a user message.
            if let Some(mode_value) = trimmed_msg.strip_prefix("/mode ") {
                // Emit and persist the user's /mode command so it appears in chat history.
                let _ = events_tx.send(ServerEvent::Message {
                    from: "user".to_string(),
                    to: target_id.clone(),
                    content: clean_msg.clone(),
                });
                if let Ok(ctx) = state.manager.get_or_create_project(root.clone()).await {
                    let _ = ctx.state_fs.append_message(
                        "user",
                        &target_id,
                        &clean_msg,
                        None,
                        session_id.as_deref(),
                    );
                    let _ = state
                        .manager
                        .db
                        .add_chat_message(crate::db::ChatMessageRecord {
                            repo_path: root.to_string_lossy().to_string(),
                            session_id: session_id.clone().unwrap_or_else(|| "default".to_string()),
                            agent_id: target_id.clone(),
                            from_id: "user".to_string(),
                            to_id: target_id.clone(),
                            content: clean_msg.clone(),
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            is_observation: false,
                        });
                }

                let mode_value = mode_value.trim().to_lowercase();
                let mut engine = agent.lock().await;
                let mode = prompt_mode_from_string(&mode_value);
                engine.set_prompt_mode(mode);
                let mode_label = mode_label(mode);
                let _ = state
                    .manager
                    .db
                    .set_project_mode(&root.to_string_lossy(), mode_label);
                let _ = state.manager.set_project_prompt_mode(&root, mode).await;
                let _ = events_tx_clone.send(ServerEvent::SettingsUpdated {
                    project_root: root.to_string_lossy().to_string(),
                    mode: mode_label.to_string(),
                });
                let _ = events_tx_clone.send(ServerEvent::Message {
                    from: target_id_clone.clone(),
                    to: "user".to_string(),
                    content: format!("Mode set to {}", mode_label),
                });
                return Json(serde_json::json!({ "status": "mode_set" })).into_response();
            }

            if !was_busy {
                // Emit user message event immediately if the target agent is not busy.
                let _ = events_tx.send(ServerEvent::Message {
                    from: "user".to_string(),
                    to: target_id.clone(),
                    content: clean_msg.clone(),
                });

                // Persist user message in DB immediately so fetchWorkspaceState sees it.
                if let Ok(ctx) = state.manager.get_or_create_project(root.clone()).await {
                    let _ = ctx.state_fs.append_message(
                        "user",
                        &target_id,
                        &clean_msg,
                        None,
                        session_id.as_deref(),
                    );

                    let _ = state
                        .manager
                        .db
                        .add_chat_message(crate::db::ChatMessageRecord {
                            repo_path: root.to_string_lossy().to_string(),
                            session_id: session_id.clone().unwrap_or_else(|| "default".to_string()),
                            agent_id: target_id.clone(),
                            from_id: "user".to_string(),
                            to_id: target_id.clone(),
                            content: clean_msg.clone(),
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            is_observation: false,
                        });
                }
            }

            tokio::spawn(async move {
                let mut engine = agent.lock().await;
                if let Some(queued_id) = queued_item_id.as_deref() {
                    // This message just left the queue and is now active.
                    let key = queue_key(
                        &project_root_for_queue,
                        &session_id_for_queue,
                        &target_id_clone,
                    );
                    {
                        let mut guard = state_clone.queued_chats.lock().await;
                        if let Some(items) = guard.get_mut(&key) {
                            items.retain(|item| item.id != queued_id);
                            if items.is_empty() {
                                guard.remove(&key);
                            }
                        }
                    }
                    emit_queue_updated(
                        &state_clone,
                        &project_root_for_queue,
                        &session_id_for_queue,
                        &target_id_clone,
                    )
                    .await;

                    // Emit and persist the queued user message now, when processing starts.
                    let _ = events_tx_clone.send(ServerEvent::Message {
                        from: "user".to_string(),
                        to: target_id_clone.clone(),
                        content: clean_msg_clone.clone(),
                    });
                    if let Ok(ctx) = manager.get_or_create_project(root_clone.clone()).await {
                        let _ = ctx.state_fs.append_message(
                            "user",
                            &target_id_clone,
                            &clean_msg_clone,
                            None,
                            session_id.as_deref(),
                        );
                        let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                            repo_path: root_clone.to_string_lossy().to_string(),
                            session_id: session_id.clone().unwrap_or_else(|| "default".to_string()),
                            agent_id: target_id_clone.clone(),
                            from_id: "user".to_string(),
                            to_id: target_id_clone.clone(),
                            content: clean_msg_clone.clone(),
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            is_observation: false,
                        });
                    }
                }

                state_clone
                    .send_agent_status(
                        target_id_clone.clone(),
                        "model_loading".to_string(),
                        Some("Model loading".to_string()),
                    )
                    .await;
                if let Ok(settings) = manager.get_project_settings(&root_clone).await {
                    engine.set_prompt_mode(prompt_mode_from_string(&settings.mode));
                }
                let mut full_response = String::new();

                // If the user is invoking a skill (slash command), skip streaming chat.
                // Go straight into the structured agent loop to avoid dumping tool JSON into the UI.
                if clean_msg_clone.trim_start().starts_with('/') {
                    // Activate skill and set the loop task from the command payload.
                    let parts: Vec<&str> = clean_msg_clone.trim().splitn(2, ' ').collect();
                    let cmd = parts[0].trim_start_matches('/');
                    let task_for_loop = parts
                        .get(1)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| {
                            "Initialize this workspace and summarize status.".to_string()
                        });

                    if let Some(manager) = engine.tools.get_manager() {
                        if let Some(skill) = manager.skill_manager.get_skill(cmd).await {
                            engine.active_skill = Some(skill);
                        }
                    }

                    // New skill run: clear stale observations.
                    engine.observations.clear();
                    engine.task = Some(task_for_loop);

                    let _ = events_tx_clone.send(ServerEvent::Message {
                        from: target_id_clone.clone(),
                        to: "user".to_string(),
                        content: format!("Running skill: {}", cmd),
                    });
                    if let Ok(ctx) = manager.get_or_create_project(root_clone.clone()).await {
                        let _ = ctx.state_fs.append_message(
                            &target_id_clone,
                            "user",
                            &format!("Running skill: {}", cmd),
                            None,
                            session_id.as_deref(),
                        );
                        let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                            repo_path: root_clone.to_string_lossy().to_string(),
                            session_id: session_id.clone().unwrap_or_else(|| "default".to_string()),
                            agent_id: target_id_clone.clone(),
                            from_id: target_id_clone.clone(),
                            to_id: "user".to_string(),
                            content: format!("Running skill: {}", cmd),
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            is_observation: false,
                        });
                    }

                    let outcome = run_loop_with_tracking(
                        &manager,
                        &root_clone,
                        &mut engine,
                        &target_id_clone,
                        session_id.as_deref(),
                        "chat:skill",
                    )
                    .await;
                    if let Err(e) = outcome {
                        tracing::warn!("Skill loop failed: {}", e);
                        let _ = events_tx_clone.send(ServerEvent::Message {
                            from: target_id_clone.clone(),
                            to: "user".to_string(),
                            content: format!("Error: {}", e),
                        });
                        if let Ok(ctx) = manager.get_or_create_project(root_clone.clone()).await {
                            let err_msg = format!("Error: {}", e);
                            let _ = ctx.state_fs.append_message(
                                &target_id_clone,
                                "user",
                                &err_msg,
                                None,
                                session_id.as_deref(),
                            );
                            let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                                repo_path: root_clone.to_string_lossy().to_string(),
                                session_id: session_id
                                    .clone()
                                    .unwrap_or_else(|| "default".to_string()),
                                agent_id: target_id_clone.clone(),
                                from_id: target_id_clone.clone(),
                                to_id: "user".to_string(),
                                content: err_msg,
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs(),
                                is_observation: false,
                            });
                        }
                    } else {
                        if let Ok(outcome) = &outcome {
                            emit_outcome_event(outcome, &events_tx_clone, &target_id_clone);
                        }
                        // Force UI refresh after loop
                        let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                    }

                    state_clone
                        .send_agent_status(
                            target_id_clone.clone(),
                            "idle".to_string(),
                            Some("Idle".to_string()),
                        )
                        .await;
                    return;
                }

                let prompt_mode = engine.get_prompt_mode();

                // In structured mode, force JSON/tool usage by running the structured agent loop
                // (chat streaming is best-effort and many models will output plain text commands).
                if prompt_mode == crate::engine::PromptMode::Structured {
                    state_clone
                        .send_agent_status(
                            target_id_clone.clone(),
                            "thinking".to_string(),
                            Some("Thinking".to_string()),
                        )
                        .await;
                    // New structured task: clear stale observations so the loop starts clean.
                    engine.observations.clear();
                    let task_for_loop = clean_msg_clone.trim().to_string();
                    engine.task = Some(task_for_loop);
                    let outcome = run_loop_with_tracking(
                        &manager,
                        &root_clone,
                        &mut engine,
                        &target_id_clone,
                        session_id.as_deref(),
                        "chat:structured-loop",
                    )
                    .await;
                    if let Ok(outcome) = &outcome {
                        emit_outcome_event(outcome, &events_tx_clone, &target_id_clone);
                    } else if let Err(err) = outcome {
                        let _ = events_tx_clone.send(ServerEvent::Message {
                            from: target_id_clone.clone(),
                            to: "user".to_string(),
                            content: format!("Error: {}", err),
                        });
                    }
                    let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                    state_clone
                        .send_agent_status(
                            target_id_clone.clone(),
                            "idle".to_string(),
                            Some("Idle".to_string()),
                        )
                        .await;
                    return;
                }

                // In chat mode, run a bounded agentic loop:
                // model -> tool -> observation -> model (repeat) until plain-text answer.

                match engine
                    .chat_stream(&clean_msg_clone, session_id.as_deref(), prompt_mode)
                    .await
                {
                    Ok(mut stream) => {
                        state_clone
                            .send_agent_status(
                                target_id_clone.clone(),
                                "thinking".to_string(),
                                Some("Thinking".to_string()),
                            )
                            .await;
                        while let Some(token_result) = stream.next().await {
                            if let Ok(token) = token_result {
                                full_response.push_str(&token);
                                let _ = events_tx_clone.send(ServerEvent::Token {
                                    agent_id: target_id_clone.clone(),
                                    token,
                                });
                            }
                        }

                        // Debug log: split streamed model output into text + json (truncated).
                        let (text_part, json_part) =
                            crate::engine::model_message_log_parts(&full_response, 100, 100);
                        let json_rendered = json_part
                            .as_ref()
                            .and_then(|v| serde_json::to_string(v).ok())
                            .unwrap_or_else(|| "null".to_string());
                        tracing::info!(
                            "Chat model output split: text='{}' json={}",
                            text_part.replace('\n', "\\n"),
                            json_rendered
                        );

                        // Chat-mode tool loop budget is fully config-driven.
                        let chat_tool_max_iters = engine.cfg.max_iters;
                        let allow_patch = engine
                            .spec
                            .as_ref()
                            .map(|s| s.allows_policy(AgentPolicyCapability::Patch))
                            .unwrap_or(false);
                        let allow_finalize = engine
                            .spec
                            .as_ref()
                            .map(|s| s.allows_policy(AgentPolicyCapability::Finalize))
                            .unwrap_or(false);
                        let mut pending_tool = extract_chat_tool_call(&full_response);
                        let mut final_response = if pending_tool.is_some() {
                            None
                        } else if let Some(err_msg) = chat_mode_structured_output_error(
                            &full_response,
                            allow_patch,
                            allow_finalize,
                        ) {
                            Some(err_msg)
                        } else {
                            Some(full_response.clone())
                        };
                        let mut tool_steps = 0usize;
                        let mut last_tool_sig = String::new();
                        let mut duplicate_tool_streak = 0usize;
                        let mut read_paths_seen: HashSet<String> = HashSet::new();
                        let mut read_paths_order: Vec<String> = Vec::new();
                        let mut steps_since_new_read = 0usize;

                        while final_response.is_none() {
                            if tool_steps >= chat_tool_max_iters {
                                final_response = Some(format!(
                                    "I stopped after {} tool steps to respect max_iters.",
                                    chat_tool_max_iters
                                ));
                                break;
                            }
                            let Some((tool, args)) = pending_tool.take() else {
                                break;
                            };
                            tool_steps += 1;
                            steps_since_new_read = steps_since_new_read.saturating_add(1);

                            let call_sig = crate::engine::tool_call_signature(&tool, &args);
                            if call_sig == last_tool_sig {
                                duplicate_tool_streak = duplicate_tool_streak.saturating_add(1);
                            } else {
                                duplicate_tool_streak = 0;
                                last_tool_sig = call_sig;
                            }
                            if duplicate_tool_streak >= CHAT_DUP_TOOL_STREAK_LIMIT {
                                tracing::warn!(
                                    "Chat tool loop breaker: duplicate call streak={} tool={}",
                                    duplicate_tool_streak + 1,
                                    tool
                                );
                                let reply = force_plaintext_summary(
                                    &mut engine,
                                    &events_tx_clone,
                                    &target_id_clone,
                                    session_id.as_deref(),
                                    &clean_msg_clone,
                                    &read_paths_order,
                                    "same tool call repeated",
                                )
                                .await;
                                final_response = Some(reply);
                                break;
                            }

                            let tool_start_status =
                                tool_status_line(&tool, Some(&args), ToolStatusPhase::Start);
                            let tool_done_status =
                                tool_status_line(&tool, Some(&args), ToolStatusPhase::Done);
                            let tool_failed_status =
                                tool_status_line(&tool, Some(&args), ToolStatusPhase::Failed);

                            state_clone
                                .send_agent_status(
                                    target_id_clone.clone(),
                                    "calling_tool".to_string(),
                                    Some(tool_start_status),
                                )
                                .await;
                            let safe_args = sanitize_tool_args_for_display(&tool, &args);
                            let _ = events_tx_clone.send(ServerEvent::Message {
                                from: target_id_clone.clone(),
                                to: "user".to_string(),
                                content: serde_json::json!({
                                    "type": "tool",
                                    "tool": tool.clone(),
                                    "args": safe_args.clone()
                                })
                                .to_string(),
                            });
                            if let Ok(ctx) = manager.get_or_create_project(root_clone.clone()).await
                            {
                                let tool_msg = serde_json::json!({
                                    "type": "tool",
                                    "tool": tool.clone(),
                                    "args": safe_args
                                })
                                .to_string();
                                let _ = ctx.state_fs.append_message(
                                    &target_id_clone,
                                    "user",
                                    &tool_msg,
                                    None,
                                    session_id.as_deref(),
                                );
                                let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                                    repo_path: root_clone.to_string_lossy().to_string(),
                                    session_id: session_id
                                        .clone()
                                        .unwrap_or_else(|| "default".to_string()),
                                    agent_id: target_id_clone.clone(),
                                    from_id: target_id_clone.clone(),
                                    to_id: "user".to_string(),
                                    content: tool_msg,
                                    timestamp: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap()
                                        .as_secs(),
                                    is_observation: false,
                                });
                            }

                            let mutate_path = if matches!(tool.as_str(), "Write" | "Edit") {
                                extract_tool_path_arg(&args)
                            } else {
                                None
                            };
                            let read_path = if matches!(tool.as_str(), "Read") {
                                extract_tool_path_arg(&args)
                            } else {
                                None
                            };
                            let call = crate::engine::tools::ToolCall {
                                tool: tool.clone(),
                                args,
                            };

                            let result = match engine.tools.execute(call) {
                                Ok(result) => result,
                                Err(e) => {
                                    tracing::warn!("Tool execution failed ({}): {}", tool, e);
                                    state_clone
                                        .send_agent_status(
                                            target_id_clone.clone(),
                                            "calling_tool".to_string(),
                                            Some(tool_failed_status.clone()),
                                        )
                                        .await;
                                    let rendered = format!("tool_error: tool={} error={}", tool, e);
                                    engine.upsert_observation("error", &tool, rendered.clone());
                                    let _ = engine
                                        .manager_db_add_observation(
                                            &tool,
                                            &rendered,
                                            session_id.as_deref(),
                                        )
                                        .await;
                                    let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                                    final_response =
                                        Some(format!("Tool execution failed ({}): {}", tool, e));
                                    break;
                                }
                            };

                            let rendered_model = crate::engine::render_tool_result(&result);
                            let rendered_public = crate::engine::render_tool_result_public(&result);
                            engine.upsert_observation("tool", &tool, rendered_model.clone());
                            let _ = engine
                                .manager_db_add_observation(
                                    &tool,
                                    &rendered_public,
                                    session_id.as_deref(),
                                )
                                .await;
                            let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                            state_clone
                                .send_agent_status(
                                    target_id_clone.clone(),
                                    "calling_tool".to_string(),
                                    Some(tool_done_status),
                                )
                                .await;
                            state_clone
                                .send_agent_status(
                                    target_id_clone.clone(),
                                    "thinking".to_string(),
                                    Some("Thinking".to_string()),
                                )
                                .await;

                            // Deterministic chaining for common file workflows.
                            let mut observation_for_prompt = rendered_model.clone();
                            let mut observation_for_display = rendered_public.clone();

                            if matches!(tool.as_str(), "Read") {
                                if let Some(path) = read_path {
                                    let norm = path.trim().replace('\\', "/");
                                    if !norm.is_empty() {
                                        if read_paths_seen.insert(norm.clone()) {
                                            read_paths_order.push(norm);
                                            steps_since_new_read = 0;
                                        }
                                    }
                                }
                                if !read_paths_order.is_empty()
                                    && steps_since_new_read >= CHAT_NO_NEW_READ_STEP_LIMIT
                                {
                                    tracing::warn!(
                                        "Chat tool loop breaker: no new read files for {} steps",
                                        steps_since_new_read
                                    );
                                    let reply = force_plaintext_summary(
                                        &mut engine,
                                        &events_tx_clone,
                                        &target_id_clone,
                                        session_id.as_deref(),
                                        &clean_msg_clone,
                                        &read_paths_order,
                                        "no new files were read",
                                    )
                                    .await;
                                    final_response = Some(reply);
                                    break;
                                }
                            }

                            if matches!(tool.as_str(), "Write" | "Edit") {
                                if let Some(path) = mutate_path {
                                    let readback = engine.tools.execute(crate::engine::tools::ToolCall {
                                        tool: "Read".to_string(),
                                        args: serde_json::json!({ "path": path, "max_bytes": 8000 }),
                                    });
                                    if let Ok(read_result) = readback {
                                        let read_model =
                                            crate::engine::render_tool_result(&read_result);
                                        let read_public =
                                            crate::engine::render_tool_result_public(&read_result);
                                        engine.upsert_observation(
                                            "tool",
                                            "Read",
                                            read_model.clone(),
                                        );
                                        let _ = engine
                                            .manager_db_add_observation(
                                                "Read",
                                                &read_public,
                                                session_id.as_deref(),
                                            )
                                            .await;
                                        observation_for_prompt = format!(
                                            "{}\n\nPost-write readback:\n{}",
                                            observation_for_prompt, read_model
                                        );
                                        observation_for_display = format!(
                                            "{}\n\nPost-write readback:\n{}",
                                            observation_for_display, read_public
                                        );
                                        let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                                    }
                                }
                            }

                            let followup_prompt = format!(
                                "Continue helping with the same user request.\n\nOriginal user request:\n{}\n\nLatest observation:\n{}\n\nIf another tool is needed, respond with exactly one JSON tool call.\nOtherwise, answer the user directly in plain text.",
                                clean_msg_clone, observation_for_prompt
                            );
                            let mut followup_response = String::new();
                            match engine
                                .chat_stream(
                                    &followup_prompt,
                                    session_id.as_deref(),
                                    crate::engine::PromptMode::Chat,
                                )
                                .await
                            {
                                Ok(mut followup_stream) => {
                                    while let Some(token_result) = followup_stream.next().await {
                                        match token_result {
                                            Ok(token) => {
                                                followup_response.push_str(&token);
                                                let _ = events_tx_clone.send(ServerEvent::Token {
                                                    agent_id: target_id_clone.clone(),
                                                    token,
                                                });
                                            }
                                            Err(err) => {
                                                tracing::warn!(
                                                    "Follow-up stream token error (step {}): {}",
                                                    tool_steps,
                                                    err
                                                );
                                                final_response = Some(format!(
                                                    "I hit a model stream error while continuing the task: {}",
                                                    err
                                                ));
                                                break;
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        "Follow-up model stream failed after tool '{}' (step {}): {}",
                                        tool,
                                        tool_steps,
                                        err
                                    );
                                    final_response = Some(format!(
                                        "I couldn't continue after tool '{}' due to model error: {}",
                                        tool, err
                                    ));
                                }
                            }
                            if final_response.is_some() {
                                break;
                            }

                            let (followup_text_part, followup_json_part) =
                                crate::engine::model_message_log_parts(
                                    &followup_response,
                                    100,
                                    100,
                                );
                            let followup_json_rendered = followup_json_part
                                .as_ref()
                                .and_then(|v| serde_json::to_string(v).ok())
                                .unwrap_or_else(|| "null".to_string());
                            tracing::info!(
                                "Chat follow-up output split: text='{}' json={}",
                                followup_text_part.replace('\n', "\\n"),
                                followup_json_rendered
                            );

                            pending_tool = extract_chat_tool_call(&followup_response);
                            if pending_tool.is_none() {
                                if let Some(err_msg) = chat_mode_structured_output_error(
                                    &followup_response,
                                    allow_patch,
                                    allow_finalize,
                                ) {
                                    final_response = Some(err_msg);
                                } else {
                                    final_response = Some(followup_response);
                                }
                                break;
                            }
                        }

                        let mut reply = final_response.unwrap_or_else(|| {
                            "I couldn't produce a final answer from the tool loop.".to_string()
                        });
                        if reply.trim().is_empty() {
                            reply = "I couldn't produce a non-empty response. Please try again."
                                .to_string();
                        }

                        let _ = engine
                            .finalize_chat(
                                &clean_msg_clone,
                                &reply,
                                session_id.as_deref(),
                                prompt_mode,
                            )
                            .await;
                        let _ = events_tx_clone.send(ServerEvent::Message {
                            from: target_id_clone.clone(),
                            to: "user".to_string(),
                            content: reply,
                        });
                        let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                    }
                    Err(e) => {
                        let error_msg = format!("Error: {}", e);
                        let _ = events_tx_clone.send(ServerEvent::Message {
                            from: target_id_clone.clone(),
                            to: "user".to_string(),
                            content: error_msg.clone(),
                        });
                        if let Ok(ctx) = manager.get_or_create_project(root_clone.clone()).await {
                            let _ = ctx.state_fs.append_message(
                                &target_id_clone,
                                "user",
                                &error_msg,
                                None,
                                session_id.as_deref(),
                            );
                            let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                                repo_path: root_clone.to_string_lossy().to_string(),
                                session_id: session_id
                                    .clone()
                                    .unwrap_or_else(|| "default".to_string()),
                                agent_id: target_id_clone.clone(),
                                from_id: target_id_clone.clone(),
                                to_id: "user".to_string(),
                                content: error_msg,
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs(),
                                is_observation: false,
                            });
                        }
                    }
                }
                state_clone
                    .send_agent_status(
                        target_id_clone.clone(),
                        "idle".to_string(),
                        Some("Idle".to_string()),
                    )
                    .await;
            });

            let status = if was_busy { "queued" } else { "started" };
            Json(serde_json::json!({ "status": status })).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        chat_mode_structured_output_error, extract_chat_tool_call, parse_explicit_target_prefix,
    };

    #[test]
    fn extract_chat_tool_call_parses_supported_tool_json() {
        let input =
            r#"{"type":"tool","tool":"Grep","args":{"query":"logging.rs","globs":["src/**"]}}"#;
        let parsed = extract_chat_tool_call(input);
        assert!(parsed.is_some());
        let (tool, args) = parsed.unwrap();
        assert_eq!(tool, "Grep");
        assert_eq!(args["query"], "logging.rs");
        assert_eq!(args["globs"][0], "src/**");
    }

    #[test]
    fn extract_chat_tool_call_parses_edit_tool_json() {
        let input = r#"{"type":"tool","tool":"Edit","args":{"path":"src/logging.rs","old_string":"a","new_string":"b","replace_all":false}}"#;
        let parsed = extract_chat_tool_call(input);
        assert!(parsed.is_some());
        let (tool, args) = parsed.unwrap();
        assert_eq!(tool, "Edit");
        assert_eq!(args["path"], "src/logging.rs");
        assert_eq!(args["old_string"], "a");
        assert_eq!(args["new_string"], "b");
        assert_eq!(args["replace_all"], false);
    }

    #[test]
    fn chat_mode_structured_output_error_blocks_finalize_task() {
        let input = r#"{"type":"finalize_task","packet":{"title":"x","user_stories":[],"acceptance_criteria":[],"mermaid_wireframe":null}}"#;
        let err = chat_mode_structured_output_error(input, false, false);
        assert!(err.is_some());
    }

    #[test]
    fn chat_mode_structured_output_error_blocks_unknown_structured_json() {
        let input = r#"{"type":"unsupported_action","foo":"bar"}"#;
        let err = chat_mode_structured_output_error(input, false, false);
        assert!(err.is_some());
    }

    #[test]
    fn chat_mode_structured_output_error_allows_plain_text() {
        let err = chat_mode_structured_output_error(
            "I reviewed logging.rs and found two issues.",
            false,
            false,
        );
        assert!(err.is_none());
    }

    #[test]
    fn chat_mode_structured_output_error_finalize_allowed_still_requires_chat_shape() {
        let input = r#"{"type":"finalize_task","packet":{"title":"x","user_stories":[],"acceptance_criteria":[],"mermaid_wireframe":null}}"#;
        let err = chat_mode_structured_output_error(input, false, true);
        assert!(err.is_some());
        assert!(err
            .unwrap()
            .contains("chat mode expects plain text or a single tool call"));
    }

    #[test]
    fn parse_explicit_target_prefix_accepts_valid_mention() {
        let parsed = parse_explicit_target_prefix("@coder please review src/main.rs");
        assert_eq!(parsed, Some(("coder", "please review src/main.rs")));
    }

    #[test]
    fn parse_explicit_target_prefix_rejects_missing_body() {
        let parsed = parse_explicit_target_prefix("@coder");
        assert_eq!(parsed, None);
    }

    #[test]
    fn parse_explicit_target_prefix_rejects_invalid_agent_token() {
        let parsed = parse_explicit_target_prefix("@coder! please review");
        assert_eq!(parsed, None);
    }

    #[test]
    fn extract_chat_tool_call_does_not_infer_read_from_plain_text_with_filename() {
        let input = "All fixes have been applied to src/logging.rs and cargo check passed.";
        assert_eq!(extract_chat_tool_call(input), None);
    }

    #[test]
    fn extract_chat_tool_call_does_not_infer_bash_from_plain_text_command() {
        let input = "I'll run cargo check and report back.";
        assert_eq!(extract_chat_tool_call(input), None);
    }
}
