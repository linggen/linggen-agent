use crate::config::ModelConfig;
use crate::ollama::{ChatMessage, OllamaClient};
use crate::openai::OpenAiClient;
use anyhow::Result;
use futures_util::{Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tokio::sync::Semaphore;

/// Provider-specific client variant.
enum ProviderClient {
    Ollama(OllamaClient),
    OpenAi(OpenAiClient),
}

pub struct ModelManager {
    models: HashMap<String, ModelInstance>,
}

struct ModelInstance {
    config: ModelConfig,
    client: ProviderClient,
    semaphore: Arc<Semaphore>,
    context_window: OnceCell<Option<usize>>,
}

impl ModelManager {
    pub fn new(configs: Vec<ModelConfig>) -> Self {
        let mut models = HashMap::new();
        for cfg in configs {
            let client = match cfg.provider.as_str() {
                "openai" => {
                    ProviderClient::OpenAi(OpenAiClient::new(cfg.url.clone(), cfg.api_key.clone()))
                }
                _ => {
                    ProviderClient::Ollama(OllamaClient::new(cfg.url.clone(), cfg.api_key.clone()))
                }
            };
            let semaphore = Arc::new(Semaphore::new(1));
            models.insert(
                cfg.id.clone(),
                ModelInstance {
                    config: cfg,
                    client,
                    semaphore,
                    context_window: OnceCell::new(),
                },
            );
        }
        Self { models }
    }

    pub async fn chat_json(&self, model_id: &str, messages: &[ChatMessage]) -> Result<String> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        self.chat_json_with_keep_alive(model_id, messages, instance.config.keep_alive.clone())
            .await
    }

    pub async fn chat_json_with_keep_alive(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
    ) -> Result<String> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        let _permit = instance.semaphore.acquire().await?;
        match &instance.client {
            ProviderClient::Ollama(client) => {
                client
                    .chat_json_with_keep_alive(&instance.config.model, messages, keep_alive)
                    .await
            }
            ProviderClient::OpenAi(client) => {
                client.chat_json(&instance.config.model, messages).await
            }
        }
    }

    pub async fn chat_text_stream(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String>> + Send>>> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        self.chat_text_stream_with_keep_alive(
            model_id,
            messages,
            instance.config.keep_alive.clone(),
        )
        .await
    }

    pub async fn chat_text_stream_with_keep_alive(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String>> + Send>>> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        // Note: permit is held for the duration of the stream
        let _permit = instance.semaphore.clone().acquire_owned().await?;

        match &instance.client {
            ProviderClient::Ollama(client) => {
                let stream = client
                    .chat_text_stream_with_keep_alive(&instance.config.model, messages, keep_alive)
                    .await?;
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
            ProviderClient::OpenAi(client) => {
                let stream = client
                    .chat_text_stream(&instance.config.model, messages)
                    .await?;
                let boxed_stream: Pin<Box<dyn Stream<Item = Result<String>> + Send>> =
                    Box::pin(stream);
                Ok(Box::pin(futures_util::stream::unfold(
                    (boxed_stream, _permit),
                    |(mut stream, permit)| async move {
                        match stream.next().await {
                            Some(item) => Some((item, (stream, permit))),
                            None => None,
                        }
                    },
                )))
            }
        }
    }

    pub async fn preload_model(&self, model_id: &str, keep_alive: &str) -> Result<()> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        match &instance.client {
            ProviderClient::Ollama(client) => {
                client
                    .preload_model(&instance.config.model, keep_alive)
                    .await
            }
            // OpenAI-compatible APIs don't support preloading; no-op.
            ProviderClient::OpenAi(_) => Ok(()),
        }
    }

    pub fn list_models(&self) -> Vec<&ModelConfig> {
        self.models.values().map(|m| &m.config).collect()
    }

    /// Best-effort cached model context window (num_ctx).
    /// Priority: config override > Ollama /api/show > None.
    pub async fn context_window(&self, model_id: &str) -> Result<Option<usize>> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        // Config override takes priority (useful for cloud/remote models).
        if let Some(cw) = instance.config.context_window {
            return Ok(Some(cw));
        }

        match &instance.client {
            ProviderClient::Ollama(client) => {
                let model_name = instance.config.model.clone();
                let client = client.clone();
                let value = instance
                    .context_window
                    .get_or_try_init(|| async move {
                        client.get_model_context_window(&model_name).await
                    })
                    .await;
                Ok(*value?)
            }
            ProviderClient::OpenAi(_) => Ok(None),
        }
    }

    /// Return the OllamaClient for a specific model (if it's an Ollama provider).
    /// Used by server status endpoints.
    pub fn ollama_client_for_model(&self, model_id: &str) -> Option<&OllamaClient> {
        let instance = self.models.get(model_id)?;
        match &instance.client {
            ProviderClient::Ollama(client) => Some(client),
            ProviderClient::OpenAi(_) => None,
        }
    }

    /// Return the first OllamaClient found among configured models.
    pub fn first_ollama_client(&self) -> Option<&OllamaClient> {
        for instance in self.models.values() {
            if let ProviderClient::Ollama(client) = &instance.client {
                return Some(client);
            }
        }
        None
    }
}
