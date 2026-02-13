use crate::config::{AgentKind, AgentSpec};
use crate::server::chat_helpers::sanitize_message_for_ui;
use crate::server::{ServerEvent, ServerState};
use crate::skills::Skill;
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Deserialize)]
pub(crate) struct ProjectQuery {
    project_root: String,
}

#[derive(Deserialize)]
pub(crate) struct AgentsQuery {
    project_root: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct AgentFileQuery {
    project_root: String,
    path: String,
}

#[derive(Deserialize)]
pub(crate) struct UpsertAgentFileRequest {
    project_root: String,
    path: String,
    content: String,
}

#[derive(Deserialize)]
pub(crate) struct DeleteAgentFileRequest {
    project_root: String,
    path: String,
}

#[derive(Serialize)]
struct AgentFileListItem {
    agent_id: String,
    name: String,
    description: String,
    kind: String,
    path: String,
}

#[derive(Serialize)]
struct AgentFileResponse {
    path: String,
    content: String,
    valid: bool,
    error: Option<String>,
}

#[derive(Serialize)]
struct AgentFileWriteResponse {
    path: String,
    agent_id: String,
}

fn kind_label(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Main => "main",
        AgentKind::Subagent => "subagent",
    }
}

fn canonical_project_root(project_root: &str) -> PathBuf {
    PathBuf::from(project_root)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(project_root))
}

fn normalize_agent_md_path(path: &str) -> Result<String, String> {
    let raw = path.trim().replace('\\', "/");
    if raw.is_empty() {
        return Err("path is required".to_string());
    }
    if raw.starts_with('/') || raw.contains("..") {
        return Err("path must be a relative markdown path under agents/".to_string());
    }
    let rel = if raw.starts_with("agents/") {
        raw
    } else {
        format!("agents/{}", raw)
    };
    if !rel.to_ascii_lowercase().ends_with(".md") {
        return Err("agent files must end with .md".to_string());
    }
    if !rel
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '-' || c == '_' || c == '.')
    {
        return Err("path contains unsupported characters".to_string());
    }
    let suffix = rel.strip_prefix("agents/").unwrap_or("");
    if suffix.is_empty() || suffix.split('/').any(|seg| seg.is_empty()) {
        return Err("invalid agent markdown path".to_string());
    }
    Ok(rel)
}

