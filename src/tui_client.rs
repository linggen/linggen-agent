use crate::server::UiSseMessage;
use anyhow::Result;
use reqwest::Client;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::warn;

pub struct TuiClient {
    base_url: String,
    client: Client,
}

impl TuiClient {
    pub fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{}", port),
            client: Client::new(),
        }
    }

    pub async fn health_check(&self) -> bool {
        self.client
            .get(format!("{}/api/health", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    pub async fn send_chat(
        &self,
        project_root: &str,
        agent_id: &str,
        message: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        let mut body = json!({
            "project_root": project_root,
            "agent_id": agent_id,
            "message": message,
        });
        if let Some(sid) = session_id {
            body["session_id"] = json!(sid);
        }
        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Chat request failed ({}): {}", status, text);
        }
        Ok(())
    }

    /// Subscribe to SSE events. Returns an unbounded receiver that yields parsed UiSseMessage.
    /// The SSE connection runs in a background task.
    pub fn subscribe_sse(&self) -> mpsc::UnboundedReceiver<UiSseMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        let url = format!("{}/api/events", self.base_url);
        let client = self.client.clone();

        tokio::spawn(async move {
            let resp = match client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    warn!("SSE connect failed: {}", e);
                    return;
                }
            };

            let mut stream = resp.bytes_stream();
            let mut buf = String::new();

            use futures_util::StreamExt;
            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("SSE stream error: {}", e);
                        break;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                // Parse SSE frames: lines starting with "data:" separated by blank lines.
                while let Some(pos) = buf.find("\n\n") {
                    let frame = buf[..pos].to_string();
                    buf = buf[pos + 2..].to_string();

                    for line in frame.lines() {
                        let data = if let Some(d) = line.strip_prefix("data:") {
                            d.trim()
                        } else {
                            continue;
                        };
                        if data.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<UiSseMessage>(data) {
                            Ok(msg) => {
                                if tx.send(msg).is_err() {
                                    return; // receiver dropped
                                }
                            }
                            Err(_) => {
                                // Skip malformed frames silently
                            }
                        }
                    }
                }
            }
        });

        rx
    }
}
