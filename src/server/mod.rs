mod agent_api;
mod chat_api;
pub(crate) mod chat_helpers;
mod config_api;
mod marketplace_api;
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
    routing::{delete, get, patch, post},
    Router,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::json;
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
use chat_api::{approve_plan_handler, chat_handler, clear_chat_history_api, reject_plan_handler};
use config_api::{get_config_api, update_config_api};
use projects_api::{
    add_project, create_session, delete_agent_file_api, delete_skill_file_api,
    get_agent_context_api, get_agent_file_api, get_skill_file_api, list_agent_children_api,
    list_agent_files_api, list_agent_runs_api, list_agents_api, list_models_api, list_projects,
    list_sessions, list_skill_files_api, list_skills, remove_project, remove_session_api,
    rename_session_api, upsert_agent_file_api, upsert_skill_file_api,
};
use marketplace_api::{builtin_skills_install, builtin_skills_list, marketplace_install, marketplace_list, marketplace_search, marketplace_uninstall};
use workspace_api::{get_agent_tree, get_workspace_state, list_files, read_file_api};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatusKind {
    Idle,
    ModelLoading,
    Thinking,
    CallingTool,
    Working,
}

impl AgentStatusKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::ModelLoading => "model_loading",
            Self::Thinking => "thinking",
            Self::CallingTool => "calling_tool",
            Self::Working => "working",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "idle" => Self::Idle,
            "model_loading" => Self::ModelLoading,
            "thinking" => Self::Thinking,
            "calling_tool" => Self::CallingTool,
            "working" => Self::Working,
            _ => Self::Working,
        }
    }
}