pub(crate) async fn list_projects(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    match state.manager.db.list_projects() {
        Ok(projects) => Json(projects).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn list_agents_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<AgentsQuery>,
) -> impl IntoResponse {
    let root = query
        .project_root
        .as_deref()
        .map(canonical_project_root)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    match state.manager.list_agents(&root).await {
        Ok(agents) => Json(agents).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn list_agent_files_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    let root = canonical_project_root(&query.project_root);
    match state.manager.list_agent_specs(&root).await {
        Ok(entries) => {
            let items: Vec<AgentFileListItem> = entries
                .into_iter()
                .map(|entry| {
                    let rel = entry
                        .spec_path
                        .strip_prefix(&root)
                        .unwrap_or(entry.spec_path.as_path())
                        .to_string_lossy()
                        .to_string();
                    AgentFileListItem {
                        agent_id: entry.agent_id,
                        name: entry.spec.name,
                        description: entry.spec.description,
                        kind: kind_label(entry.spec.kind).to_string(),
                        path: rel,
                    }
                })
                .collect();
            Json(items).into_response()
        }
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

pub(crate) async fn get_agent_file_api(Query(query): Query<AgentFileQuery>) -> impl IntoResponse {
    let root = canonical_project_root(&query.project_root);
    let rel = match normalize_agent_md_path(&query.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    let full_path = root.join(&rel);
    let content = match std::fs::read_to_string(&full_path) {
        Ok(content) => content,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let parsed = AgentSpec::from_markdown_content(&content);
    Json(AgentFileResponse {
        path: rel,
        content,
        valid: parsed.is_ok(),
        error: parsed.err().map(|e| e.to_string()),
    })
    .into_response()
}

pub(crate) async fn upsert_agent_file_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<UpsertAgentFileRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    let rel = match normalize_agent_md_path(&req.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    let (spec, _) = match AgentSpec::from_markdown_content(&req.content) {
        Ok(parsed) => parsed,
        Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    };
    let full_path = root.join(&rel);
    if let Some(parent) = full_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
        }
    }
    if let Err(err) = std::fs::write(&full_path, &req.content) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    if let Err(err) = state.manager.invalidate_agent_cache(&root, None).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    let _ = state.events_tx.send(ServerEvent::StateUpdated);
    Json(AgentFileWriteResponse {
        path: rel,
        agent_id: spec.name.trim().to_lowercase(),
    })
    .into_response()
}

pub(crate) async fn delete_agent_file_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<DeleteAgentFileRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    let rel = match normalize_agent_md_path(&req.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    let full_path = root.join(&rel);
    if !full_path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(err) = std::fs::remove_file(&full_path) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    if let Err(err) = state.manager.invalidate_agent_cache(&root, None).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    let _ = state.events_tx.send(ServerEvent::StateUpdated);
    StatusCode::OK.into_response()
}

#[derive(Deserialize)]
pub(crate) struct AgentRunsQuery {
    project_root: String,
    session_id: Option<String>,
}

pub(crate) async fn list_agent_runs_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<AgentRunsQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&query.project_root);
    match state
        .manager
        .list_agent_runs(&root, query.session_id.as_deref())
        .await
    {
        Ok(runs) => Json(runs).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct AgentChildrenQuery {
    run_id: String,
}

pub(crate) async fn list_agent_children_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<AgentChildrenQuery>,
) -> impl IntoResponse {
    match state.manager.list_agent_children(&query.run_id).await {
        Ok(runs) => Json(runs).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct AgentContextQuery {
    run_id: String,
    view: Option<String>, // "summary" | "raw"
}

#[derive(Serialize)]
struct AgentContextSummary {
    message_count: usize,
    user_messages: usize,
    agent_messages: usize,
    system_messages: usize,
    started_at: u64,
    ended_at: Option<u64>,
}

#[derive(Serialize)]
struct AgentContextResponse {
    run: crate::db::AgentRunRecord,
    summary: AgentContextSummary,
    messages: Option<Vec<crate::db::ChatMessageRecord>>,
}

pub(crate) async fn get_agent_context_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<AgentContextQuery>,
) -> impl IntoResponse {
    let run = match state.manager.get_agent_run(&query.run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let all_messages = match state.manager.db.get_chat_history(
        &run.repo_path,
        &run.session_id,
        Some(&run.agent_id),
    ) {
        Ok(messages) => messages,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let end_ts = run.ended_at.unwrap_or(u64::MAX);
    let messages: Vec<crate::db::ChatMessageRecord> = all_messages
        .into_iter()
        .filter(|m| m.timestamp >= run.started_at && m.timestamp <= end_ts)
        .collect();

    let user_messages = messages.iter().filter(|m| m.from_id == "user").count();
    let system_messages = messages.iter().filter(|m| m.from_id == "system").count();
    let agent_messages = messages
        .len()
        .saturating_sub(user_messages)
        .saturating_sub(system_messages);
    let summary = AgentContextSummary {
        message_count: messages.len(),
        user_messages,
        agent_messages,
        system_messages,
        started_at: run.started_at,
        ended_at: run.ended_at,
    };

    let is_raw = query
        .view
        .as_deref()
        .map(|v| v.eq_ignore_ascii_case("raw"))
        .unwrap_or(false);

    let ui_messages: Vec<crate::db::ChatMessageRecord> = messages
        .iter()
        .cloned()
        .filter_map(|mut m| {
            let cleaned = sanitize_message_for_ui(&m.from_id, &m.content)?;
            m.content = cleaned;
            Some(m)
        })
        .collect();

    Json(AgentContextResponse {
        run,
        summary,
        messages: if is_raw { Some(ui_messages) } else { None },
    })
    .into_response()
}

pub(crate) async fn list_models_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let models = state.manager.models.list_models();
    Json(models).into_response()
}

pub(crate) async fn list_skills(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let skills: Vec<Skill> = state.skill_manager.list_skills().await;
    Json(skills).into_response()
}

pub(crate) async fn list_sessions(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    match state.manager.db.list_sessions(&query.project_root) {
        Ok(sessions) => Json(sessions).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct CreateSessionRequest {
    project_root: String,
    title: String,
}

pub(crate) async fn create_session(
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
pub(crate) struct RemoveSessionRequest {
    project_root: String,
    session_id: String,
}

pub(crate) async fn remove_session_api(
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
pub(crate) struct AddProjectRequest {
    path: String,
}

pub(crate) async fn add_project(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<AddProjectRequest>,
) -> impl IntoResponse {
    let path = PathBuf::from(&req.path);
    match state.manager.get_or_create_project(path).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub(crate) async fn remove_project(
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
