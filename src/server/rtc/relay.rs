//! Remote relay client — heartbeat + offer polling for linggen.dev signaling.
//!
//! When `~/.linggen/remote.toml` exists, the server:
//! 1. Sends heartbeats every 5 minutes to keep the instance "online" on the dashboard.
//! 2. Polls for incoming SDP offers from remote browser clients.
//! 3. Feeds offers into `create_peer()` and posts answers back.

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn, debug};

use crate::cli::login::{load_remote_config, RemoteConfig};
use crate::server::ServerState;

/// Maximum concurrent remote peer connections.
const MAX_REMOTE_PEERS: usize = 4;

fn build_relay_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Start relay background tasks if remote config exists.
/// Call this after the server is ready to accept peer connections.
pub fn spawn_relay_tasks(state: Arc<ServerState>) {
    let Some(config) = load_remote_config() else {
        debug!("No remote.toml found — relay tasks not started");
        return;
    };

    info!(
        "Remote access enabled: {} ({})",
        config.instance_name, config.instance_id
    );

    let peer_semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_REMOTE_PEERS));

    // Spawn heartbeat loop
    let hb_config = config.clone();
    tokio::spawn(async move {
        heartbeat_loop(&hb_config).await;
    });

    // Spawn offer polling loop
    let poll_config = config.clone();
    tokio::spawn(async move {
        offer_poll_loop(&poll_config, state, peer_semaphore).await;
    });
}

/// Send heartbeats every 5 minutes to keep the instance online.
async fn heartbeat_loop(config: &RemoteConfig) {
    let client = build_relay_client();
    let url = format!(
        "{}/api/instances/{}/heartbeat",
        config.relay_url, config.instance_id
    );

    loop {
        match client
            .post(&url)
            .bearer_auth(&config.api_token)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                debug!("Heartbeat sent");
            }
            Ok(resp) => {
                warn!("Heartbeat failed: {}", resp.status());
            }
            Err(e) => {
                warn!("Heartbeat error: {}", e);
            }
        }

        tokio::time::sleep(Duration::from_secs(300)).await; // 5 minutes
    }
}

/// Poll for incoming SDP offers and create peer connections.
async fn offer_poll_loop(config: &RemoteConfig, state: Arc<ServerState>, peer_sem: Arc<tokio::sync::Semaphore>) {
    let client = build_relay_client();
    let url = format!(
        "{}/api/signaling/{}/offer",
        config.relay_url, config.instance_id
    );
    let mut error_backoff = Duration::from_secs(5);

    loop {
        match client
            .get(&url)
            .bearer_auth(&config.api_token)
            .send()
            .await
        {
            Ok(resp) if resp.status() == 204 => {
                error_backoff = Duration::from_secs(5); // reset on success
            }
            Ok(resp) if resp.status().is_success() => {
                error_backoff = Duration::from_secs(5); // reset on success
                match resp.json::<serde_json::Value>().await {
                    Ok(data) => {
                        let nonce = data["nonce"].as_str().unwrap_or("").to_string();
                        let sdp = data["sdp"].as_str().unwrap_or("").to_string();

                        if !nonce.is_empty() && !sdp.is_empty() {
                            info!("Received remote offer (nonce: {nonce})");
                            // Acquire semaphore permit to cap concurrent peers.
                            let permit = match peer_sem.clone().try_acquire_owned() {
                                Ok(p) => p,
                                Err(_) => {
                                    warn!("Max remote peers ({MAX_REMOTE_PEERS}) reached, dropping offer");
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                    continue;
                                }
                            };
                            let cfg = config.clone();
                            let st = state.clone();
                            let cl = client.clone();
                            tokio::spawn(async move {
                                handle_remote_offer(&cfg, &st, &cl, &nonce, &sdp).await;
                                drop(permit); // release on peer task exit
                            });
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse offer response: {}", e);
                    }
                }
            }
            Ok(resp) if resp.status() == 401 || resp.status() == 403 => {
                warn!("Relay auth rejected ({}). Re-run `ling login` to fix. Stopping relay.", resp.status());
                return;
            }
            Ok(resp) => {
                warn!("Offer poll failed: {} (backoff {:?})", resp.status(), error_backoff);
                tokio::time::sleep(error_backoff).await;
                error_backoff = (error_backoff * 2).min(Duration::from_secs(120));
            }
            Err(e) => {
                warn!("Offer poll error: {} (backoff {:?})", e, error_backoff);
                tokio::time::sleep(error_backoff).await;
                error_backoff = (error_backoff * 2).min(Duration::from_secs(120));
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Process a remote SDP offer: create peer connection and post answer back.
async fn handle_remote_offer(
    config: &RemoteConfig,
    state: &Arc<ServerState>,
    client: &reqwest::Client,
    nonce: &str,
    offer_sdp: &str,
) {
    // Create peer connection for remote offer (binds to 0.0.0.0, ICE-lite)
    match super::peer::create_remote_peer(offer_sdp.to_string(), state.clone()).await {
        Ok(answer_sdp) => {
            info!("Created peer connection for remote offer, posting answer");

            let answer_url = format!(
                "{}/api/signaling/{}/answer",
                config.relay_url, config.instance_id
            );

            match client
                .post(&answer_url)
                .bearer_auth(&config.api_token)
                .json(&serde_json::json!({
                    "nonce": nonce,
                    "sdp": answer_sdp,
                }))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    info!("Answer posted for nonce {nonce}");
                }
                Ok(resp) => {
                    warn!("Failed to post answer: {}", resp.status());
                }
                Err(e) => {
                    warn!("Error posting answer: {}", e);
                }
            }
        }
        Err(e) => {
            warn!("Failed to create peer for remote offer: {}", e);
        }
    }
}
