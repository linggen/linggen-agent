use crate::server::ServerState;
use crate::server::chat_helpers::sanitize_message_for_ui;
use crate::skills::Skill;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Deserialize)]
pub(crate) struct ProjectQuery {
    project_root: String,
}

pub(crate) async fn list_projects(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    match state.manager.db.list_projects() {
        Ok(projects) => Json(projects).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn list_agents_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    match state.manager.list_agents().await {
        Ok(agents) => Json(agents).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
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

    let all_messages = match state
        .manager
        .db
        .get_chat_history(&run.repo_path, &run.session_id, Some(&run.agent_id))
    {
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
