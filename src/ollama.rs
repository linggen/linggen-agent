use anyhow::Result;
use futures_util::Stream;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio_util::codec::{FramedRead, LinesCodec};

#[derive(Clone)]
pub struct OllamaClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
}

impl OllamaClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    /// Ask Ollama to return a JSON-formatted assistant message (we set format: "json").
    pub async fn chat_json(&self, model: &str, messages: &[ChatMessage]) -> Result<String> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        if let Some(last) = messages.last() {
            tracing::info!("Ollama Request (JSON): model={}, messages={}, total_chars={}\nLast Message ({}): {:.200}...", 
                model, messages.len(), total_len, last.role, last.content);
        } else {
            tracing::info!(
                "Ollama Request (JSON): model={}, messages={}, total_chars={}",
                model,
                messages.len(),
                total_len
            );
        }

        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            stream: Some(false),
            format: Some("json".to_string()),
        };

        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("ollama error ({}): {}", status, text);
        }

        let payload: ChatResponse = resp.json().await?;
        Ok(payload.message.content)
    }

    /// Plain text chat (no structured output enforcement).
    #[allow(dead_code)]
    pub async fn chat_text(&self, model: &str, messages: &[ChatMessage]) -> Result<String> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!(
            "Ollama Request (Text): model={}, messages={}, total_chars={}",
            model,
            messages.len(),
            total_len
        );

        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            stream: Some(false),
            format: None,
        };

        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("ollama error ({}): {}", status, text);
        }

        let payload: ChatResponse = resp.json().await?;
        Ok(payload.message.content)
    }

    /// Streaming text chat.
    pub async fn chat_text_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
    ) -> Result<impl Stream<Item = Result<String>> + Send> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        if let Some(last) = messages.last() {
            tracing::info!("Ollama Request (Stream): model={}, messages={}, total_chars={}\nLast Message ({}): {:.200}...", 
                model, messages.len(), total_len, last.role, last.content);
        } else {
            tracing::info!(
                "Ollama Request (Stream): model={}, messages={}, total_chars={}",
                model,
                messages.len(),
                total_len
            );
        }

        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            stream: Some(true),
            format: None,
        };

        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("ollama error ({}): {}", status, text);
        }

        let stream = resp
            .bytes_stream()
            .map(|item| item.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
        let reader = tokio_util::io::StreamReader::new(stream);
        let lines = FramedRead::new(reader, LinesCodec::new());

        let token_stream = lines.map(|line_result| {
            let line = line_result.map_err(|e| anyhow::anyhow!("stream error: {}", e))?;
            if line.trim().is_empty() {
                return Ok("".to_string());
            }
            // Ollama sends one JSON object per line
            let payload: ChatResponse = serde_json::from_str(&line)
                .map_err(|e| anyhow::anyhow!("json parse error: {} (line: {})", e, line))?;
            Ok(payload.message.content)
        });

        Ok(token_stream)
    }

    /// Get the status of currently running models in Ollama.
    pub async fn get_ps(&self) -> Result<OllamaPsResponse> {
        let url = format!("{}/api/ps", self.base_url);
        let mut rb = self.http.get(url);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let payload: OllamaPsResponse = resp.json().await?;
        Ok(payload)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaPsResponse {
    pub models: Vec<OllamaPsModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaPsModel {
    pub name: String,
    pub model: String,
    pub size: u64,
    pub size_vram: u64,
    pub details: OllamaPsModelDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaPsModelDetails {
    pub parent_model: String,
    pub format: String,
    pub family: String,
    pub parameter_size: String,
    pub quantization_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}
