use crate::agent_manager::AgentManager;
use crate::engine::PromptMode;
use crate::skills::Skill;
use crate::state_fs::StateFile;
use axum::{
    extract::{Query, State},
    http::{StatusCode, Uri},
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{delete, get, post},
    Json, Router,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tokio::sync::broadcast;
use tokio::sync::Mutex;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::info;

#[derive(RustEmbed)]
#[folder = "ui/dist/"]
struct Assets;

pub struct ServerState {
    pub manager: Arc<AgentManager>,
    pub dev_mode: bool,
    pub events_tx: broadcast::Sender<ServerEvent>,
    pub skill_manager: Arc<crate::skills::SkillManager>,
    pub queued_chats: Arc<Mutex<HashMap<String, Vec<QueuedChatItem>>>>,
    pub queue_seq: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueuedChatItem {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub preview: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ServerEvent {
    StateUpdated,
    Message {
        from: String,
        to: String,
        content: String,
    },
    AgentStatus {
        agent_id: String,
        status: String,
    },
    SettingsUpdated {
        project_root: String,
        mode: String,
    },
    QueueUpdated {
        project_root: String,
        session_id: String,
        agent_id: String,
        items: Vec<QueuedChatItem>,
    },
    Token {
        agent_id: String,
        token: String,
    },
    Observation {
        agent_id: String,
        content: String,
    },
    Outcome {
        agent_id: String,
        outcome: crate::engine::AgentOutcome,
    },
}

pub async fn start_server(
    manager: Arc<AgentManager>,
    skill_manager: Arc<crate::skills::SkillManager>,
    port: u16,
    dev_mode: bool,
    mut agent_events_rx: mpsc::UnboundedReceiver<crate::agent_manager::AgentEvent>,
) -> anyhow::Result<()> {
    info!("linggen-agent server starting on port {}...", port);

    // SSE can be bursty (tokens, tool steps). Use a larger buffer to reduce lag drops.
    let (events_tx, _) = broadcast::channel(4096);

    // Bridge internal AgentManager events to SSE for the UI.
    {
        let events_tx = events_tx.clone();
        tokio::spawn(async move {
            while let Some(event) = agent_events_rx.recv().await {
                let mapped = match event {
                    crate::agent_manager::AgentEvent::StateUpdated => Some(ServerEvent::StateUpdated),
                    crate::agent_manager::AgentEvent::Message { from, to, content } => {
                        Some(ServerEvent::Message { from, to, content })
                    }
                    crate::agent_manager::AgentEvent::Outcome { agent_id, outcome } => {
                        Some(ServerEvent::Outcome { agent_id, outcome })
                    }
                    crate::agent_manager::AgentEvent::TaskUpdate { .. } => {
                        // UI will refresh state from DB
                        Some(ServerEvent::StateUpdated)
                    }
                };

                if let Some(ev) = mapped {
                    let _ = events_tx.send(ev);
                }
            }
        });
    }

    let state = Arc::new(ServerState {
        manager,
        dev_mode,
        events_tx,
        skill_manager,
        queued_chats: Arc::new(Mutex::new(HashMap::new())),
        queue_seq: AtomicU64::new(1),
    });

    let app = Router::new()
        .route("/api/projects", get(list_projects))
        .route("/api/projects", post(add_project))
        .route("/api/projects", delete(remove_project))
        .route("/api/agents", get(list_agents_api))
        .route("/api/models", get(list_models_api))
        .route("/api/skills", get(list_skills))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions", post(create_session))
        .route("/api/sessions", delete(remove_session_api))
        .route("/api/settings", get(get_settings_api))
        .route("/api/settings", post(update_settings_api))
        .route("/api/task", post(set_task))
        .route("/api/run", post(run_agent))
        .route("/api/chat", post(chat_handler))
        .route("/api/chat/clear", post(clear_chat_history_api))
        .route("/api/workspace/tree", get(get_agent_tree))
        .route("/api/files", get(list_files))
        .route("/api/file", get(read_file_api))
        .route("/api/lead/state", get(get_lead_state))
        .route("/api/events", get(events_handler))
        .route("/api/utils/pick-folder", get(pick_folder))
        .route("/api/utils/ollama-status", get(get_ollama_status))
        .fallback(static_handler)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!("Server running on http://localhost:{}", port);
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Deserialize)]
struct TaskRequest {
    project_root: String,
    agent_id: String,
    task: String,
}

async fn set_task(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<TaskRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    match state
        .manager
        .get_or_create_agent(&root, &req.agent_id)
        .await
    {
        Ok(agent) => {
            let mut engine = agent.lock().await;
            engine.set_task(req.task.clone());

            // Persist Lead task if it's Lead
            if req.agent_id == "lead" {
                if let Ok(ctx) = state.manager.get_or_create_project(root).await {
                    let lead_task = StateFile::PmTask {
                        id: format!(
                            "lead-{}",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs()
                        ),
                        status: "active".to_string(),
                        assigned_tasks: Vec::new(),
                    };
                    let _ = ctx.state_fs.write_file("active.md", &lead_task, &req.task);
                    let _ = state.events_tx.send(ServerEvent::StateUpdated);
                }
            }

            StatusCode::OK
        }
        Err(_) => StatusCode::NOT_FOUND,
    }
}

#[derive(Deserialize)]
struct RunRequest {
    project_root: String,
    agent_id: String,
    session_id: Option<String>,
}

async fn run_agent(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let agent_id = req.agent_id.clone();
    let session_id = req.session_id.clone();
    let events_tx = state.events_tx.clone();
    let manager = state.manager.clone();

    match state
        .manager
        .get_or_create_agent(&root, &req.agent_id)
        .await
    {
        Ok(agent) => {
            tokio::spawn(async move {
                let _ = events_tx.send(ServerEvent::AgentStatus {
                    agent_id: agent_id.clone(),
                    status: "working".to_string(),
                });
                let mut engine = agent.lock().await;
                let outcome = engine
                    .run_agent_loop(session_id.as_deref())
                    .await
                    .unwrap_or(crate::engine::AgentOutcome::None);

                // If Lead finalized a task, save it
                if agent_id == "lead" {
                    if let crate::engine::AgentOutcome::Task(packet) = &outcome {
                        if let Ok(ctx) = manager.get_or_create_project(root).await {
                            let task_id = format!(
                                "task-{}",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs()
                            );
                            let coder_task = StateFile::CoderTask {
                                id: task_id.clone(),
                                status: "queued".to_string(),
                                story_id: None,
                                assigned_to: "coder".to_string(),
                            };
                            let body = format!(
                                "## {}\n\n### User Stories\n{}\n\n### Acceptance Criteria\n{}",
                                packet.title,
                                packet.user_stories.join("\n"),
                                packet.acceptance_criteria.join("\n")
                            );
                            let _ = ctx.state_fs.write_file(
                                &format!("tasks/{}.md", task_id),
                                &coder_task,
                                &body,
                            );
                            let _ = events_tx.send(ServerEvent::StateUpdated);
                        }
                    }
                }

                let _ = events_tx.send(ServerEvent::Outcome {
                    agent_id: agent_id.clone(),
                    outcome,
                });
                let _ = events_tx.send(ServerEvent::AgentStatus {
                    agent_id: agent_id.clone(),
                    status: "idle".to_string(),
                });
            });

            Json(serde_json::json!({ "status": "started" })).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct ChatRequest {
    project_root: String,
    agent_id: String,
    message: String,
    session_id: Option<String>,
}

#[derive(Deserialize)]
struct ClearChatRequest {
    project_root: String,
    session_id: Option<String>,
}

#[derive(Deserialize)]
struct SettingsQuery {
    project_root: String,
}

#[derive(Deserialize)]
struct UpdateSettingsRequest {
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

fn queue_key(project_root: &str, session_id: &str, agent_id: &str) -> String {
    format!("{project_root}|{session_id}|{agent_id}")
}

fn queue_preview(message: &str) -> String {
    const LIMIT: usize = 100;
    let trimmed = message.trim();
    if trimmed.len() <= LIMIT {
        trimmed.to_string()
    } else {
        format!("{}...", &trimmed[..LIMIT])
    }
}

async fn emit_queue_updated(
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

async fn get_settings_api(
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

async fn update_settings_api(
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

async fn clear_chat_history_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ClearChatRequest>,
) -> impl IntoResponse {
    let session_id = req.session_id.unwrap_or_else(|| "default".to_string());
    match state
        .manager
        .db
        .clear_chat_history(&req.project_root, &session_id)
    {
        Ok(removed) => {
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(serde_json::json!({ "removed": removed })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn chat_handler(
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
                emit_queue_updated(&state, &project_root_str, &effective_session_id, &target_id).await;
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

            // Emit user message event immediately
            let _ = events_tx.send(ServerEvent::Message {
                from: "user".to_string(),
                to: target_id.clone(),
                content: clean_msg.clone(),
            });

            // Persist user message in DB immediately so fetchLeadState sees it
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

            let emit_outcome = |outcome: &crate::engine::AgentOutcome,
                                events_tx: &broadcast::Sender<ServerEvent>,
                                from_id: &str| {
                match outcome {
                    crate::engine::AgentOutcome::Task(packet) => {
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
                    crate::engine::AgentOutcome::Ask(question) => {
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
            };

            tokio::spawn(async move {
                if let Some(queued_id) = queued_item_id.as_deref() {
                    let key = queue_key(&project_root_for_queue, &session_id_for_queue, &target_id_clone);
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
                }

                let _ = events_tx_clone.send(ServerEvent::AgentStatus {
                    agent_id: target_id_clone.clone(),
                    status: "working".to_string(),
                });
                let mut engine = agent.lock().await;
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

                    let outcome = engine.run_agent_loop(session_id.as_deref()).await;
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
                            emit_outcome(outcome, &events_tx_clone, &target_id_clone);
                        }
                        // Force UI refresh after loop
                        let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                    }

                    let _ = events_tx_clone.send(ServerEvent::AgentStatus {
                        agent_id: target_id_clone.clone(),
                        status: "idle".to_string(),
                    });
                    return;
                }

                let prompt_mode = engine.get_prompt_mode();
                match engine
                    .chat_stream(&clean_msg_clone, session_id.as_deref(), prompt_mode)
                    .await
                {
                    Ok(mut stream) => {
                        while let Some(token_result) = stream.next().await {
                            if let Ok(token) = token_result {
                                full_response.push_str(&token);
                                let _ = events_tx_clone.send(ServerEvent::Token {
                                    agent_id: target_id_clone.clone(),
                                    token,
                                });
                            }
                        }

                        // Finalize chat in engine (updates history and DB)
                        let _ = engine
                            .finalize_chat(
                                &clean_msg_clone,
                                &full_response,
                                session_id.as_deref(),
                                prompt_mode,
                            )
                            .await;

                        // If the model asked for a tool, don't dump the raw model output (often multi-JSON)
                        // into the chat UI. Instead send a clean single tool JSON message and proceed.
                        let mut handled_tool = false;
                        if let Ok(action) = crate::engine::parse_first_action(&full_response) {
                            if let crate::engine::ModelAction::Tool { tool, args } = action {
                                handled_tool = true;
                                let _ = events_tx_clone.send(ServerEvent::Message {
                                    from: target_id_clone.clone(),
                                    to: "user".to_string(),
                                    content: serde_json::json!({
                                        "type": "tool",
                                        "tool": tool.clone(),
                                        "args": args.clone()
                                    })
                                    .to_string(),
                                });
                                if let Ok(ctx) = manager.get_or_create_project(root_clone.clone()).await {
                                    let tool_msg = serde_json::json!({
                                        "type": "tool",
                                        "tool": tool.clone(),
                                        "args": args.clone()
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
                                let call = crate::engine::tools::ToolCall { tool: tool.clone(), args };
                                match engine.tools.execute(call) {
                                    Ok(result) => {
                                        let rendered_model = crate::engine::render_tool_result(&result);
                                        let rendered_public =
                                            crate::engine::render_tool_result_public(&result);
                                        engine.observations.push(rendered_model.clone());

                                        // Record observation in DB
                                        let _ = engine
                                            .manager_db_add_observation(
                                                &tool,
                                                &rendered_public,
                                                session_id.as_deref(),
                                            )
                                            .await;

                                        let _ = events_tx_clone.send(ServerEvent::Observation {
                                            agent_id: target_id_clone.clone(),
                                            content: format!("Executed {}: {}", tool, rendered_public),
                                        });

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
                                            if let Ok(action) =
                                                crate::engine::parse_first_action(&followup_response)
                                            {
                                                if let crate::engine::ModelAction::Tool { .. } = action {
                                                    // Model asked for another tool in follow-up; continue the autonomous loop
                                                    // instead of dumping raw JSON into chat and stopping.
                                                    engine.task = Some(clean_msg_clone.clone());
                                                    let outcome = engine
                                                        .run_agent_loop(session_id.as_deref())
                                                        .await;
                                                    if let Ok(outcome) = &outcome {
                                                        emit_outcome(
                                                            outcome,
                                                            &events_tx_clone,
                                                            &target_id_clone,
                                                        );
                                                    }
                                                    let _ = events_tx_clone
                                                        .send(ServerEvent::StateUpdated);
                                                } else {
                                                    let _ = events_tx_clone.send(ServerEvent::Message {
                                                        from: target_id_clone.clone(),
                                                        to: "user".to_string(),
                                                        content: followup_response.clone(),
                                                    });
                                                    let _ = events_tx_clone.send(ServerEvent::StateUpdated);
                                                }
                                            } else {
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
                                        engine.observations.push(rendered.clone());
                                        let _ = engine
                                            .manager_db_add_observation(&tool, &rendered, session_id.as_deref())
                                            .await;
                                        let _ = events_tx_clone.send(ServerEvent::Observation {
                                            agent_id: target_id_clone.clone(),
                                            content: format!("Tool error {}: {}", tool, e),
                                        });
                                        let task_for_loop = if clean_msg_clone.starts_with('/') {
                                            clean_msg_clone
                                                .splitn(2, ' ')
                                                .nth(1)
                                                .unwrap_or("Initialize and proceed.")
                                                .trim()
                                                .to_string()
                                        } else {
                                            clean_msg_clone.clone()
                                        };
                                        engine.task = Some(task_for_loop);
                                        // Ask the model again with the error + schema.
                                        let outcome = engine.run_agent_loop(session_id.as_deref()).await;
                                        if let Ok(outcome) = &outcome {
                                            emit_outcome(outcome, &events_tx_clone, &target_id_clone);
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
                            // Normal assistant message (ask/finalize/other text)
                            let _ = events_tx_clone.send(ServerEvent::Message {
                                from: target_id_clone.clone(),
                                to: "user".to_string(),
                                content: full_response.clone(),
                            });
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
                let _ = events_tx_clone.send(ServerEvent::AgentStatus {
                    agent_id: target_id_clone.clone(),
                    status: "idle".to_string(),
                });
            });

            Json(serde_json::json!({ "status": "started" })).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct FileQuery {
    project_root: String,
    path: Option<String>,
}

async fn list_files(
    State(_state): State<Arc<ServerState>>,
    Query(query): Query<FileQuery>,
) -> impl IntoResponse {
    let rel_path = query.path.unwrap_or_default();
    let full_path = PathBuf::from(&query.project_root).join(&rel_path);

    if !full_path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let mut entries = Vec::new();
    if let Ok(dir) = std::fs::read_dir(full_path) {
        for entry in dir {
            if let Ok(entry) = entry {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                entries.push(serde_json::json!({
                    "name": name,
                    "isDir": is_dir,
                    "path": if rel_path.is_empty() { name } else { format!("{}/{}", rel_path, name) }
                }));
            }
        }
    }
    Json(entries).into_response()
}

async fn read_file_api(
    State(_state): State<Arc<ServerState>>,
    Query(query): Query<FileQuery>,
) -> impl IntoResponse {
    let rel_path = match query.path {
        Some(p) => p,
        None => return StatusCode::BAD_REQUEST.into_response(),
    };
    let full_path = PathBuf::from(&query.project_root).join(&rel_path);

    match std::fs::read_to_string(full_path) {
        Ok(content) => Json(serde_json::json!({ "content": content })).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Serialize)]
struct LeadStateResponse {
    active_lead_task: Option<(crate::state_fs::StateFile, String)>,
    user_stories: Option<(crate::state_fs::StateFile, String)>,
    tasks: Vec<(crate::state_fs::StateFile, String)>,
    messages: Vec<(crate::state_fs::StateFile, String)>,
}

#[derive(Deserialize)]
struct ProjectQuery {
    project_root: String,
    session_id: Option<String>,
}

async fn get_lead_state(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&query.project_root);
    if let Ok(ctx) = state.manager.get_or_create_project(root).await {
        let active_lead_task = ctx.state_fs.read_file("active.md").ok();
        let user_stories = ctx.state_fs.read_file("user-stories.md").ok();
        let tasks = ctx.state_fs.list_tasks().unwrap_or_default();

        // Get messages from Redb instead of StateFs
        let messages = state
            .manager
            .db
            .get_chat_history(
                &query.project_root,
                query.session_id.as_deref().unwrap_or("default"),
                None,
            )
            .unwrap_or_default();

        // Map ChatMessageRecord to the format expected by the UI
        let mapped_messages: Vec<(crate::state_fs::StateFile, String)> = messages
            .into_iter()
            .map(|m| {
                (
                    crate::state_fs::StateFile::Message {
                        id: format!("msg-{}", m.timestamp),
                        from: m.from_id,
                        to: m.to_id,
                        ts: m.timestamp,
                        task_id: None,
                    },
                    m.content,
                )
            })
            .collect();

        Json(LeadStateResponse {
            active_lead_task,
            user_stories,
            tasks,
            messages: mapped_messages,
        })
        .into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn list_projects(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    match state.manager.db.list_projects() {
        Ok(projects) => Json(projects).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn list_agents_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    match state.manager.list_agents().await {
        Ok(agents) => Json(agents).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn list_models_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let models = state.manager.models.list_models();
    Json(models).into_response()
}

async fn list_skills(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let skills: Vec<Skill> = state.skill_manager.list_skills().await;
    Json(skills).into_response()
}

async fn list_sessions(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    match state.manager.db.list_sessions(&query.project_root) {
        Ok(sessions) => Json(sessions).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
struct CreateSessionRequest {
    project_root: String,
    title: String,
}

async fn create_session(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateSessionRequest>,
) -> impl IntoResponse {
    let id = format!(
        "sess-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let session = crate::db::SessionInfo {
        id: id.clone(),
        repo_path: req.project_root,
        title: req.title,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };

    match state.manager.db.add_session(session) {
        Ok(_) => Json(serde_json::json!({ "id": id })).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
struct RemoveSessionRequest {
    project_root: String,
    session_id: String,
}

async fn remove_session_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RemoveSessionRequest>,
) -> impl IntoResponse {
    match state
        .manager
        .db
        .remove_session(&req.project_root, &req.session_id)
    {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Deserialize)]
struct AddProjectRequest {
    path: String,
}

async fn add_project(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<AddProjectRequest>,
) -> impl IntoResponse {
    let path = PathBuf::from(&req.path);
    match state.manager.get_or_create_project(path).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn remove_project(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<AddProjectRequest>, // Reuse same struct for path
) -> impl IntoResponse {
    match state.manager.db.remove_project(&req.path) {
        Ok(_) => {
            // Also remove from active projects map
            let mut projects = state.manager.projects.lock().await;
            projects.remove(&req.path);
            StatusCode::OK
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn get_agent_tree(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    let root_path = PathBuf::from(&query.project_root);
    let repo_name = root_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    match state.manager.db.get_repo_activity(&query.project_root) {
        Ok(activities) => {
            // Build a simple tree structure from activities
            let mut tree = serde_json::Map::new();
            for act in activities {
                let parts: Vec<&str> = act.file_path.split('/').collect();
                let mut current = &mut tree;
                for (i, part) in parts.iter().enumerate() {
                    if i == parts.len() - 1 {
                        current.insert(
                            part.to_string(),
                            serde_json::json!({
                                "type": "file",
                                "agent": act.agent_id,
                                "status": act.status,
                                "path": act.file_path,
                            }),
                        );
                    } else {
                        let entry = current
                            .entry(part.to_string())
                            .or_insert(serde_json::json!({
                                "type": "dir",
                                "children": {}
                            }));
                        current = entry
                            .as_object_mut()
                            .unwrap()
                            .get_mut("children")
                            .unwrap()
                            .as_object_mut()
                            .unwrap();
                    }
                }
            }

            // Wrap in a root node for the repo
            let root_tree = serde_json::json!({
                repo_name: {
                    "type": "dir",
                    "path": query.project_root,
                    "children": tree
                }
            });

            Json(root_tree).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn static_handler(State(state): State<Arc<ServerState>>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    if state.dev_mode {
        // In dev mode, we could proxy to Vite, but for simplicity in this MVP
        // we'll just try to serve from Assets or return 404.
        // Real proxying would use tower-http's ReverseProxy.
    }

    let path = if path.is_empty() { "index.html" } else { path };

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .header("Content-Type", mime.as_ref())
                .body(axum::body::Body::from(content.data))
                .unwrap()
        }
        None => {
            // Fallback to index.html for SPA routing
            let index = Assets::get("index.html").unwrap();
            Response::builder()
                .header("Content-Type", "text/html")
                .body(axum::body::Body::from(index.data))
                .unwrap()
        }
    }
}

async fn events_handler(
    State(state): State<Arc<ServerState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events_tx.subscribe();
    let stream = BroadcastStream::new(rx).map(|msg| match msg {
        Ok(event) => {
            let data = serde_json::to_string(&event).unwrap_or_default();
            Ok(Event::default().data(data))
        }
        // If the broadcast stream lags/drops messages, emit a parseable event that
        // prompts the UI to refresh from DB instead of sending non-JSON "error".
        Err(_) => {
            let data = serde_json::to_string(&ServerEvent::StateUpdated).unwrap_or_default();
            Ok(Event::default().data(data))
        }
    });

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn pick_folder() -> impl IntoResponse {
    // ... (existing code)
}

async fn get_ollama_status(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    // We'll just check the first ollama model we find
    let models = state.manager.models.list_models();
    let ollama_model = models.iter().find(|m| m.provider == "ollama");

    if let Some(m) = ollama_model {
        let client = crate::ollama::OllamaClient::new(m.url.clone(), m.api_key.clone());
        match client.get_ps().await {
            Ok(status) => Json(status).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    } else {
        (StatusCode::NOT_FOUND, "No Ollama models configured").into_response()
    }
}
