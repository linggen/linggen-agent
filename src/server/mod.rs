mod agent_api;
mod chat_api;
mod chat_helpers;
mod projects_api;
mod workspace_api;

use crate::agent_manager::AgentManager;
use axum::{
    extract::State,
    http::Uri,
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{delete, get, post},
    Router,
};
use rust_embed::RustEmbed;
use serde::Serialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::info;

use agent_api::{cancel_agent_run, run_agent, set_task};
use chat_api::{chat_handler, clear_chat_history_api, get_settings_api, update_settings_api};
use projects_api::{
    add_project, create_session, get_agent_context_api, list_agent_children_api,
    list_agent_runs_api, list_agents_api, list_models_api, list_projects, list_sessions,
    list_skills, remove_project,
    remove_session_api,
};
use workspace_api::{get_agent_tree, get_lead_state, list_files, read_file_api};

#[derive(RustEmbed)]
#[folder = "ui/dist/"]
struct Assets;

pub struct ServerState {
    pub manager: Arc<AgentManager>,
    pub dev_mode: bool,
    pub events_tx: broadcast::Sender<ServerEvent>,
    pub skill_manager: Arc<crate::skills::SkillManager>,
    pub queued_chats: Arc<Mutex<HashMap<String, Vec<QueuedChatItem>>>>,
    status_seq: AtomicU64,
    active_statuses: Arc<Mutex<HashMap<String, ActiveStatusRecord>>>,
    pub queue_seq: AtomicU64,
    pub event_seq: AtomicU64,
}

#[derive(Debug, Clone)]
struct ActiveStatusRecord {
    status_id: String,
    status: String,
    detail: Option<String>,
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
    SubagentSpawned {
        parent_id: String,
        subagent_id: String,
        task: String,
    },
    SubagentResult {
        parent_id: String,
        subagent_id: String,
        outcome: crate::engine::AgentOutcome,
    },
    AgentStatus {
        agent_id: String,
        status: String,
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lifecycle: Option<String>, // "doing" | "done"
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
    ContextUsage {
        agent_id: String,
        stage: String,
        message_count: usize,
        char_count: usize,
        estimated_tokens: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_limit: Option<usize>,
        compressed: bool,
        summary_count: usize,
    },
    Outcome {
        agent_id: String,
        outcome: crate::engine::AgentOutcome,
    },
}

impl ServerState {
    pub async fn send_agent_status(&self, agent_id: String, status: String, detail: Option<String>) {
        let mut done_event: Option<ServerEvent> = None;
        let mut status_id: Option<String> = None;
        let mut lifecycle: Option<String> = None;

        {
            let mut active = self.active_statuses.lock().await;
            if status.eq_ignore_ascii_case("idle") {
                if let Some(prev) = active.remove(&agent_id) {
                    done_event = Some(ServerEvent::AgentStatus {
                        agent_id: agent_id.clone(),
                        status: prev.status,
                        detail: prev.detail,
                        status_id: Some(prev.status_id),
                        lifecycle: Some("done".to_string()),
                    });
                }
            } else {
                if let Some(prev) = active.get(&agent_id).cloned() {
                    if !prev.status.eq_ignore_ascii_case(&status) {
                        done_event = Some(ServerEvent::AgentStatus {
                            agent_id: agent_id.clone(),
                            status: prev.status,
                            detail: prev.detail,
                            status_id: Some(prev.status_id),
                            lifecycle: Some("done".to_string()),
                        });
                        active.remove(&agent_id);
                    } else {
                        status_id = Some(prev.status_id.clone());
                        lifecycle = Some("doing".to_string());
                        active.insert(
                            agent_id.clone(),
                            ActiveStatusRecord {
                                status_id: prev.status_id,
                                status: status.clone(),
                                detail: detail.clone(),
                            },
                        );
                    }
                }

                if status_id.is_none() {
                    let next_id = format!("status-{}", self.status_seq.fetch_add(1, Ordering::Relaxed));
                    status_id = Some(next_id.clone());
                    lifecycle = Some("doing".to_string());
                    active.insert(
                        agent_id.clone(),
                        ActiveStatusRecord {
                            status_id: next_id,
                            status: status.clone(),
                            detail: detail.clone(),
                        },
                    );
                }
            }
        }

        if let Some(done) = done_event {
            let _ = self.events_tx.send(done);
        }

        let _ = self.events_tx.send(ServerEvent::AgentStatus {
            agent_id,
            status,
            detail,
            status_id,
            lifecycle,
        });
    }
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

    let state = Arc::new(ServerState {
        manager,
        dev_mode,
        events_tx,
        skill_manager,
        queued_chats: Arc::new(Mutex::new(HashMap::new())),
        status_seq: AtomicU64::new(1),
        active_statuses: Arc::new(Mutex::new(HashMap::new())),
        queue_seq: AtomicU64::new(1),
        event_seq: AtomicU64::new(1),
    });

