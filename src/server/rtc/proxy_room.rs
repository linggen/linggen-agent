//! Proxy room consumer — connect to a room owner's linggen as a model provider.
//!
//! When this linggen instance is a "linggen" consumer in a proxy room,
//! this module establishes a WebRTC connection to the owner's linggen,
//! discovers available models, and registers them as proxy providers.

use std::sync::Arc;
use tracing::{info, warn};

use crate::cli::login::load_remote_config;
use crate::server::ServerState;

/// Join a proxy room and register the owner's models as proxy providers.
///
/// This:
/// 1. Connects to the owner's linggen via WebRTC relay
/// 2. Lists available models via `list_models` DC message
/// 3. Registers them in the ModelManager with "proxy:" prefix
///
/// The proxy models appear alongside local models and can be used by
/// the agent engine transparently.
pub async fn connect_proxy_room(
    state: Arc<ServerState>,
    instance_id: &str,
    owner_name: Option<String>,
) -> anyhow::Result<()> {
    let config = load_remote_config()
        .ok_or_else(|| anyhow::anyhow!("Not logged in to linggen.dev. Run `ling login` first."))?;

    info!("Connecting to proxy room (instance: {instance_id})");

    // Establish WebRTC connection to the room owner
    let conn = super::proxy_client::connect_to_room(
        &config.relay_url,
        instance_id,
        &config.api_token,
    ).await?;

    info!("WebRTC connection established to proxy room");

    // Create the proxy model client with request demuxing
    let proxy_client = Arc::new(
        crate::agent_manager::proxy_provider::ProxyModelClient::new(
            conn.request_tx,
            conn.response_rx,
        )
    );

    // Discover models available on the owner's linggen
    let models = proxy_client.list_models().await?;
    info!("Discovered {} models from proxy room", models.len());

    if models.is_empty() {
        warn!("No models available from proxy room");
        return Ok(());
    }

    // Register proxy models: swap out the Arc<ModelManager> with a new one
    // that includes the proxy models.
    let mut model_lock = state.manager.models.write().await;
    let old_mm = Arc::clone(&model_lock);

    // Build a new ModelManager from the current config + proxy models
    let configs: Vec<_> = old_mm.list_models().iter().map(|c| (*c).clone()).collect();
    let mut new_mm = crate::agent_manager::models::ModelManager::new(configs);
    new_mm.register_proxy_models(proxy_client, models, owner_name);

    *model_lock = Arc::new(new_mm);
    drop(model_lock);

    info!("Proxy room models registered successfully");
    Ok(())
}

/// Disconnect proxy room models — remove all "proxy:" models from ModelManager.
pub async fn disconnect_proxy_room(state: Arc<ServerState>) {
    let mut model_lock = state.manager.models.write().await;
    let old_mm = Arc::clone(&model_lock);

    // Rebuild ModelManager with only non-proxy models
    let configs: Vec<_> = old_mm.list_models().iter()
        .filter(|c| c.provider != "proxy")
        .map(|c| (*c).clone())
        .collect();
    let new_mm = crate::agent_manager::models::ModelManager::new(configs);
    *model_lock = Arc::new(new_mm);

    info!("Proxy room models removed");
}
