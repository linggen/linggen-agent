use crate::config::Config;
use crate::server::ServerState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

pub(crate) async fn get_config_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let mut config = state.manager.get_config_snapshot().await;
    // Redact API keys before sending to the client.
    for model in &mut config.models {
        if model.api_key.is_some() {
            model.api_key = Some("***".to_string());
        }
    }
    Json(config).into_response()
}

pub(crate) async fn update_config_api(
    State(state): State<Arc<ServerState>>,
    Json(mut new_config): Json<Config>,
) -> impl IntoResponse {
    // Preserve existing API keys when the client sends back the redacted placeholder.
    let current = state.manager.get_config_snapshot().await;
    for model in &mut new_config.models {
        if model.api_key.as_deref() == Some("***") {
            // Find the matching model in the current config and keep its real key.
            let existing_key = current
                .models
                .iter()
                .find(|m| m.id == model.id)
                .and_then(|m| m.api_key.clone());
            model.api_key = existing_key;
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
