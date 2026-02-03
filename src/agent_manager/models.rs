use crate::config::ModelConfig;
use crate::ollama::{ChatMessage, OllamaClient};
use anyhow::Result;
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

    pub async fn chat_text(&self, model_id: &str, messages: &[ChatMessage]) -> Result<String> {
        let instance = self.models.get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        
        let _permit = instance.semaphore.acquire().await?;
        instance.client.chat_text(&instance.config.model, messages).await
    }
}
