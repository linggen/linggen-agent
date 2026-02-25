use crate::agent_manager::models::{StreamChunk, TokenUsage};
use anyhow::Result;
use futures_util::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct OpenAiClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
}

impl OpenAiClient {
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

    /// Non-streaming JSON-mode chat completion.
    pub async fn chat_json(
        &self,
        model: &str,
        messages: &[crate::ollama::ChatMessage],
    ) -> Result<String> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        if let Some(last) = messages.last() {
            tracing::info!(
                "OpenAI Request (JSON): model={}, messages={}, total_chars={}\nLast Message ({}): {:.200}...",
                model, messages.len(), total_len, last.role, last.content
            );
        } else {
            tracing::info!(
                "OpenAI Request (JSON): model={}, messages={}, total_chars={}",
                model, messages.len(), total_len
            );
        }

        let url = format!("{}/chat/completions", self.base_url);
        let oai_messages: Vec<OaiMessage> = messages.iter().map(OaiMessage::from_chat).collect();
        let req = OaiRequest {
            model: model.to_string(),
            messages: oai_messages,
            stream: false,
            response_format: Some(OaiResponseFormat {
                r#type: "json_object".to_string(),
            }),
        };

        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("openai error ({}): {}", status, text);
        }

        let payload: OaiChatResponse = resp.json().await?;
        let content = payload
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();
        Ok(content)
    }

    /// Streaming text chat completion (SSE format).
    pub async fn chat_text_stream(
        &self,
        model: &str,
        messages: &[crate::ollama::ChatMessage],
    ) -> Result<impl Stream<Item = Result<StreamChunk>> + Send> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        if let Some(last) = messages.last() {
            tracing::info!(
                "OpenAI Request (Stream): model={}, messages={}, total_chars={}\nLast Message ({}): {:.200}...",
                model, messages.len(), total_len, last.role, last.content
            );
        } else {
            tracing::info!(
                "OpenAI Request (Stream): model={}, messages={}, total_chars={}",
                model, messages.len(), total_len
            );
        }

        let url = format!("{}/chat/completions", self.base_url);
        let oai_messages: Vec<OaiMessage> = messages.iter().map(OaiMessage::from_chat).collect();
        let req = OaiRequest {
            model: model.to_string(),
            messages: oai_messages,
            stream: true,
            response_format: None,
        };

        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("openai error ({}): {}", status, text);
        }

        // OpenAI streams SSE: "data: {...}\n\n" lines, terminated by "data: [DONE]"
        let byte_stream = resp
            .bytes_stream()
            .map(|item| item.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
        let reader = tokio_util::io::StreamReader::new(byte_stream);
        let lines = tokio_util::codec::FramedRead::new(reader, tokio_util::codec::LinesCodec::new());

        use futures_util::StreamExt;
        let token_stream = lines.filter_map(|line_result| async move {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => return Some(Err(anyhow::anyhow!("stream error: {}", e))),
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let data = match trimmed.strip_prefix("data: ") {
                Some(d) => d.trim(),
                None => return None,
            };
            if data == "[DONE]" {
                return None;
            }
            let chunk: OaiStreamChunk = match serde_json::from_str(data) {
                Ok(c) => c,
                Err(e) => {
                    return Some(Err(anyhow::anyhow!(
                        "openai json parse error: {} (data: {})",
                        e,
                        data
                    )));
                }
            };

            // Check for usage data (some providers include it in the final chunk).
            if let Some(usage) = chunk.usage {
                return Some(Ok(StreamChunk::Usage(TokenUsage {
                    prompt_tokens: usage.prompt_tokens.map(|v| v as usize),
                    completion_tokens: usage.completion_tokens.map(|v| v as usize),
                    total_tokens: usage.total_tokens.map(|v| v as usize),
                })));
            }

            let content = chunk
                .choices
                .into_iter()
                .next()
                .and_then(|c| c.delta.content)
                .unwrap_or_default();
            if content.is_empty() {
                None
            } else {
                Some(Ok(StreamChunk::Token(content)))
            }
        });

        Ok(token_stream)
    }
}

// --- Wire types ---

#[derive(Debug, Serialize)]
struct OaiMessage {
    role: String,
    content: OaiContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OaiContent {
    Text(String),
    Parts(Vec<OaiContentPart>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum OaiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OaiImageUrl },
}

#[derive(Debug, Serialize)]
struct OaiImageUrl {
    url: String,
}

impl OaiMessage {
    fn from_chat(msg: &crate::ollama::ChatMessage) -> Self {
        let content = if msg.images.is_empty() {
            OaiContent::Text(msg.content.clone())
        } else {
            let mut parts = vec![OaiContentPart::Text {
                text: msg.content.clone(),
            }];
            for img in &msg.images {
                parts.push(OaiContentPart::ImageUrl {
                    image_url: OaiImageUrl {
                        url: format!("data:image/png;base64,{}", img),
                    },
                });
            }
            OaiContent::Parts(parts)
        };
        Self {
            role: msg.role.clone(),
            content,
        }
    }
}

#[derive(Debug, Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<OaiResponseFormat>,
}

#[derive(Debug, Serialize)]
struct OaiResponseFormat {
    r#type: String,
}

#[derive(Debug, Deserialize)]
struct OaiChatResponse {
    choices: Vec<OaiChoice>,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    message: OaiChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct OaiChoiceMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct OaiStreamChunk {
    choices: Vec<OaiStreamChoice>,
    #[serde(default)]
    usage: Option<OaiUsage>,
}

#[derive(Debug, Deserialize)]
struct OaiUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OaiStreamChoice {
    delta: OaiStreamDelta,
}

#[derive(Debug, Deserialize)]
struct OaiStreamDelta {
    content: Option<String>,
}
