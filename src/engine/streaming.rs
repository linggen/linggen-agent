use super::types::*;
use crate::engine::actions;
use crate::engine::render::normalize_tool_path_arg;
use crate::ollama::ChatMessage;
use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;
use tokio_stream::StreamExt as TokioStreamExt;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Free functions for parallel tool execution
// ---------------------------------------------------------------------------

/// Returns true for tools that are safe to execute in parallel.
/// Includes read-only tools and write tools (Write/Edit) which are
/// parallelizable when targeting different files (checked separately).
pub(crate) fn can_parallel_tool(tool: &str) -> bool {
    matches!(
        tool,
        "Read" | "Glob" | "Grep" | "WebSearch" | "WebFetch"
            | "capture_screenshot" | "Skill" | "Write" | "Edit"
    )
}

/// Check if a batch of parallelizable actions has write-path conflicts
/// (multiple Write/Edit targeting the same file). Returns true if conflicts
/// exist, meaning the batch must fall back to sequential execution.
pub(crate) fn has_write_path_conflicts(
    actions: &[(&str, &serde_json::Value)],
    ws_root: &Path,
) -> bool {
    let mut write_paths: HashSet<String> = HashSet::new();
    for (tool, args) in actions {
        if matches!(*tool, "Write" | "Edit") {
            match normalize_tool_path_arg(ws_root, args) {
                Some(path) => {
                    if !write_paths.insert(path) {
                        return true; // duplicate path
                    }
                }
                None => return true, // can't determine path — be safe
            }
        }
    }
    false
}

/// Check if context files (CLAUDE.md, MEMORY.md, etc.) have changed
/// by comparing a content hash against the previous hash.
/// Runs on a background thread during tool execution.
pub(crate) fn check_context_staleness(
    prev_hash: Option<u64>,
    ws_root: &Path,
    memory_dir: Option<&Path>,
) -> bool {
    let Some(prev_hash) = prev_hash else { return false };
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    for name in &["CLAUDE.md", "AGENTS.md", ".cursorrules"] {
        if let Ok(content) = std::fs::read_to_string(ws_root.join(name)) {
            content.hash(&mut hasher);
        }
    }
    if let Some(mem_dir) = memory_dir {
        if let Ok(content) = std::fs::read_to_string(mem_dir.join("MEMORY.md")) {
            content.hash(&mut hasher);
        }
    }
    hasher.finish() != prev_hash
}

// ---------------------------------------------------------------------------
// AgentEngine model streaming methods
// ---------------------------------------------------------------------------

impl AgentEngine {
    /// Stream model output with thinking-token forwarding.
    ///
    /// Uses `chat_text_stream` (no format constraint) instead of `chat_json`
    /// so the model can emit prose "thinking" tokens before the JSON action.
    /// Thinking tokens are forwarded via `self.thinking_tx` and the full
    /// accumulated text is returned for action parsing.
    pub(crate) async fn stream_with_thinking_model(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
    ) -> Result<StreamResult> {
        use crate::agent_manager::models::StreamChunk;
        let mut stream = self
            .model_manager
            .chat_text_stream(model_id, messages)
            .await?;
        let mut accumulated = String::new();
        let mut thinking_ended = false;
        let mut token_usage = None;
        let mut first_action: Option<(actions::ModelAction, usize)> = None;

        while let Some(chunk_result) = TokioStreamExt::next(&mut stream).await {
            let chunk = chunk_result?;
            match chunk {
                StreamChunk::Token(token) => {
                    accumulated.push_str(&token);
                    if !thinking_ended {
                        if Self::looks_like_json_action_start(&accumulated) {
                            thinking_ended = true;
                            if let Some(tx) = &self.thinking_tx {
                                let _ = tx.send(ThinkingEvent::Done);
                            }
                        } else if let Some(tx) = &self.thinking_tx {
                            let _ = tx.send(ThinkingEvent::Token(token));
                        }
                    }
                    // After thinking ended, try to parse the first complete action
                    // from the accumulated buffer. This avoids double-parsing later.
                    if thinking_ended && first_action.is_none() {
                        if let Some(parsed) = actions::try_parse_first_action(&accumulated) {
                            first_action = Some(parsed);
                        }
                    }
                }
                StreamChunk::Usage(usage) => {
                    token_usage = Some(usage);
                }
            }
        }

        // If thinking never ended (entire response was prose), signal done.
        if !thinking_ended {
            if let Some(tx) = &self.thinking_tx {
                let _ = tx.send(ThinkingEvent::Done);
            }
        }

        Ok(StreamResult {
            full_text: accumulated,
            token_usage,
            first_action,
        })
    }

