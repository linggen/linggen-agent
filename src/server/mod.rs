use crate::agent_manager::AgentManager;
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
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
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
) -> anyhow::Result<()> {
    setup_tracing();
    info!("linggen-agent server starting on port {}...", port);

    let (events_tx, _) = broadcast::channel(100);

    let state = Arc::new(ServerState {
        manager,
        dev_mode,
        events_tx,
        skill_manager,
    });

    let app = Router::new()
        .route("/api/projects", get(list_projects))
        .route("/api/projects", post(add_project))
        .route("/api/projects", delete(remove_project))
        .route("/api/agents", get(list_agents_api))
        .route("/api/skills", get(list_skills))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions", post(create_session))
        .route("/api/sessions", delete(remove_session_api))
        .route("/api/task", post(set_task))
        .route("/api/run", post(run_agent))
        .route("/api/chat", post(chat_handler))
        .route("/api/workspace/tree", get(get_agent_tree))
        .route("/api/files", get(list_files))
        .route("/api/file", get(read_file_api))
        .route("/api/lead/state", get(get_lead_state))
        .route("/api/events", get(events_handler))
        .route("/api/utils/pick-folder", get(pick_folder))
        .fallback(static_handler)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!("Server running on http://localhost:{}", port);
    axum::serve(listener, app).await?;

    Ok(())
}

fn setup_tracing() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .compact()
        .init();
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

                let _ = events_tx.send(ServerEvent::Outcome { agent_id, outcome });
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

async fn chat_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let session_id = req.session_id.clone();
    let events_tx = state.events_tx.clone();
    let manager_clone = state.manager.clone();

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

    match state.manager.get_or_create_agent(&root, &target_id).await {
        Ok(agent) => {
            tokio::spawn(async move {
                let mut engine = agent.lock().await;
                let response = engine
                    .chat(&clean_msg, session_id.as_deref())
                    .await
                    .unwrap_or_else(|e| format!("Error: {}", e));

                // Log message if it's Lead or Coder
                if let Ok(ctx) = manager_clone.get_or_create_project(root).await {
                    let _ = ctx.state_fs.append_message(
                        "user",
                        &target_id,
                        &clean_msg,
                        None,
                        session_id.as_deref(),
                    );
                    let _ = ctx.state_fs.append_message(
                        &target_id,
                        "user",
                        &response,
                        None,
                        session_id.as_deref(),
                    );
                }

                let _ = events_tx.send(ServerEvent::Message {
                    from: "user".to_string(),
                    to: target_id.clone(),
                    content: clean_msg,
                });
                let _ = events_tx.send(ServerEvent::Message {
                    from: target_id,
                    to: "user".to_string(),
                    content: response,
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
        Err(_) => Ok(Event::default().data("error")),
    });

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn pick_folder() -> impl IntoResponse {
    // On macOS, we can use an AppleScript to open a folder picker.
    // This is a bit of a hack but works well for a local-only tool.
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg("choose folder with prompt \"Select a repository folder:\"")
            .output();

        if let Ok(output) = output {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                // osascript returns "alias Macintosh HD:Users:path:to:folder:"
                // or "folder Macintosh HD:Users:path:to:folder:"
                // We need to convert this to a POSIX path.
                let posix_output = std::process::Command::new("osascript")
                    .arg("-e")
                    .arg(format!(
                        "POSIX path of alias \"{}\"",
                        path_str.replace("alias ", "").replace("folder ", "")
                    ))
                    .output();

                if let Ok(posix_output) = posix_output {
                    let posix_path = String::from_utf8_lossy(&posix_output.stdout)
                        .trim()
                        .to_string();
                    return Json(serde_json::json!({ "path": posix_path })).into_response();
                }
            }
        }
    }

    // Fallback or other OS (placeholder for now)
    StatusCode::NOT_IMPLEMENTED.into_response()
}