#[derive(Debug, Clone)]
struct ActiveStatusRecord {
    status_id: String,
    status: AgentStatusKind,
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
    QueueUpdated {
        project_root: String,
        session_id: String,
        agent_id: String,
        items: Vec<QueuedChatItem>,
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
    Token {
        agent_id: String,
        token: String,
        done: bool,
        thinking: bool,
    },
    ChangeReport {
        agent_id: String,
        files: Vec<serde_json::Value>,
        truncated_count: usize,
    },
    PlanUpdate {
        agent_id: String,
        plan: crate::engine::Plan,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSseMessage {
    pub id: String,
    pub seq: u64,
    pub rev: u64,
    pub ts_ms: u64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// UI SSE kind/phase constants
// ---------------------------------------------------------------------------

const UI_KIND_MESSAGE: &str = "message";
const UI_KIND_ACTIVITY: &str = "activity";
const UI_KIND_QUEUE: &str = "queue";
const UI_KIND_RUN: &str = "run";
const UI_KIND_TOKEN: &str = "token";

const UI_PHASE_SYNC: &str = "sync";
const UI_PHASE_OUTCOME: &str = "outcome";
const UI_PHASE_CONTEXT_USAGE: &str = "context_usage";
const UI_PHASE_SUBAGENT_SPAWNED: &str = "subagent_spawned";
const UI_PHASE_SUBAGENT_RESULT: &str = "subagent_result";
const UI_PHASE_CHANGE_REPORT: &str = "change_report";
const UI_PHASE_PLAN_UPDATE: &str = "plan_update";
const UI_PHASE_DOING: &str = "doing";
const UI_PHASE_DONE: &str = "done";

fn default_status_text(status: AgentStatusKind) -> String {
    match status {
        AgentStatusKind::ModelLoading => "Model loading...".to_string(),
        AgentStatusKind::Thinking => "Thinking...".to_string(),
        AgentStatusKind::CallingTool => "Calling tool...".to_string(),
        AgentStatusKind::Working => "Working...".to_string(),
        AgentStatusKind::Idle => "Idle".to_string(),
    }
}

fn map_server_event_to_ui_message(event: ServerEvent, seq: u64) -> Option<UiSseMessage> {
    let ts_ms = crate::util::now_ts_ms();
    match event {
        ServerEvent::Message { from, to, content } => {
            let cleaned = crate::server::chat_helpers::sanitize_message_for_ui(&from, &content)?;
            if from != "user" && crate::server::chat_helpers::is_progress_text_for_ui(&cleaned) {
                return None;
            }
            Some(UiSseMessage {
                id: format!("msg-{seq}"),
                seq,
                rev: seq,
                ts_ms,
                kind: UI_KIND_MESSAGE.to_string(),
                phase: None,
                text: Some(cleaned),
                agent_id: Some(from.clone()),
                session_id: None,
                project_root: None,
                data: Some(json!({
                    "from": from,
                    "to": to,
                    "role": if from == "user" { "user" } else { "assistant" },
                })),
            })
        }
        ServerEvent::AgentStatus {
            agent_id,
            status,
            detail,
            status_id,
            lifecycle,
        } => {
            if status.eq_ignore_ascii_case("idle") && lifecycle.is_none() {
                // Still emit the idle event so the UI can transition agent status.
                return Some(UiSseMessage {
                    id: format!("act-{seq}"),
                    seq,
                    rev: seq,
                    ts_ms,
                    kind: UI_KIND_ACTIVITY.to_string(),
                    phase: Some(UI_PHASE_DONE.to_string()),
                    text: None,
                    agent_id: Some(agent_id),
                    session_id: None,
                    project_root: None,
                    data: Some(json!({ "status": "idle" })),
                });
            }
            let phase = lifecycle.or_else(|| {
                if status.eq_ignore_ascii_case("idle") {
                    Some(UI_PHASE_DONE.to_string())
                } else {
                    Some(UI_PHASE_DOING.to_string())
                }
            });
            let text = detail
                .and_then(|v| {
                    let t = v.trim().to_string();
                    if t.is_empty() {
                        None
                    } else {
                        Some(t)
                    }
                })
                .unwrap_or_else(|| default_status_text(AgentStatusKind::from_str_loose(&status)));
            Some(UiSseMessage {
                id: status_id.unwrap_or_else(|| format!("activity-{agent_id}-{status}-{seq}")),
                seq,
                rev: seq,
                ts_ms,
                kind: UI_KIND_ACTIVITY.to_string(),
                phase,
                text: Some(text),
                agent_id: Some(agent_id),
                session_id: None,
                project_root: None,
                data: Some(json!({ "status": status })),
            })
        }
        ServerEvent::QueueUpdated {
            project_root,
            session_id,
            agent_id,
            items,
        } => Some(UiSseMessage {
            id: format!("queue-{project_root}|{session_id}|{agent_id}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_QUEUE.to_string(),
            phase: None,
            text: Some(format!(
                "Queued {} message{}",
                items.len(),
                if items.len() == 1 { "" } else { "s" }
            )),
            agent_id: Some(agent_id),
            session_id: Some(session_id),
            project_root: Some(project_root),
            data: Some(json!({ "items": items })),
        }),
        ServerEvent::StateUpdated => Some(UiSseMessage {
            id: format!("run-sync-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_SYNC.to_string()),
            text: Some("State updated".to_string()),
            agent_id: None,
            session_id: None,
            project_root: None,
            data: None,
        }),
        ServerEvent::Outcome { agent_id, outcome } => Some(UiSseMessage {
            id: format!("run-outcome-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_OUTCOME.to_string()),
            text: Some("Run outcome".to_string()),
            agent_id: Some(agent_id),
            session_id: None,
            project_root: None,
            data: Some(json!({ "outcome": outcome })),
        }),
        ServerEvent::ContextUsage {
            agent_id,
            stage,
            message_count,
            char_count,
            estimated_tokens,
            token_limit,
            compressed,
            summary_count,
        } => Some(UiSseMessage {
            id: format!("run-context-{agent_id}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_CONTEXT_USAGE.to_string()),
            text: None,
            agent_id: Some(agent_id.clone()),
            session_id: None,
            project_root: None,
            data: Some(json!({
                "agent_id": agent_id,
                "stage": stage,
                "message_count": message_count,
                "char_count": char_count,
                "estimated_tokens": estimated_tokens,
                "token_limit": token_limit,
                "compressed": compressed,
                "summary_count": summary_count,
            })),
        }),
        ServerEvent::SubagentSpawned {
            parent_id,
            subagent_id,
            task,
        } => Some(UiSseMessage {
            id: format!("run-subagent-spawned-{subagent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_SUBAGENT_SPAWNED.to_string()),
            text: Some(format!("Spawned subagent {}", subagent_id)),
            agent_id: Some(parent_id),
            session_id: None,
            project_root: None,
            data: Some(json!({ "subagent_id": subagent_id, "task": task })),
        }),
        ServerEvent::SubagentResult {
            parent_id,
            subagent_id,
            outcome,
        } => Some(UiSseMessage {
            id: format!("run-subagent-result-{subagent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_SUBAGENT_RESULT.to_string()),
            text: Some(format!("Subagent {} returned", subagent_id)),
            agent_id: Some(parent_id),
            session_id: None,
            project_root: None,
            data: Some(json!({ "subagent_id": subagent_id, "outcome": outcome })),
        }),
        ServerEvent::Token {
            agent_id,
            token,
            done,
            thinking,
        } => Some(UiSseMessage {
            id: format!("token-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_TOKEN.to_string(),
            phase: if done { Some(UI_PHASE_DONE.to_string()) } else { None },
            text: Some(token),
            agent_id: Some(agent_id),
            session_id: None,
            project_root: None,
            data: if thinking { Some(json!({ "thinking": true })) } else { None },
        }),
        ServerEvent::ChangeReport {
            agent_id,
            files,
            truncated_count,
        } => Some(UiSseMessage {
            id: format!("run-change-report-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_CHANGE_REPORT.to_string()),
            text: Some("Change report".to_string()),
            agent_id: Some(agent_id),
            session_id: None,
            project_root: None,
            data: Some(json!({
                "files": files,
                "truncated_count": truncated_count,
            })),
        }),
        ServerEvent::PlanUpdate { agent_id, plan } => Some(UiSseMessage {
            id: format!("run-plan-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_PLAN_UPDATE.to_string()),
            text: Some("Plan updated".to_string()),
            agent_id: Some(agent_id),
            session_id: None,
            project_root: None,
            data: Some(json!({ "plan": plan })),
        }),
    }
}

impl ServerState {
    pub async fn send_agent_status(
        &self,
        agent_id: String,
        status: AgentStatusKind,
        detail: Option<String>,
    ) {
        let mut done_event: Option<ServerEvent> = None;
        let mut status_id: Option<String> = None;
        let mut lifecycle: Option<String> = None;

        {
            let mut active = self.active_statuses.lock().await;
            if status == AgentStatusKind::Idle {
                if let Some(prev) = active.remove(&agent_id) {
                    done_event = Some(ServerEvent::AgentStatus {
                        agent_id: agent_id.clone(),
                        status: prev.status.as_str().to_string(),
                        detail: prev.detail,
                        status_id: Some(prev.status_id),
                        lifecycle: Some(UI_PHASE_DONE.to_string()),
                    });
                }
            } else {
                if let Some(prev) = active.get(&agent_id).cloned() {
                    if prev.status != status {
                        done_event = Some(ServerEvent::AgentStatus {
                            agent_id: agent_id.clone(),
                            status: prev.status.as_str().to_string(),
                            detail: prev.detail,
                            status_id: Some(prev.status_id),
                            lifecycle: Some(UI_PHASE_DONE.to_string()),
                        });
                        active.remove(&agent_id);
                    } else {
                        status_id = Some(prev.status_id.clone());
                        lifecycle = Some(UI_PHASE_DOING.to_string());
                        active.insert(
                            agent_id.clone(),
                            ActiveStatusRecord {
                                status_id: prev.status_id,
                                status,
                                detail: detail.clone(),
                            },
                        );
                    }
                }

                if status_id.is_none() {
                    let next_id =
                        format!("status-{}", self.status_seq.fetch_add(1, Ordering::Relaxed));
                    status_id = Some(next_id.clone());
                    lifecycle = Some(UI_PHASE_DOING.to_string());
                    active.insert(
                        agent_id.clone(),
                        ActiveStatusRecord {
                            status_id: next_id,
                            status,
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
            status: status.as_str().to_string(),
            detail,
            status_id,
            lifecycle,
        });
    }
}

pub struct ServerHandle {
    pub state: Arc<ServerState>,
    pub task: tokio::task::JoinHandle<anyhow::Result<()>>,
    pub port: u16,
}

pub async fn prepare_server(
    manager: Arc<AgentManager>,
    skill_manager: Arc<crate::skills::SkillManager>,
    port: u16,
    dev_mode: bool,
    mut agent_events_rx: mpsc::UnboundedReceiver<crate::agent_manager::AgentEvent>,
) -> anyhow::Result<ServerHandle> {
    info!("linggen-agent server starting on port {}...", port);

    // SSE can be bursty (tool/status steps). Use a larger buffer to reduce lag drops.
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
                        let _ =
                            state_clone
                                .events_tx
                                .send(ServerEvent::Message { from, to, content });
                    }
                    crate::agent_manager::AgentEvent::AgentStatus {
                        agent_id,
                        status,
                        detail,
                    } => {
                        state_clone
                            .send_agent_status(agent_id, AgentStatusKind::from_str_loose(&status), detail)
                            .await;
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
                    crate::agent_manager::AgentEvent::PlanUpdate { agent_id, plan } => {
                        let _ = state_clone
                            .events_tx
                            .send(ServerEvent::PlanUpdate { agent_id, plan });
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
        .route("/api/agent-files", get(list_agent_files_api))
        .route("/api/agent-file", get(get_agent_file_api))
        .route("/api/agent-file", post(upsert_agent_file_api))
        .route("/api/agent-file", delete(delete_agent_file_api))
        .route("/api/agent-runs", get(list_agent_runs_api))
        .route("/api/agent-children", get(list_agent_children_api))
        .route("/api/agent-context", get(get_agent_context_api))
        .route("/api/models", get(list_models_api))
        .route("/api/config", get(get_config_api).post(update_config_api))
        .route("/api/skills", get(list_skills))
        .route("/api/marketplace/search", get(marketplace_search))
        .route("/api/marketplace/list", get(marketplace_list))
        .route("/api/marketplace/install", post(marketplace_install))
        .route("/api/marketplace/uninstall", delete(marketplace_uninstall))
        .route("/api/builtin-skills", get(builtin_skills_list))
        .route("/api/builtin-skills/install", post(builtin_skills_install))
        .route("/api/skill-files", get(list_skill_files_api))
        .route("/api/skill-file", get(get_skill_file_api))
        .route("/api/skill-file", post(upsert_skill_file_api))
        .route("/api/skill-file", delete(delete_skill_file_api))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions", post(create_session))
        .route("/api/sessions", patch(rename_session_api))
        .route("/api/sessions", delete(remove_session_api))
        .route("/api/task", post(set_task))
        .route("/api/run", post(run_agent))
        .route("/api/agent-cancel", post(cancel_agent_run))
        .route("/api/chat", post(chat_handler))
        .route("/api/chat/clear", post(clear_chat_history_api))
        .route("/api/plan/approve", post(approve_plan_handler))
        .route("/api/plan/reject", post(reject_plan_handler))
        .route("/api/workspace/tree", get(get_agent_tree))
        .route("/api/files", get(list_files))
        .route("/api/file", get(read_file_api))
        .route("/api/workspace/state", get(get_workspace_state))
        .route("/api/events", get(events_handler))
        .route("/api/health", get(health_handler))
        .route("/api/utils/pick-folder", get(pick_folder))
        .route("/api/utils/ollama-status", get(get_ollama_status))
        .route("/api/memory/{*rest}", get(memory_proxy).post(memory_proxy).delete(memory_proxy))
        .fallback(static_handler)
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    let actual_port = listener.local_addr()?.port();
    info!("Server running on http://localhost:{}", actual_port);

    let task = tokio::spawn(async move {
        axum::serve(listener, app).await?;
        Ok(())
    });

    Ok(ServerHandle {
        state,
        task,
        port: actual_port,
    })
}

pub async fn start_server(
    manager: Arc<AgentManager>,
    skill_manager: Arc<crate::skills::SkillManager>,
    port: u16,
    dev_mode: bool,
    agent_events_rx: mpsc::UnboundedReceiver<crate::agent_manager::AgentEvent>,
) -> anyhow::Result<()> {
    let handle = prepare_server(manager, skill_manager, port, dev_mode, agent_events_rx).await?;
    handle.task.await??;
    Ok(())
}

async fn static_handler(State(state): State<Arc<ServerState>>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    if state.dev_mode {
        // In dev mode, static assets are served by the Vite dev server.
        // Return 404 so the user knows to use the Vite proxy.
        return Response::builder()
            .status(404)
            .header("Content-Type", "text/plain")
            .body(axum::body::Body::from(
                "Dev mode: static assets are served by Vite. Use the Vite dev server URL instead.",
            ))
            .unwrap();
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
            match Assets::get("index.html") {
                Some(index) => Response::builder()
                    .header("Content-Type", "text/html")
                    .body(axum::body::Body::from(index.data))
                    .unwrap(),
                None => Response::builder()
                    .status(404)
                    .body(axum::body::Body::from("Not found"))
                    .unwrap(),
            }
        }
    }
}

async fn events_handler(
    State(state): State<Arc<ServerState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |msg| {
        let event = match msg {
            Ok(event) => event,
            Err(_) => ServerEvent::StateUpdated,
        };
        let seq = state.event_seq.fetch_add(1, Ordering::Relaxed);
        let ui_msg = map_server_event_to_ui_message(event, seq)?;
        let data = serde_json::to_string(&ui_msg).unwrap_or_default();
        Some(Ok(Event::default().data(data)))
    });

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn health_handler() -> impl IntoResponse {
    axum::Json(json!({ "ok": true }))
}

/// Proxy /api/memory/* requests to the ling-mem server.
async fn memory_proxy(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path(rest): axum::extract::Path<String>,
    method: axum::http::Method,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let config = state.manager.get_config_snapshot().await;
    let base = config.memory.server_url.trim_end_matches('/');
    let target_url = format!("{}/api/{}", base, rest);

    let client = reqwest::Client::new();
    let mut req = client.request(method.clone(), &target_url);

    // Forward content-type header
    if let Some(ct) = headers.get("content-type") {
        req = req.header("content-type", ct);
    }

    if !body.is_empty() {
        req = req.body(body.to_vec());
    }

    match req.send().await {
        Ok(resp) => {
            let status = axum::http::StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(axum::http::StatusCode::BAD_GATEWAY);
            let ct = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_string();
            let bytes = resp.bytes().await.unwrap_or_default();
            Response::builder()
                .status(status)
                .header("content-type", ct)
                .body(axum::body::Body::from(bytes))
                .unwrap()
        }
        Err(e) => Response::builder()
            .status(axum::http::StatusCode::BAD_GATEWAY)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::to_vec(&json!({ "error": format!("Memory server unreachable: {}", e) }))
                    .unwrap_or_default(),
            ))
            .unwrap(),
    }
}

async fn pick_folder() -> impl IntoResponse {
    (axum::http::StatusCode::NOT_IMPLEMENTED, "Folder picker not implemented").into_response()
}

async fn get_ollama_status(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let models_guard = state.manager.models.read().await;
    if let Some(client) = models_guard.first_ollama_client() {
        match client.get_ps().await {
            Ok(status) => axum::Json(status).into_response(),
            Err(e) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    } else {
        (
            axum::http::StatusCode::NOT_FOUND,
            "No Ollama models configured",
        )
            .into_response()
    }
}
