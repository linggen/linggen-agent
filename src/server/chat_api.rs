use crate::engine::PromptMode;
use crate::config::AgentKind;
use crate::server::chat_helpers::{
    emit_outcome_event, emit_queue_updated, extract_tool_path_arg, queue_key, queue_preview,
    sanitize_tool_args_for_display,
};
use crate::server::{QueuedChatItem, ServerEvent, ServerState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
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
        let w = raw
            .trim_matches(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/')));
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

fn extract_shell_command_candidate(text: &str) -> Option<String> {
    // Prefer commands inside fenced code blocks, otherwise fall back to any
    // line that looks like a standalone shell command.
    let mut in_code = false;
    let mut code_lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code && !line.is_empty() {
            code_lines.push(line.to_string());
        }
    }

    let mut candidates: Vec<String> = Vec::new();
    candidates.extend(code_lines);
    candidates.extend(
        text.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string()),
    );

    // Keep this list aligned with `validate_shell_command` allowlist in `src/engine/tools.rs`.
    // (We only extract; the actual tool execution still validates.)
    const TOKENS: &[&str] = &[
        "find", "fd", "rg", "grep", "git", "ls", "pwd", "head", "tail", "cat", "wc", "sort",
        "uniq", "tr", "sed", "awk", "cargo", "rustc", "npm", "pnpm", "yarn", "node", "python3",
        "pytest", "go", "make", "just",
    ];

    for line in candidates {
        let t = line.trim_start();
        let token = t
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_start_matches('(');
        if TOKENS.iter().any(|v| v.eq_ignore_ascii_case(token)) {
            // Ignore obvious prose like "run: find ..." but accept raw commands.
            if t.to_ascii_lowercase().starts_with("run: ")
                || t.to_ascii_lowercase().starts_with("command: ")
            {
                continue;
            }
            return Some(t.to_string());
        }
    }
    None
}

