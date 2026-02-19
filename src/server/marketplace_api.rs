use crate::server::{ServerEvent, ServerState};
use crate::skills::marketplace::{self, SkillScope};
use crate::skills;
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Query / request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct SearchQuery {
    q: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct InstallRequest {
    name: String,
    repo_url: Option<String>,
    git_ref: Option<String>,
    scope: Option<SkillScope>,
    project_root: Option<String>,
    #[serde(default)]
    force: bool,
}

#[derive(Deserialize)]
pub(crate) struct UninstallRequest {
    name: String,
    scope: Option<SkillScope>,
    project_root: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub(crate) async fn marketplace_search(
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    let q = query.q.unwrap_or_default();
    if q.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing query parameter 'q'").into_response();
    }

    match marketplace::search_marketplace(&q).await {
        Ok(skills) => axum::Json(skills).into_response(),
        Err(e) => {
            tracing::error!(err = %e, "Marketplace search failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub(crate) async fn marketplace_list(
    Query(query): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(20);

    match marketplace::list_marketplace(limit).await {
        Ok(skills) => axum::Json(skills).into_response(),
        Err(e) => {
            tracing::error!(err = %e, "Marketplace list failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub(crate) async fn marketplace_install(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<InstallRequest>,
) -> impl IntoResponse {
    let scope = req.scope.unwrap_or_default();
    let project_root_path = req.project_root.as_deref().map(Path::new);

    let target_dir = match marketplace::skill_target_dir(&req.name, scope, project_root_path) {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    match marketplace::install_skill(
        &req.name,
        req.repo_url.as_deref(),
        req.git_ref.as_deref(),
        &target_dir,
        req.force,
    )
    .await
    {
        Ok(msg) => {
            let _ = state.skill_manager.load_all(project_root_path).await;
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            axum::Json(serde_json::json!({ "ok": true, "message": msg })).into_response()
        }
        Err(e) => {
            tracing::error!(err = %e, skill = %req.name, "Marketplace install failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub(crate) async fn marketplace_uninstall(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<UninstallRequest>,
) -> impl IntoResponse {
    let scope = req.scope.unwrap_or_default();
    let project_root_path = req.project_root.as_deref().map(Path::new);

    let target_dir = match marketplace::skill_target_dir(&req.name, scope, project_root_path) {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    match marketplace::delete_skill(&req.name, &target_dir) {
        Ok(msg) => {
            let _ = state.skill_manager.load_all(project_root_path).await;
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            axum::Json(serde_json::json!({ "ok": true, "message": msg })).into_response()
        }
        Err(e) => {
            tracing::error!(err = %e, skill = %req.name, "Marketplace uninstall failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Built-in skills
// ---------------------------------------------------------------------------

pub(crate) async fn builtin_skills_list() -> impl IntoResponse {
    axum::Json(skills::list_builtin_skills())
}

#[derive(Deserialize)]
pub(crate) struct BuiltInInstallRequest {
    name: String,
}

pub(crate) async fn builtin_skills_install(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<BuiltInInstallRequest>,
) -> impl IntoResponse {
    let target_dir = match marketplace::skill_target_dir(&req.name, SkillScope::Global, None) {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    // Install from linggen/skills repo
    match marketplace::install_skill(
        &req.name,
        Some("https://github.com/linggen/skills"),
        Some("main"),
        &target_dir,
        true, // force overwrite to get latest version
    )
    .await
    {
        Ok(msg) => {
            let _ = state.skill_manager.load_all(None).await;
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            axum::Json(serde_json::json!({ "ok": true, "message": msg })).into_response()
        }
        Err(e) => {
            tracing::error!(err = %e, skill = %req.name, "Built-in skill install failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