    /// Build an ordered model chain for fallback attempts.
    ///
    /// Order: primary model → `routing.default_models` → remaining configured models.
    /// Filters out models marked unavailable by the health tracker (keeps at least one).
    fn build_model_chain(&self) -> Vec<String> {
        let primary = self.model_id.clone();
        let all_ids = self.model_manager.model_ids();

        // Start with the primary, then default_models from config, then remaining.
        let mut chain = vec![primary.clone()];
        for dm in &self.default_models {
            if !chain.contains(dm) && all_ids.contains(dm) {
                chain.push(dm.clone());
            }
        }
        for id in &all_ids {
            if !chain.contains(id) {
                chain.push(id.clone());
            }
        }
        chain
    }

    /// Call the LLM with automatic fallback to other configured models
    /// when the primary model hits a rate limit (429) or context limit (400).
    /// Uses the health tracker to skip models known to be down/quota-exhausted.
    pub(crate) async fn stream_with_fallback(&mut self, messages: &[ChatMessage]) -> Result<StreamResult> {
        use crate::agent_manager::models;

        let chain = self.build_model_chain();
        let primary = self.model_id.clone();
        let health = self.model_manager.health.clone();
        let mut last_err: Option<anyhow::Error> = None;

        for model_id in &chain {
            // Skip models known to be unavailable (but always try at least the primary).
            if model_id != &primary && !health.is_available(model_id).await {
                info!("Skipping model '{}' (health tracker: unavailable)", model_id);
                continue;
            }

            match self.stream_with_thinking_model(model_id, messages).await {
                Ok(result) => {
                    health.mark_healthy(model_id).await;
                    self.last_token_usage = result.token_usage.clone();
                    if model_id != &primary {
                        let reason = last_err
                            .as_ref()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "unavailable".to_string());
                        info!(
                            "Fallback to '{}' succeeded (primary '{}' failed: {})",
                            model_id, primary, reason
                        );
                        self.model_id = model_id.clone();
                        self.emit_model_fallback_event(&primary, model_id, &reason).await;
                    }
                    return Ok(result);
                }
                Err(e) if models::is_fallback_worthy_error(&e) => {
                    warn!(
                        "Model '{}' returned fallback-worthy error: {}",
                        model_id, e
                    );
                    health.mark_error(model_id, &e.to_string()).await;
                    last_err = Some(e);
                    continue;
                }
                Err(e) => {
                    // Non-fallback-worthy error (e.g. network down, bad config) — don't try others.
                    health.mark_error(model_id, &e.to_string()).await;
                    return Err(e);
                }
            }
        }

        // All models exhausted — return the last error.
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("No models available")))
    }

    /// Emit a ModelFallback event via the agent manager.
    async fn emit_model_fallback_event(&self, preferred: &str, actual: &str, reason: &str) {
        let Some(manager) = self.tools.get_manager() else { return };
        let agent_id = self.agent_id.clone().unwrap_or_else(|| "unknown".to_string());
        manager
            .send_event(crate::agent_manager::AgentEvent::ModelFallback {
                agent_id,
                preferred_model: preferred.to_string(),
                actual_model: actual.to_string(),
                reason: reason.to_string(),
            })
            .await;
    }

    /// Drain any pending tool progress lines from the channel and forward
    /// them as AgentEvent::ToolProgress to the manager.
    pub(crate) async fn drain_tool_progress(
        &self,
        progress_rx: &mut tokio::sync::mpsc::UnboundedReceiver<(String, String, String)>,
    ) {
        let Some(manager) = self.tools.get_manager() else { return };
        let agent_id = self.agent_id.clone().unwrap_or_else(|| "unknown".to_string());
        while let Ok((tool, stream, line)) = progress_rx.try_recv() {
            manager
                .send_event(crate::agent_manager::AgentEvent::ToolProgress {
                    agent_id: agent_id.clone(),
                    tool,
                    line,
                    stream,
                })
                .await;
        }
    }

    pub(crate) fn looks_like_json_action_start(text: &str) -> bool {
        if let Some(brace_idx) = text.rfind('{') {
            text[brace_idx..].contains("\"type\"")
        } else {
            false
        }
    }
}
