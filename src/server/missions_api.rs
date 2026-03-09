use crate::project_store::missions::{self, MISSION_AGENT_ID};
use crate::server::{ServerEvent, ServerState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub(crate) struct OptionalProjectQuery {
    #[serde(default)]
    project_root: Option<String>,
}

/// GET /api/missions — list all global missions (optionally filtered by project).
pub(crate) async fn list_missions(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<OptionalProjectQuery>,
) -> impl IntoResponse {
    match state.manager.missions.list_all_missions() {
        Ok(missions) => {
            let filtered = if let Some(ref pr) = q.project_root {
                if pr == "__all__" {
                    missions
                } else {
                    missions
                        .into_iter()
                        .filter(|m| m.project.as_deref() == Some(pr.as_str()) || m.project.is_none())
                        .collect()
                }
            } else {
                missions
            };
            Json(serde_json::json!({ "missions": filtered })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list missions: {}", e),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct CreateMissionRequest {
    #[serde(default)]
    name: Option<String>,
    schedule: String,
    prompt: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    project: Option<String>,
    // Legacy field — accept but ignore
    #[serde(default)]
    #[allow(dead_code)]
    project_root: Option<String>,
}

/// POST /api/missions
pub(crate) async fn create_mission(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateMissionRequest>,
) -> impl IntoResponse {
    if let Err(e) = missions::validate_cron(&req.schedule) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }

    let project = req.project.or(req.project_root);

    match state.manager.missions.create_mission(
        req.name,
        &req.schedule,
        &req.prompt,
        req.model,
        project,
    ) {
        Ok(mission) => {
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(mission).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create mission: {}", e),
        )
            .into_response(),
    }
}

/// GET /api/missions/:id
pub(crate) async fn get_mission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.manager.missions.get_mission(&id) {
        Ok(Some(mission)) => Json(mission).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get mission: {}", e),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct UpdateMissionRequest {
    #[serde(default)]
    name: Option<Option<String>>,
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    model: Option<Option<String>>,
    #[serde(default)]
    project: Option<Option<String>>,
    #[serde(default)]
    enabled: Option<bool>,
    // Legacy field — accept but ignore
    #[serde(default)]
    #[allow(dead_code)]
    project_root: Option<String>,
}

/// PUT /api/missions/:id
pub(crate) async fn update_mission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMissionRequest>,
) -> impl IntoResponse {
    if let Some(ref s) = req.schedule {
        if let Err(e) = missions::validate_cron(s) {
            return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
        }
    }

    match state.manager.missions.update_mission(
        &id,
        req.name,
        req.schedule.as_deref(),
        req.prompt.as_deref(),
        req.model,
        req.project,
        req.enabled,
    ) {
        Ok(mission) => {
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(mission).into_response()
        }
        Err(e) => {
            let status = if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, format!("Failed to update mission: {}", e)).into_response()
        }
    }
}

/// DELETE /api/missions/:id
pub(crate) async fn delete_mission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.manager.missions.delete_mission(&id) {
        Ok(()) => {
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete mission: {}", e),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct TriggerMissionRequest {
    #[serde(default)]
    project_root: Option<String>,
}

/// POST /api/missions/:id/trigger — run a mission immediately
pub(crate) async fn trigger_mission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<TriggerMissionRequest>,
) -> impl IntoResponse {
    let mission = match state.manager.missions.get_mission(&id) {
        Ok(Some(m)) => m,
        Ok(None) => return (StatusCode::NOT_FOUND, "Mission not found".to_string()).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    // Determine project root: from request, mission, or current dir
    let project_path = req
        .project_root
        .or_else(|| mission.project.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().to_string_lossy().to_string());

    let root = std::path::PathBuf::from(&project_path);

    // Check if mission agent is busy
    let agent = match state.manager.get_or_create_agent(&root, MISSION_AGENT_ID).await {
        Ok(a) => a,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get agent: {}", e)).into_response(),
    };
    if agent.try_lock().is_err() {
        return (StatusCode::CONFLICT, "Mission agent is busy".to_string()).into_response();
    }

    let _ = state.events_tx.send(ServerEvent::MissionTriggered {
        mission_id: mission.id.clone(),
        agent_id: MISSION_AGENT_ID.to_string(),
        project_root: project_path.clone(),
    });

    state
        .manager
        .update_agent_activity(&project_path, MISSION_AGENT_ID)
        .await;

    let state_clone = state.clone();

    tokio::spawn(async move {
        crate::server::mission_scheduler::dispatch_mission_prompt_public(
            state_clone,
            root,
            &project_path,
            &mission,
        )
        .await;
    });

    Json(serde_json::json!({ "ok": true, "message": "Mission triggered" })).into_response()
}

/// GET /api/missions/:id/runs
pub(crate) async fn list_mission_runs(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.manager.missions.list_mission_runs(&id) {
        Ok(runs) => Json(serde_json::json!({ "runs": runs })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list mission runs: {}", e),
        )
            .into_response(),
    }
}
