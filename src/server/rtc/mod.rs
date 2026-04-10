//! WebRTC transport — WHIP signaling + str0m data channels.
//!
//! This module handles:
//! - WHIP endpoint (`POST /api/rtc/whip`) for SDP offer/answer exchange
//! - Data channel management (control channel + per-session channels)
//! - Bridging data channel messages to/from the existing event system
//!
//! str0m is Sans-IO: we drive the event loop ourselves using a UDP socket
//! in a tokio task per peer connection.

mod peer;
pub(crate) mod page_state;
pub mod proxy_client;
pub mod proxy_room;
pub mod relay;
pub mod room_config;

/// Context for a proxy room consumer connection.
/// When present, the peer is a consumer (not the instance owner) and
/// permissions/tools are restricted accordingly.
#[derive(Debug, Clone)]
pub struct ConsumerContext {
    /// "browser" = chat mode only (no tools), "linggen" = read mode (capped)
    pub consumer_type: String,
    /// Optional daily token budget from the room config
    pub token_budget_daily: Option<i64>,
    /// Consumer's user ID on linggen.dev
    pub consumer_user_id: Option<String>,
}

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::server::ServerState;

/// Return the WHIP auth token (local UI fetches this before connecting).
pub async fn whip_token_handler(
    State(state): State<Arc<ServerState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "token": state.whip_token }))
}

/// WHIP endpoint: accept SDP offer, return SDP answer.
///
/// The client sends a complete SDP offer (with ICE candidates bundled).
/// We create an Rtc instance, bind a UDP socket, accept the offer,
/// and return the SDP answer. The peer connection runs in a background task.
///
/// Requires `Authorization: Bearer <whip_token>` header.
pub async fn whip_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Verify WHIP auth token
    let auth_ok = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t == state.whip_token)
        .unwrap_or(false);
    if !auth_ok {
        return (StatusCode::UNAUTHORIZED, "Invalid or missing WHIP token").into_response();
    }

    let offer_str = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid UTF-8 in SDP offer").into_response();
        }
    };

    match peer::create_peer(offer_str, state).await {
        Ok(answer_sdp) => (
            StatusCode::CREATED,
            [(header::CONTENT_TYPE, "application/sdp")],
            answer_sdp,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("WHIP error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("WHIP error: {e}")).into_response()
        }
    }
}