fn extract_read_file_candidate(text: &str) -> Option<String> {
    // Heuristic: if the model says it's going to "read/open/check/look at" a file,
    // extract a likely file path token and auto-run read_file.
    let lower = text.to_ascii_lowercase();
    let intent = ["read", "open", "check", "look at", "inspect", "view"]
        .iter()
        .any(|k| lower.contains(k));
    if !intent {
        return None;
    }

    // Allowed extensions (keep aligned with looks_like_file_or_path_request)
    const EXTS: &[&str] = &[
        "rs", "toml", "md", "txt", "json", "yaml", "yml", "ts", "tsx", "js", "jsx", "py", "sql",
        "go", "java", "kt", "c", "h", "cpp", "hpp",
    ];

    let mut best: Option<String> = None;
    for raw in text.split_whitespace() {
        let w = raw.trim_matches(|c: char| {
            !(c.is_ascii_alphanumeric()
                || matches!(c, '.' | '_' | '-' | '/' | '\\'))
        });
        if w.is_empty() {
            continue;
        }
        let Some(dot) = w.rfind('.') else { continue };
        let ext = w[dot + 1..].to_ascii_lowercase();
        if !EXTS.iter().any(|e| *e == ext) {
            continue;
        }
        // Prefer paths with separators, then shorter tokens.
        let score = (if w.contains('/') || w.contains('\\') { 0 } else { 1 }, w.len());
        match &best {
            None => best = Some(w.to_string()),
            Some(existing) => {
                let existing_score = (
                    if existing.contains('/') || existing.contains('\\') { 0 } else { 1 },
                    existing.len(),
                );
                if score < existing_score {
                    best = Some(w.to_string());
                }
            }
        }
    }
    best
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
        .begin_agent_run(
            root,
            session_id,
            agent_id,
            None,
            Some(detail.to_string()),
        )
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
                let _ = manager
                    .finish_agent_run(&run_id, status, Some(msg))
                    .await;
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
        Ok(settings) => Json(SettingsResponse { mode: settings.mode }).into_response(),
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
    let session_id = req.session_id.clone().unwrap_or_else(|| "default".to_string());
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

    // Check for @Lead or @Coder prefix
    let (target_id, clean_msg) = if req.message.starts_with("@Lead ") {
        ("lead", req.message.strip_prefix("@Lead ").unwrap())
    } else if req.message.starts_with("@Coder ") {
        ("coder", req.message.strip_prefix("@Coder ").unwrap())
    } else {
        (req.agent_id.as_str(), req.message.as_str())
    };

    let target_id = target_id.to_string();
    let clean_msg = clean_msg.to_string();
    let trimmed_msg = clean_msg.trim();

    let kind = state
        .manager
        .resolve_agent_kind(&target_id)
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
                    let _ = state.manager.db.add_chat_message(crate::db::ChatMessageRecord {
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

                // Persist user message in DB immediately so fetchLeadState sees it.
                if let Ok(ctx) = state.manager.get_or_create_project(root.clone()).await {
                    let _ = ctx.state_fs.append_message(
                        "user",
                        &target_id,
                        &clean_msg,
                        None,
                        session_id.as_deref(),
                    );

                    let _ = state.manager.db.add_chat_message(crate::db::ChatMessageRecord {
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
                    let key =
                        queue_key(&project_root_for_queue, &session_id_for_queue, &target_id_clone);
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
                        .unwrap_or_else(|| "Initialize this workspace and summarize status.".to_string());

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
                                session_id: session_id.clone().unwrap_or_else(|| "default".to_string()),
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

                // In chat mode, avoid jumping into the structured multi-step loop for file/path
                // requests. We prefer one-tool execution + plain-text follow-up to prevent
                // exhausting max tool iterations on simple read/search requests.

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

                        // If the model asked for a tool, don't dump the raw model output (often multi-JSON)
                        // into the chat UI. Instead send a clean single tool JSON message and proceed.
                        let mut handled_tool = false;
                        let mut forced_tool_from_command: Option<(String, serde_json::Value)> = None;
                        if let Ok(action) = crate::engine::parse_first_action(&full_response) {
                            if let crate::engine::ModelAction::Tool { tool, args } = action {
                                handled_tool = true;
                                forced_tool_from_command = Some((tool, args));
                            }
                        }
                        if !handled_tool {
                            if let Some(cmd) = extract_shell_command_candidate(&full_response) {
                                // Model suggested a raw shell command in chat mode. Convert it into a tool call.
                                handled_tool = true;
                                forced_tool_from_command = Some((
                                    "run_command".to_string(),
                                    serde_json::json!({ "cmd": cmd, "timeout_ms": 30000 }),
                                ));
                            }
                        }
                        if !handled_tool {
                            if let Some(path) = extract_read_file_candidate(&full_response) {
                                handled_tool = true;
                                forced_tool_from_command = Some((
                                    "read_file".to_string(),
                                    serde_json::json!({ "path": path, "max_bytes": 8000 }),
                                ));
                            }
                        }

                        if let Some((tool, args)) = forced_tool_from_command {
                            {
                                state_clone
                                    .send_agent_status(
                                        target_id_clone.clone(),
                                        "calling_tool".to_string(),
                                        Some(format!("Calling {}", tool)),
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
                                if let Ok(ctx) = manager.get_or_create_project(root_clone.clone()).await {
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
                                        session_id: session_id.clone().unwrap_or_else(|| "default".to_string()),
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

                                // 1. Execute the tool that was just requested in chat
                                let write_path = if matches!(tool.as_str(), "write_file" | "Write") {
                                    extract_tool_path_arg(&args)
                                } else {
                                    None
                                };
                                let call = crate::engine::tools::ToolCall { tool: tool.clone(), args };
                                match engine.tools.execute(call) {
                                    Ok(result) => {
                                        let rendered_model = crate::engine::render_tool_result(&result);
                                        let rendered_public =
                                            crate::engine::render_tool_result_public(&result);
                                        engine.upsert_observation("tool", &tool, rendered_model.clone());

                                        // Record observation in DB
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
                                                "thinking".to_string(),
                                                Some("Thinking".to_string()),
                                            )
                                            .await;

                                        if prompt_mode == crate::engine::PromptMode::Chat {
                                            // Chat mode: always summarize tool observation directly in plain text.
                                            // This avoids re-entering structured loops for simple read/search/list
                                            // requests, which can hit max_iters without producing user-facing output.
                                            if matches!(tool.as_str(), "write_file" | "Write") {
                                                let mut observation_for_summary = rendered_public.clone();
                                                if let Some(path) = write_path {
                                                    let readback = engine.tools.execute(crate::engine::tools::ToolCall {
                                                        tool: "read_file".to_string(),
                                                        args: serde_json::json!({
                                                            "path": path,
                                                            "max_bytes": 8000
                                                        }),
                                                    });
                                                    if let Ok(read_result) = readback {
                                                        let read_public =
                                                            crate::engine::render_tool_result_public(&read_result);
                                                        let _ = engine
                                                            .manager_db_add_observation(
                                                                "read_file",
                                                                &read_public,
                                                                session_id.as_deref(),
                                                            )
                                                            .await;
                                                        let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                                                        state_clone
                                                            .send_agent_status(
                                                                target_id_clone.clone(),
                                                                "thinking".to_string(),
                                                                Some("Thinking".to_string()),
                                                            )
                                                            .await;
                                                        observation_for_summary = format!(
                                                            "{}\n\nPost-write readback:\n{}",
                                                            observation_for_summary, read_public
                                                        );
                                                    }
                                                }
                                                let followup_prompt = format!(
                                                    "Use the observation below to answer the user's request in plain text.\n\nUser request: {}\n\nObservation:\n{}\n\nRequirements:\n- Be concise but concrete.\n- If you ran a command, summarize key results.\n- If you read or wrote files, summarize what you found/changed.",
                                                    clean_msg_clone,
                                                    observation_for_summary
                                                );

                                                let mut followup_response = String::new();
                                                if let Ok(mut followup_stream) =
                                                    engine
                                                        .chat_stream(
                                                            &followup_prompt,
                                                            session_id.as_deref(),
                                                            crate::engine::PromptMode::Chat,
                                                        )
                                                        .await
                                                {
                                                    while let Some(token_result) = followup_stream.next().await {
                                                        if let Ok(token) = token_result {
                                                            followup_response.push_str(&token);
                                                            let _ = events_tx_clone.send(ServerEvent::Token {
                                                                agent_id: target_id_clone.clone(),
                                                                token,
                                                            });
                                                        }
                                                    }

                                                    let _ = engine
                                                        .finalize_chat(
                                                            &followup_prompt,
                                                            &followup_response,
                                                            session_id.as_deref(),
                                                            crate::engine::PromptMode::Chat,
                                                        )
                                                        .await;
                                                    let _ = events_tx_clone.send(ServerEvent::Message {
                                                        from: target_id_clone.clone(),
                                                        to: "user".to_string(),
                                                        content: followup_response.clone(),
                                                    });
                                                }
                                                let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                                            } else {
                                                let followup_prompt = format!(
                                                    "Use the observation below to answer the user's request in plain text.\n\nUser request: {}\n\nObservation:\n{}\n\nRequirements:\n- Be concise but concrete.\n- If you ran a command, summarize key results.\n- If you read files, mention the file path(s) and key findings.",
                                                    clean_msg_clone,
                                                    rendered_public
                                                );

                                                let mut followup_response = String::new();
                                                if let Ok(mut followup_stream) =
                                                    engine
                                                        .chat_stream(
                                                            &followup_prompt,
                                                            session_id.as_deref(),
                                                            crate::engine::PromptMode::Chat,
                                                        )
                                                        .await
                                                {
                                                    while let Some(token_result) = followup_stream.next().await {
                                                        if let Ok(token) = token_result {
                                                            followup_response.push_str(&token);
                                                            let _ = events_tx_clone.send(ServerEvent::Token {
                                                                agent_id: target_id_clone.clone(),
                                                                token,
                                                            });
                                                        }
                                                    }

                                                    let _ = engine
                                                        .finalize_chat(
                                                            &followup_prompt,
                                                            &followup_response,
                                                            session_id.as_deref(),
                                                            crate::engine::PromptMode::Chat,
                                                        )
                                                        .await;
                                                    let _ = events_tx_clone.send(ServerEvent::Message {
                                                        from: target_id_clone.clone(),
                                                        to: "user".to_string(),
                                                        content: followup_response.clone(),
                                                    });
                                                }
                                                let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                                            }
                                        } else {
                                            let followup_prompt = format!(
                                                "Use the observation below to answer the user's request in plain text.\n\nUser request: {}\n\nObservation:\n{}",
                                                clean_msg_clone,
                                                rendered_public
                                            );

                                            let mut followup_response = String::new();
                                            if let Ok(mut followup_stream) =
                                                engine
                                                    .chat_stream(
                                                        &followup_prompt,
                                                        session_id.as_deref(),
                                                        crate::engine::PromptMode::Chat,
                                                    )
                                                    .await
                                            {
                                                while let Some(token_result) = followup_stream.next().await {
                                                    if let Ok(token) = token_result {
                                                        followup_response.push_str(&token);
                                                        let _ = events_tx_clone.send(ServerEvent::Token {
                                                            agent_id: target_id_clone.clone(),
                                                            token,
                                                        });
                                                    }
                                                }

                                                let _ = engine
                                                    .finalize_chat(
                                                        &followup_prompt,
                                                        &followup_response,
                                                        session_id.as_deref(),
                                                        crate::engine::PromptMode::Chat,
                                                    )
                                                    .await;
                                                let _ = events_tx_clone.send(ServerEvent::Message {
                                                    from: target_id_clone.clone(),
                                                    to: "user".to_string(),
                                                    content: followup_response.clone(),
                                                });
                                                let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("Tool execution failed ({}): {}", tool, e);
                                        // Record the tool error as an observation so the loop can self-correct.
                                        let rendered = format!("tool_error: tool={} error={}", tool, e);
                                        engine.upsert_observation("error", &tool, rendered.clone());
                                        let _ = engine
                                            .manager_db_add_observation(&tool, &rendered, session_id.as_deref())
                                            .await;
                                        let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                                        let task_for_loop =
                                            clean_msg_clone.trim().to_string();
                                        engine.task = Some(task_for_loop);
                                        // Ask the model again with the error + schema.
                                        let outcome = run_loop_with_tracking(
                                            &manager,
                                            &root_clone,
                                            &mut engine,
                                            &target_id_clone,
                                            session_id.as_deref(),
                                            "chat:error-retry",
                                        )
                                        .await;
                                        if let Ok(outcome) = &outcome {
                                            emit_outcome_event(outcome, &events_tx_clone, &target_id_clone);
                                        } else {
                                            let _ = events_tx_clone.send(ServerEvent::Message {
                                                from: target_id_clone.clone(),
                                                to: "user".to_string(),
                                                content: format!("Tool execution failed ({}): {}", tool, e),
                                            });
                                            if let Ok(ctx) = manager.get_or_create_project(root_clone.clone()).await {
                                                let err_msg = format!("Tool execution failed ({}): {}", tool, e);
                                                let _ = ctx.state_fs.append_message(
                                                    &target_id_clone,
                                                    "user",
                                                    &err_msg,
                                                    None,
                                                    session_id.as_deref(),
                                                );
                                                let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                                                    repo_path: root_clone.to_string_lossy().to_string(),
                                                    session_id: session_id.clone().unwrap_or_else(|| "default".to_string()),
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
                                        }
                                        let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                                    }
                                };
                            }
                        }

                        if !handled_tool {
                            // Finalize chat in engine (updates history and DB) only when we're showing
                            // the model's response directly to the user.
                            let _ = engine
                                .finalize_chat(
                                    &clean_msg_clone,
                                    &full_response,
                                    session_id.as_deref(),
                                    prompt_mode,
                                )
                                .await;
                            // Normal assistant message (finalize/other text)
                            let _ = events_tx_clone.send(ServerEvent::Message {
                                from: target_id_clone.clone(),
                                to: "user".to_string(),
                                content: full_response.clone(),
                            });
                            // Force UI refresh after sending final message
                            let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                        } else {
                            // Tool path: ensure UI refresh even though we didn't store the raw assistant text.
                            let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                        }
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
                                session_id: session_id.clone().unwrap_or_else(|| "default".to_string()),
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
