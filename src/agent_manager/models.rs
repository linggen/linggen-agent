use crate::config::ModelConfig;
use crate::ollama::{ChatMessage, OllamaClient};
use anyhow::Result;
use futures_util::{Stream, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;

pub struct ModelManager {
    models: HashMap<String, ModelInstance>,
}

struct ModelInstance {
    config: ModelConfig,
    client: OllamaClient,
    semaphore: Arc<Semaphore>,
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
            });
        }
        Self { models }
    }

    pub async fn chat_json(&self, model_id: &str, messages: &[ChatMessage]) -> Result<String> {
        let instance = self.models.get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        
        let _permit = instance.semaphore.acquire().await?;
        instance.client.chat_json(&instance.config.model, messages).await
    }

    pub async fn chat_text_stream(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
    ) -> Result<impl Stream<Item = Result<String>> + Send + Unpin> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        // Note: permit is held for the duration of the stream
        let _permit = instance.semaphore.clone().acquire_owned().await?;
        let stream = instance
            .client
            .chat_text_stream(&instance.config.model, messages)
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

    pub fn list_models(&self) -> Vec<&ModelConfig> {
        self.models.values().map(|m| &m.config).collect()
    }
}
