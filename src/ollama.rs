use anyhow::Result;
use futures_util::Stream;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio_util::codec::{FramedRead, LinesCodec};

#[derive(Clone)]
pub struct OllamaClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
}

impl OllamaClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    /// Ask Ollama to return a JSON-formatted assistant message (we set format: "json").
    pub async fn chat_json(&self, model: &str, messages: &[ChatMessage]) -> Result<String> {
        self.chat_json_with_keep_alive(model, messages, None).await
    }

    pub async fn chat_json_with_keep_alive(
        &self,
        model: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
    ) -> Result<String> {
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
            keep_alive,
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
        self.chat_text_with_keep_alive(model, messages, None).await
    }

    #[allow(dead_code)]
    pub async fn chat_text_with_keep_alive(
        &self,
        model: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
    ) -> Result<String> {
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
            keep_alive,
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
        self.chat_text_stream_with_keep_alive(model, messages, None)
            .await
    }

    pub async fn chat_text_stream_with_keep_alive(
        &self,
        model: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
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
            keep_alive,
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

    /// Preload a model into memory and keep it there.
    pub async fn preload_model(&self, model: &str, keep_alive: &str) -> Result<()> {
        tracing::info!(
            "Preloading Ollama model: {} (keep_alive={})",
            model,
            keep_alive
        );
        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: model.to_string(),
            messages: vec![],
            stream: Some(false),
            format: None,
            keep_alive: Some(keep_alive.to_string()),
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
        Ok(())
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

    /// Best-effort: fetch model context window (num_ctx) from Ollama.
    ///
    /// Ollama exposes model metadata at /api/show. We parse either:
    /// - parameters.num_ctx (if present in object form),
    /// - num_ctx/context_length lines from parameters/modelfile text, or
    /// - model_info.*.context_length keys.
    pub async fn get_model_context_window(&self, model: &str) -> Result<Option<usize>> {
        let url = format!("{}/api/show", self.base_url);
        let req = serde_json::json!({ "name": model });
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

        let payload: OllamaShowResponse = resp.json().await?;

        // 1) parameters object or text
        if let Some(params) = payload.parameters.as_ref() {
            match params {
                OllamaShowParameters::Map(map) => {
                    if let Some(v) = map.get("num_ctx").and_then(parse_usize_value) {
                        return Ok(Some(v));
                    }
                    if let Some(v) = map.get("context_length").and_then(parse_usize_value) {
                        return Ok(Some(v));
                    }
                }
                OllamaShowParameters::Text(text) => {
                    if let Some(v) = parse_num_ctx_from_text(text) {
                        return Ok(Some(v));
                    }
                }
            }
        }

        // 2) parse from model_info keys like "<arch>.context_length"
        if let Some(model_info) = payload.model_info.as_ref() {
            for (key, value) in model_info {
                let key_lc = key.to_ascii_lowercase();
                if key_lc == "context_length"
                    || key_lc.ends_with(".context_length")
                    || key_lc.ends_with("_context_length")
                {
                    if let Some(v) = parse_usize_value(value) {
                        return Ok(Some(v));
                    }
                }
            }
        }

        // 3) parse from details object when available
        if let Some(details) = payload.details.as_ref() {
            for k in ["context_length", "num_ctx"] {
                if let Some(v) = details.get(k).and_then(parse_usize_value) {
                    return Ok(Some(v));
                }
            }
        }

        // 4) parse from modelfile text
        if let Some(modelfile) = payload.modelfile.as_deref() {
            if let Some(v) = parse_num_ctx_from_text(modelfile) {
                return Ok(Some(v));
            }
        }

        Ok(None)
    }
}

fn parse_usize_value(value: &serde_json::Value) -> Option<usize> {
    if let Some(v) = value.as_u64() {
        return usize::try_from(v).ok();
    }
    if let Some(s) = value.as_str() {
        return parse_usize_token(s);
    }
    None
}

fn parse_usize_token(raw: &str) -> Option<usize> {
    let cleaned = raw.trim().trim_matches('"').trim_matches('\'');
    if let Ok(v) = cleaned.parse::<usize>() {
        return Some(v);
    }
    for token in cleaned.split(|c: char| c.is_whitespace() || c == '=' || c == ':') {
        let t = token.trim().trim_matches(',').trim_matches(';');
        if t.is_empty() {
            continue;
        }
        if let Ok(v) = t.parse::<usize>() {
            return Some(v);
        }
    }
    None
}

fn parse_num_ctx_from_line(line: &str) -> Option<usize> {
    let mut s = line.trim();
    if let Some(rest) = s.strip_prefix("PARAMETER") {
        s = rest.trim();
    }
    if s.is_empty() {
        return None;
    }

    if let Some((k, v)) = s.split_once('=') {
        let key = k.trim().to_ascii_lowercase();
        if key == "num_ctx" || key == "context_length" {
            return parse_usize_token(v);
        }
    }
    if let Some((k, v)) = s.split_once(':') {
        let key = k.trim().to_ascii_lowercase();
        if key == "num_ctx" || key == "context_length" {
            return parse_usize_token(v);
        }
    }

    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() >= 2 {
        let key = parts[0].trim().to_ascii_lowercase();
        if key == "num_ctx" || key == "context_length" {
            return parse_usize_token(parts[1]);
        }
    }
    None
}

fn parse_num_ctx_from_text(text: &str) -> Option<usize> {
    for line in text.lines() {
        if let Some(v) = parse_num_ctx_from_line(line) {
            return Some(v);
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaShowResponse {
    #[serde(default)]
    parameters: Option<OllamaShowParameters>,
    #[serde(default)]
    modelfile: Option<String>,
    #[serde(default)]
    model_info: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    details: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum OllamaShowParameters {
    Map(HashMap<String, serde_json::Value>),
    Text(String),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_alive: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}
