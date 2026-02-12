use crate::config::ModelConfig;
use crate::ollama::{ChatMessage, OllamaClient};
use anyhow::Result;
use futures_util::{Stream, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tokio::sync::Semaphore;

pub struct ModelManager {
    models: HashMap<String, ModelInstance>,
}

struct ModelInstance {
    config: ModelConfig,
    client: OllamaClient,
    semaphore: Arc<Semaphore>,
    context_window: OnceCell<Option<usize>>,
}

impl ModelManager {
    pub fn new(configs: Vec<ModelConfig>) -> Self {
        let mut models = HashMap::new();
        for cfg in configs {
            let client = OllamaClient::new(cfg.url.clone(), cfg.api_key.clone());
            // Default to 1 concurrent request per model if not specified
            // (We could add max_concurrent to ModelConfig later)
            let semaphore = Arc::new(Semaphore::new(1));
            models.insert(cfg.id.clone(), ModelInstance {
                config: cfg,
                client,
                semaphore,
                context_window: OnceCell::new(),
            });
        }
        Self { models }
    }

    pub async fn chat_json(&self, model_id: &str, messages: &[ChatMessage]) -> Result<String> {
        let instance = self.models.get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        self.chat_json_with_keep_alive(model_id, messages, instance.config.keep_alive.clone()).await
    }

    pub async fn chat_json_with_keep_alive(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
    ) -> Result<String> {
        let instance = self.models.get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        
        let _permit = instance.semaphore.acquire().await?;
        instance.client.chat_json_with_keep_alive(&instance.config.model, messages, keep_alive).await
    }

    pub async fn chat_text_stream(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
    ) -> Result<impl Stream<Item = Result<String>> + Send + Unpin> {
        let instance = self.models.get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        self.chat_text_stream_with_keep_alive(model_id, messages, instance.config.keep_alive.clone()).await
    }

    pub async fn chat_text_stream_with_keep_alive(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
    ) -> Result<impl Stream<Item = Result<String>> + Send + Unpin> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        // Note: permit is held for the duration of the stream
        let _permit = instance.semaphore.clone().acquire_owned().await?;
        let stream = instance
            .client
            .chat_text_stream_with_keep_alive(&instance.config.model, messages, keep_alive)
            .await?;

        // Wrap stream to ensure permit is released when stream ends
        Ok(Box::pin(futures_util::stream::unfold(
            (Box::pin(stream), _permit),
            |(mut stream, permit)| async move {
                match stream.next().await {
                    Some(item) => Some((item, (stream, permit))),
                    None => None,
                }
            },
        )))
    }

    pub async fn preload_model(&self, model_id: &str, keep_alive: &str) -> Result<()> {
        let instance = self.models.get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        instance.client.preload_model(&instance.config.model, keep_alive).await
    }

    pub fn list_models(&self) -> Vec<&ModelConfig> {
        self.models.values().map(|m| &m.config).collect()
    }

    /// Best-effort cached model context window (num_ctx).
    pub async fn context_window(&self, model_id: &str) -> Result<Option<usize>> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        let model_name = instance.config.model.clone();
        let client = instance.client.clone();
        let value = instance
            .context_window
            .get_or_init(|| async move { client.get_model_context_window(&model_name).await.ok().flatten() })
            .await;
        Ok(*value)
    }
}
