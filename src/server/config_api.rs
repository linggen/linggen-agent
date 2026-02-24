use crate::config::Config;
use crate::credentials::{self, Credentials};
use crate::server::ServerState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

pub(crate) async fn get_config_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let mut config = state.manager.get_config_snapshot().await;
    // Strip API keys from config response — credentials are served separately.
    for model in &mut config.models {
        model.api_key = None;
    }
    Json(config).into_response()
}

pub(crate) async fn update_config_api(
    State(state): State<Arc<ServerState>>,
    Json(mut new_config): Json<Config>,
) -> impl IntoResponse {
    // Strip API keys from config — they should be saved via /api/credentials instead.
    // For backward compat: if the UI sends a non-redacted, non-empty key, migrate it
    // to credentials.json.
    let creds_file = credentials::credentials_file();
    let mut creds = Credentials::load(&creds_file);
    let mut creds_changed = false;

    for model in &mut new_config.models {
        if let Some(ref key) = model.api_key {
            if key != "***" && !key.is_empty() {
                // Migrate to credentials.json
                creds.set_api_key(&model.id, Some(key.clone()));
                creds_changed = true;
            }
        }
        // Always strip from TOML
        model.api_key = None;
    }

    if creds_changed {
        if let Err(e) = creds.save(&creds_file) {
            tracing::warn!("Failed to save credentials.json: {}", e);
        }
    }

    if let Err(e) = new_config.validate() {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    match state.manager.apply_config(new_config).await {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Model health API — in-memory health status for all configured models
// ---------------------------------------------------------------------------

pub(crate) async fn get_models_health(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    use crate::agent_manager::models::ModelHealthStatus;

    let models_guard = state.manager.models.read().await;
    let health_records = models_guard.health.get_all().await;

    // Build a map of model_id → health record for easy lookup
    let health_map: std::collections::HashMap<String, _> =
        health_records.into_iter().collect();

    // Return health for all configured models
    let config = state.manager.get_config_snapshot().await;
    let result: Vec<serde_json::Value> = config
        .models
        .iter()
        .map(|m| {
            if let Some(rec) = health_map.get(&m.id) {
                serde_json::json!({
                    "id": m.id,
                    "health": rec.status,
                    "last_error": rec.last_error,
                    "since_secs": rec.since_secs,
                })
            } else {
                serde_json::json!({
                    "id": m.id,
                    "health": ModelHealthStatus::Healthy,
                    "last_error": null,
                    "since_secs": null,
                })
            }
        })
        .collect();

    Json(result).into_response()
}

// ---------------------------------------------------------------------------
// Credentials API — reads/writes ~/.linggen/credentials.json
// ---------------------------------------------------------------------------

pub(crate) async fn get_credentials_api() -> impl IntoResponse {
    let creds = Credentials::load(&credentials::credentials_file());
    Json(creds.redacted()).into_response()
}

#[derive(serde::Deserialize)]
pub(crate) struct UpdateCredentialsRequest {
    /// Model ID → API key. Send null/empty to remove.
    #[serde(flatten)]
    entries: std::collections::HashMap<String, serde_json::Value>,
}

pub(crate) async fn update_credentials_api(
    Json(body): Json<UpdateCredentialsRequest>,
) -> impl IntoResponse {
    let creds_file = credentials::credentials_file();
    let mut creds = Credentials::load(&creds_file);

    for (model_id, value) in &body.entries {
        match value {
            serde_json::Value::Object(obj) => {
                let api_key = obj
                    .get("api_key")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                // Skip if value is the redacted placeholder
                if api_key.as_deref() == Some("***") {
                    continue;
                }
                creds.set_api_key(model_id, api_key);
            }
            serde_json::Value::Null => {
                creds.set_api_key(model_id, None);
            }
            _ => {}
        }
    }

    match creds.save(&creds_file) {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