    // Bridge internal AgentManager events to SSE for the UI.
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            while let Some(event) = agent_events_rx.recv().await {
                match event {
                    crate::agent_manager::AgentEvent::StateUpdated => {
                        let _ = state_clone.events_tx.send(ServerEvent::StateUpdated);
                    }
                    crate::agent_manager::AgentEvent::Message { from, to, content } => {
                        let _ = state_clone
                            .events_tx
                            .send(ServerEvent::Message { from, to, content });
                    }
                    crate::agent_manager::AgentEvent::AgentStatus {
                        agent_id,
                        status,
                        detail,
                    } => {
                        state_clone.send_agent_status(agent_id, status, detail).await;
                    }
                    crate::agent_manager::AgentEvent::SubagentSpawned {
                        parent_id,
                        subagent_id,
                        task,
                    } => {
                        let _ = state_clone.events_tx.send(ServerEvent::SubagentSpawned {
                            parent_id,
                            subagent_id,
                            task,
                        });
                    }
                    crate::agent_manager::AgentEvent::SubagentResult {
                        parent_id,
                        subagent_id,
                        outcome,
                    } => {
                        let _ = state_clone.events_tx.send(ServerEvent::SubagentResult {
                            parent_id,
                            subagent_id,
                            outcome,
                        });
                    }
                    crate::agent_manager::AgentEvent::Outcome { agent_id, outcome } => {
                        let _ = state_clone
                            .events_tx
                            .send(ServerEvent::Outcome { agent_id, outcome });
                    }
                    crate::agent_manager::AgentEvent::ContextUsage {
                        agent_id,
                        stage,
                        message_count,
                        char_count,
                        estimated_tokens,
                        token_limit,
                        compressed,
                        summary_count,
                    } => {
                        let _ = state_clone.events_tx.send(ServerEvent::ContextUsage {
                            agent_id,
                            stage,
                            message_count,
                            char_count,
                            estimated_tokens,
                            token_limit,
                            compressed,
                            summary_count,
                        });
                    }
                    crate::agent_manager::AgentEvent::TaskUpdate { .. } => {
                        // UI will refresh state from DB
                        let _ = state_clone.events_tx.send(ServerEvent::StateUpdated);
                    }
                }
            }
        });
    }

    let app = Router::new()
        .route("/api/projects", get(list_projects))
        .route("/api/projects", post(add_project))
        .route("/api/projects", delete(remove_project))
        .route("/api/agents", get(list_agents_api))
        .route("/api/agent-runs", get(list_agent_runs_api))
        .route("/api/agent-children", get(list_agent_children_api))
        .route("/api/agent-context", get(get_agent_context_api))
        .route("/api/models", get(list_models_api))
        .route("/api/skills", get(list_skills))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions", post(create_session))
        .route("/api/sessions", delete(remove_session_api))
        .route("/api/settings", get(get_settings_api))
        .route("/api/settings", post(update_settings_api))
        .route("/api/task", post(set_task))
        .route("/api/run", post(run_agent))
        .route("/api/agent-cancel", post(cancel_agent_run))
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
    #[derive(Serialize)]
    struct SseEnvelope {
        seq: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        project_root: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(flatten)]
        event: ServerEvent,
    }

    fn infer_event_context(event: &ServerEvent) -> (Option<String>, Option<String>) {
        match event {
            ServerEvent::QueueUpdated {
                project_root,
                session_id,
                ..
            } => (Some(project_root.clone()), Some(session_id.clone())),
            ServerEvent::SettingsUpdated { project_root, .. } => {
                (Some(project_root.clone()), None)
            }
            _ => (None, None),
        }
    }

    let rx = state.events_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |msg| {
        let event = match msg {
            Ok(event) => event,
            // If the broadcast stream lags/drops messages, emit a parseable event that
            // prompts the UI to refresh from DB instead of sending non-JSON "error".
            Err(_) => ServerEvent::StateUpdated,
        };
        let filtered = crate::server::chat_helpers::sanitize_server_event_for_ui(event)?;
        let seq = state.event_seq.fetch_add(1, Ordering::Relaxed);
        let (project_root, session_id) = infer_event_context(&filtered);
        let envelope = SseEnvelope {
            seq,
            project_root,
            session_id,
            event: filtered,
        };
        let data = serde_json::to_string(&envelope).unwrap_or_default();
        Some(Ok(Event::default().data(data)))
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
            Ok(status) => axum::Json(status).into_response(),
            Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    } else {
        (axum::http::StatusCode::NOT_FOUND, "No Ollama models configured").into_response()
    }
}
