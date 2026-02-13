pub mod actions;
pub mod patch;
pub mod render;
pub mod tools;

use crate::agent_manager::models::ModelManager;
use crate::agent_manager::AgentManager;
use crate::config::{AgentKind, AgentPolicyCapability, AgentSpec};
use crate::engine::patch::validate_unified_diff;
use crate::engine::tools::{ToolCall, Tools};
use crate::ollama::ChatMessage;
use crate::skills::Skill;
use anyhow::Result;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

pub use actions::{model_message_log_parts, parse_first_action, ModelAction};
pub use render::{
    normalize_tool_path_arg, render_tool_result, render_tool_result_public,
    sanitize_tool_args_for_display, tool_call_signature,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRole {
    #[serde(rename = "lead")]
    Lead,
    #[serde(rename = "coder")]
    Coder,
    #[serde(rename = "operator")]
    Operator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPacket {
    pub title: String,
    pub user_stories: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub mermaid_wireframe: Option<String>,
}

pub struct EngineConfig {
    pub ws_root: PathBuf,
    pub max_iters: usize,
    #[allow(dead_code)]
    pub stream: bool,
    pub write_safety_mode: crate::config::WriteSafetyMode,
    pub prompt_loop_breaker: Option<String>,
}

const CONTEXT_SOFT_TOKEN_LIMIT: usize = 8_000;
const CONTEXT_SOFT_MESSAGE_LIMIT: usize = 72;
const CONTEXT_KEEP_TAIL_MESSAGES: usize = 28;
const CONTEXT_MAX_SUMMARY_PASSES: usize = 3;
const CHAT_INPUT_LOG_PREVIEW_CHARS: usize = 240;

fn chat_input_log_preview(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let char_count = collapsed.chars().count();
    if char_count <= CHAT_INPUT_LOG_PREVIEW_CHARS {
        collapsed
    } else {
        let prefix = collapsed
            .chars()
            .take(CHAT_INPUT_LOG_PREVIEW_CHARS)
            .collect::<String>();
        format!("{prefix}... ({char_count} chars)")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextType {
    System,
    UserInput,
    AssistantReply,
    ToolCall,
    ToolResult,
    Observation,
    Status,
    Error,
    Summary,
}

#[derive(Debug, Clone)]
pub struct ObservationRecord {
    pub observation_type: String,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRecord {
    pub id: u64,
    pub ts: u64,
    pub context_type: ContextType,
    pub name: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub content: String,
    pub meta: JsonValue,
}

pub struct AgentEngine {
    pub cfg: EngineConfig,
    pub model_manager: Arc<ModelManager>,
    pub model_id: String,
    pub tools: Tools,
    pub role: AgentRole,
    pub task: Option<String>,
    // Agent spec and runtime context
    pub spec: Option<AgentSpec>,
    pub spec_system_prompt: Option<String>,
    pub agent_id: Option<String>,
    // Rolling observations with metadata that we feed back to the model.
    pub observations: Vec<ObservationRecord>,
    pub context_records: Vec<ContextRecord>,
    pub next_context_id: u64,
    // Conversational history for chat.
    pub chat_history: Vec<ChatMessage>,
    // Active skill if any
    pub active_skill: Option<Skill>,
    pub prompt_mode: PromptMode,
    pub parent_agent_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    Structured,
    Chat,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum AgentOutcome {
    #[serde(rename = "task")]
    Task(TaskPacket),
    #[serde(rename = "patch")]
    Patch(String),
    #[serde(rename = "none")]
    None,
}

impl AgentEngine {
    pub fn new(
        cfg: EngineConfig,
        model_manager: Arc<ModelManager>,
        model_id: String,
        role: AgentRole,
    ) -> Result<Self> {
        let tools = Tools::new(cfg.ws_root.clone())?;
        Ok(Self {
            cfg,
            model_manager,
            model_id,
            tools,
            role,
            task: None,
            spec: None,
            spec_system_prompt: None,
            agent_id: None,
            observations: Vec::new(),
            context_records: Vec::new(),
            next_context_id: 1,
            chat_history: Vec::new(),
            active_skill: None,
            prompt_mode: PromptMode::Structured,
            parent_agent_id: None,
            run_id: None,
        })
    }

    pub fn set_spec(&mut self, agent_id: String, spec: AgentSpec, system_prompt: String) {
        let policy = spec.policy.clone();
        self.agent_id = Some(agent_id);
        self.spec = Some(spec);
        self.spec_system_prompt = Some(system_prompt);
        self.tools.set_policy(Some(policy));
    }

    pub fn set_manager_context(&mut self, manager: Arc<AgentManager>) {
        if let Some(agent_id) = &self.agent_id {
            let kind = self
                .spec
                .as_ref()
                .map(|s| s.kind)
                .unwrap_or(crate::config::AgentKind::Main);
            self.tools.set_context(manager, agent_id.clone(), kind);
        }
    }

    pub fn set_role(&mut self, role: AgentRole) {
        self.role = role;
        self.observations.clear();
        self.context_records.clear();
        self.next_context_id = 1;
        self.chat_history.clear();
    }

    pub fn set_task(&mut self, task: String) {
        self.task = Some(task);
        self.observations.clear();
        self.context_records.clear();
        self.next_context_id = 1;
        self.chat_history.clear();
    }

    pub fn set_parent_agent(&mut self, parent_agent_id: Option<String>) {
        self.parent_agent_id = parent_agent_id;
    }

    pub fn set_run_id(&mut self, run_id: Option<String>) {
        self.run_id = run_id.clone();
        self.tools.set_run_id(run_id);
    }

    pub fn get_task(&self) -> Option<String> {
        self.task.clone()
    }

    pub fn set_prompt_mode(&mut self, mode: PromptMode) {
        self.prompt_mode = mode;
    }

    pub fn get_prompt_mode(&self) -> PromptMode {
        self.prompt_mode
    }

    pub async fn chat_stream(
        &mut self,
        message: &str,
        _session_id: Option<&str>,
        mode: PromptMode,
    ) -> Result<impl Stream<Item = Result<String>> + Send + Unpin> {
        let message_preview = chat_input_log_preview(message);
        info!(
            "Processing chat stream for role {:?}: {}",
            self.role, message_preview
        );

        let mut clean_message = message.to_string();
        if message.starts_with('/') {
            let parts: Vec<&str> = message.splitn(2, ' ').collect();
            let cmd = parts[0].trim_start_matches('/');

            if let Some(manager) = self.tools.get_manager() {
                if let Some(skill) = manager.skill_manager.get_skill(cmd).await {
                    info!("Activating skill: {}", skill.name);
                    self.active_skill = Some(skill);
                    if parts.len() > 1 {
                        clean_message = parts[1].to_string();
                    } else {
                        clean_message = "I'm ready to use this skill. How can I help?".to_string();
                    }
                }
            }
        } else {
            // Prevent a previously activated skill from affecting normal chat.
            self.active_skill = None;
        }

        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: self.system_prompt(),
        }];
        self.push_context_record(
            ContextType::System,
            Some("chat_stream_prompt".to_string()),
            None,
            None,
            messages[0].content.clone(),
            serde_json::json!({ "mode": format!("{:?}", mode) }),
        );
        let allowed_tools = self.allowed_tool_names();

        // Add workspace context to the first message if history is short
        if self.chat_history.len() == 0 {
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "Workspace root: {}\nCurrent Role: {:?}\n\nTask: {}\n\nTool schema:\n{}",
                    self.cfg.ws_root.display(),
                    self.role,
                    self.task.as_deref().unwrap_or("No task set yet."),
                    tools::tool_schema_json(allowed_tools.as_ref())
                ),
            });
        }

        messages.extend(self.chat_history.clone());
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: clean_message,
        });
        self.push_context_record(
            ContextType::UserInput,
            Some("chat_stream_input".to_string()),
            Some("user".to_string()),
            self.agent_id.clone(),
            messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default(),
            serde_json::json!({ "source": "chat_stream" }),
        );
        let summary_count = self.maybe_compact_model_messages(&mut messages, "chat_stream");
        self.emit_context_usage_event("chat_stream", &messages, summary_count)
            .await;

        let stream = self
            .model_manager
            .chat_text_stream(&self.model_id, &messages)
            .await?;

        Ok(stream)
    }

    pub async fn finalize_chat(
        &mut self,
        user_message: &str,
        assistant_response: &str,
        session_id: Option<&str>,
        mode: PromptMode,
    ) -> Result<()> {
        let mut clean_message = user_message.to_string();
        if user_message.starts_with('/') {
            let parts: Vec<&str> = user_message.splitn(2, ' ').collect();
            if parts.len() > 1 {
                clean_message = parts[1].to_string();
            } else {
                clean_message = "I'm ready to use this skill. How can I help?".to_string();
            }
        }

        self.chat_history.push(ChatMessage {
            role: "user".to_string(),
            content: clean_message.clone(),
        });
        let already_recorded_user_input = self
            .context_records
            .last()
            .map(|r| r.context_type == ContextType::UserInput && r.content == clean_message)
            .unwrap_or(false);
        if !already_recorded_user_input {
            self.push_context_record(
                ContextType::UserInput,
                Some("chat_input".to_string()),
                Some("user".to_string()),
                self.agent_id.clone(),
                clean_message.clone(),
                serde_json::json!({ "source": "finalize_chat" }),
            );
        }

        let final_content = if mode == PromptMode::Chat {
            // Strip XML-style tags like <search_indexing> from chat responses if they leak
            let mut cleaned = assistant_response.to_string();
            while let Some(start) = cleaned.find('<') {
                if let Some(end) = cleaned[start..].find('>') {
                    cleaned.replace_range(start..start + end + 1, "");
                } else {
                    break;
                }
            }
            cleaned.trim().to_string()
        } else {
            // Try to parse the response as JSON to extract a user-facing summary.
            if let Ok(action) = serde_json::from_str::<ModelAction>(assistant_response) {
                match action {
                    ModelAction::FinalizeTask { packet } => {
                        format!(
                            "I've finalized the task: {}. You can review it in the Planning section.",
                            packet.title
                        )
                    }
                    ModelAction::Tool { tool, .. } => {
                        format!(
                            "I'm using the tool: {}. I will continue automatically.",
                            tool
                        )
                    }
                    ModelAction::Patch { .. } => {
                        "I've proposed a code patch. I will apply it now.".to_string()
                    }
                }
            } else {
                // If JSON parsing fails, it might be because of leaked tags.
                // We don't strip them here because we want to see the error,
                // but for the final display we can be more lenient.
                assistant_response.to_string()
            }
        };

        // Record assistant response in DB
        if let Some(manager) = self.tools.get_manager() {
            let target = self.outbound_target();
            let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                repo_path: self.cfg.ws_root.to_string_lossy().to_string(),
                session_id: session_id.unwrap_or("default").to_string(),
                agent_id: self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                from_id: self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                to_id: target,
                content: final_content.clone(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                is_observation: false,
            });
        }

        self.chat_history.push(ChatMessage {
            role: "assistant".to_string(),
            content: final_content.clone(),
        });
        self.push_context_record(
            ContextType::AssistantReply,
            Some("chat_reply".to_string()),
            self.agent_id.clone(),
            Some("user".to_string()),
            final_content,
            serde_json::json!({ "mode": format!("{:?}", mode) }),
        );

        Ok(())
    }

    pub async fn manager_db_add_observation(
        &self,
        tool: &str,
        rendered: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        if let Some(manager) = self.tools.get_manager() {
            let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                repo_path: self.cfg.ws_root.to_string_lossy().to_string(),
                session_id: session_id.unwrap_or("default").to_string(),
                agent_id: self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                from_id: "system".to_string(),
                to_id: self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                content: format!("Tool {}: {}", tool, rendered),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                is_observation: true,
            });
        }
        Ok(())
    }

    pub async fn manager_db_add_assistant_message(
        &self,
        content: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let target = self.outbound_target();
            // Emit to UI immediately (SSE), so structured terminal messages are visible
            // even when no outer chat handler emits an explicit Outcome event.
            manager
                .send_event(crate::agent_manager::AgentEvent::Message {
                    from: agent_id.clone(),
                    to: target.clone(),
                    content: content.to_string(),
                })
                .await;
            let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                repo_path: self.cfg.ws_root.to_string_lossy().to_string(),
                session_id: session_id.unwrap_or("default").to_string(),
                agent_id: agent_id.clone(),
                from_id: agent_id.clone(),
                to_id: target,
                content: content.to_string(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                is_observation: false,
            });

            // Nudge UI to refresh immediately.
            manager
                .send_event(crate::agent_manager::AgentEvent::StateUpdated)
                .await;
        }
        Ok(())
    }

    pub async fn run_agent_loop(&mut self, session_id: Option<&str>) -> Result<AgentOutcome> {
        if self.is_cancelled().await {
            anyhow::bail!("run cancelled");
        }

        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            manager
                .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                    agent_id,
                    status: "working".to_string(),
                    detail: Some("Running".to_string()),
                })
                .await;
        }

        // Sync world state before running the loop if we have a manager
        if let Some(manager) = self.tools.get_manager() {
            let _ = manager.sync_world_state(&self.cfg.ws_root).await;
        }

        let Some(task) = self.task.clone() else {
            anyhow::bail!("no task set; use /task <text>");
        };

        info!(
            "Starting agent loop for role {:?} with task: {}",
            self.role, task
        );
        self.push_context_record(
            ContextType::Status,
            Some("autonomous_loop_start".to_string()),
            self.agent_id.clone(),
            None,
            format!("Starting autonomous loop for task: {}", task),
            serde_json::json!({ "mode": "structured" }),
        );

        // Record start of loop in DB
        if let Some(manager) = self.tools.get_manager() {
            let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                repo_path: self.cfg.ws_root.to_string_lossy().to_string(),
                session_id: session_id.unwrap_or("default").to_string(),
                agent_id: self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                from_id: "system".to_string(),
                to_id: self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                content: format!("Starting autonomous loop for task: {}", task),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                is_observation: true,
            });
        }

        let system = self.system_prompt();
        let allowed_tools = self.allowed_tool_names();
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: system,
        }];
        self.push_context_record(
            ContextType::System,
            Some("structured_loop_prompt".to_string()),
            None,
            None,
            messages[0].content.clone(),
            serde_json::json!({ "mode": "structured" }),
        );

        // Include chat history so the model has context of the current conversation.
        // IMPORTANT: We append the structured bootstrap *after* history+observations so the
        // last message is always a fresh user instruction for the current task.
        messages.extend(self.chat_history.clone());

        for obs in &self.observations {
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: Self::observation_for_model(obs),
            });
        }

        // Provide tool schema + workspace info (last user message, so the model starts the task
        // instead of continuing a prior assistant greeting from chat_history).
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: format!(
                "Autonomous agent loop started. Ignore any prior greetings or small talk.\n\nWorkspace root: {}\n\nCurrent Role: {:?}\n\nTask: {}\n\nTool schema (respond with a single JSON object per turn):\n{}",
                self.cfg.ws_root.display(),
                self.role,
                task,
                tools::tool_schema_json(allowed_tools.as_ref())
            ),
        });
        self.push_context_record(
            ContextType::UserInput,
            Some("structured_bootstrap".to_string()),
            Some("system".to_string()),
            self.agent_id.clone(),
            messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default(),
            serde_json::json!({ "source": "run_agent_loop" }),
        );

        #[derive(Clone)]
        struct CachedToolObs {
            model: String,
        }

        // Cache tool results by (tool,args) to prevent repetition loops.
        let mut tool_cache: HashMap<String, CachedToolObs> = HashMap::new();
        // Guardrails: track files read in this loop so writes to existing files are never blind.
        let mut read_paths: HashSet<String> = HashSet::new();

        // Pre-populate read_paths from prior context if we've already read files in this session.
        // IMPORTANT: Read results are stored as observations, not in chat_history.
        let mut ingest_read_file_text = |text: &str| {
            if !text.contains("Read:") || text.contains("tool_error:") {
                return;
            }
            if let Some(start) = text.find("Read: ") {
                let path_part = &text[start + 6..];
                let raw_path = path_part.split_whitespace().next().unwrap_or("");
                if raw_path.is_empty() {
                    return;
                }
                let clean_path = raw_path
                    .trim_end_matches(')')
                    .trim_end_matches(',')
                    .trim_end_matches('.')
                    .to_string();
                if clean_path.is_empty() {
                    return;
                }
                read_paths.insert(clean_path.clone());

                // Also insert normalized relative form if possible
                if let Ok(abs) = self.cfg.ws_root.join(&clean_path).canonicalize() {
                    if let Ok(rel) = abs.strip_prefix(&self.cfg.ws_root) {
                        read_paths.insert(rel.to_string_lossy().to_string());
                    }
                }
            }
        };

        for obs in &self.observations {
            if obs.name == "Read" {
                ingest_read_file_text(&obs.content);
            }
        }
        for msg in &self.chat_history {
            ingest_read_file_text(&msg.content);
        }

        // Guardrail: repeated search with no matches means no progress.
        let mut empty_search_streak = 0usize;
        // Guardrail: repeated get_repo_info or other redundant calls.
        let mut redundant_tool_streak = 0usize;
        let mut last_tool_sig = String::new();
        // Guardrail: malformed action JSON can cause endless retries.
        let mut invalid_json_streak = 0usize;
        // Guardrail: identical assistant responses (looping).
        let mut last_assistant_response = String::new();
        let mut identical_response_streak = 0usize;
        let mut loop_nudge_count = 0usize;

        for _ in 0..self.cfg.max_iters {
            if self.is_cancelled().await {
                anyhow::bail!("run cancelled");
            }

            if let Some(manager) = self.tools.get_manager() {
                let agent_id = self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                manager
                    .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                        agent_id,
                        status: "thinking".to_string(),
                        detail: Some("Thinking".to_string()),
                    })
                    .await;
            }

            let summary_count = self.maybe_compact_model_messages(&mut messages, "loop_iter");
            self.emit_context_usage_event("loop_iter", &messages, summary_count)
                .await;

            // Ask model for the next action as JSON.
            let raw = self
                .model_manager
                .chat_json(&self.model_id, &messages)
                .await?;

            // Debug log: split model output into text + json (truncated).
            let (text_part, json_part) = crate::engine::model_message_log_parts(&raw, 100, 100);
            let json_rendered = json_part
                .as_ref()
                .and_then(|v| serde_json::to_string(v).ok())
                .unwrap_or_else(|| "null".to_string());
            info!(
                "Model response split: text='{}' json={}",
                text_part.replace('\n', "\\n"),
                json_rendered
            );

            // Repetition check
            if raw == last_assistant_response {
                identical_response_streak += 1;
            } else {
                identical_response_streak = 0;
                last_assistant_response = raw.clone();
            }

            if identical_response_streak >= 3 {
                loop_nudge_count += 1;
                if loop_nudge_count >= 2 {
                    let message = format!(
                        "I couldn't continue automatically because I got stuck in a repetition loop (same response {} times).",
                        identical_response_streak + 1
                    );
                    let _ = self
                        .manager_db_add_assistant_message(&message, session_id)
                        .await;
                    self.active_skill = None;
                    return Ok(AgentOutcome::None);
                }

                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: "It looks like you are trapped in a loop, sending the same response multiple times. Please try a different approach or tool to make progress.".to_string(),
                });
                self.push_context_record(
                    ContextType::Error,
                    Some("loop_detected".to_string()),
                    self.agent_id.clone(),
                    None,
                    "Model trapped in a loop. Nudging with a warning message.".to_string(),
                    serde_json::json!({ "streak": identical_response_streak + 1 }),
                );
                // Reset streak after nudging so we don't immediately trigger it again next turn
                // but keep last_assistant_response to detect if it continues after the nudge.
                identical_response_streak = 0;
                continue;
            }

            let action: ModelAction = match parse_first_action(&raw) {
                Ok(v) => v,
                Err(e) => {
                    invalid_json_streak += 1;
                    if invalid_json_streak >= 4 {
                        let message = "I couldn't continue automatically because the model kept returning malformed structured output.".to_string();
                        let _ = self
                            .manager_db_add_assistant_message(&message, session_id)
                            .await;
                        self.active_skill = None;
                        return Ok(AgentOutcome::None);
                    }
                    messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: format!(
                            "Your previous response was not valid JSON ({e}). Respond again with ONE JSON object matching the tool schema. Raw was:\n{raw}"
                        ),
                    });
                    self.push_context_record(
                        ContextType::Error,
                        Some("invalid_json".to_string()),
                        self.agent_id.clone(),
                        None,
                        format!("invalid_json: {}", e),
                        serde_json::json!({ "raw": raw }),
                    );
                    continue;
                }
            };
            invalid_json_streak = 0;

            match action {
                ModelAction::Tool { tool, args } => {
                    let canonical_tool = tools::canonical_tool_name(&tool)
                        .unwrap_or(tool.as_str())
                        .to_string();

                    if let Some(allowed) = &allowed_tools {
                        if !self.is_tool_allowed(allowed, &tool) {
                            let mut allowed_list = allowed.iter().cloned().collect::<Vec<_>>();
                            allowed_list.sort();
                            let rendered = format!(
                                "tool_not_allowed: tool={} canonical={} allowed={}",
                                tool,
                                canonical_tool,
                                allowed_list.join(",")
                            );
                            self.upsert_observation("error", &canonical_tool, rendered.clone());
                            let _ = self
                                .manager_db_add_observation(&canonical_tool, &rendered, session_id)
                                .await;
                            messages.push(ChatMessage {
                                role: "user".to_string(),
                                content: format!(
                                    "Tool '{}' is not allowed for this agent. Use one of [{}].",
                                    tool,
                                    allowed_list.join(", ")
                                ),
                            });
                            continue;
                        }
                    }

                    let safe_args = sanitize_tool_args_for_display(&canonical_tool, &args);
                    self.upsert_context_record_by_type_name(
                        ContextType::ToolCall,
                        &canonical_tool,
                        self.agent_id.clone(),
                        Some(self.outbound_target()),
                        serde_json::to_string(&safe_args).unwrap_or_else(|_| "{}".to_string()),
                        serde_json::json!({ "args": safe_args.clone() }),
                    );
                    info!(
                        "Agent requested tool: {} (requested: {}) with args: {}",
                        canonical_tool, tool, safe_args
                    );
                    if canonical_tool == "Read" {
                        if let Some(path) = normalize_tool_path_arg(&self.cfg.ws_root, &args) {
                            read_paths.insert(path);
                        }
                    }

                    if canonical_tool == "Write" {
                        if let Some(path) = normalize_tool_path_arg(&self.cfg.ws_root, &args) {
                            let existing = self.cfg.ws_root.join(&path).exists();
                            if existing && !read_paths.contains(&path) {
                                match self.cfg.write_safety_mode {
                                    crate::config::WriteSafetyMode::Strict => {
                                        let rendered = format!(
                                            "tool_error: tool=Write error=precondition_failed: must call Read on '{}' before Write for existing files",
                                            path
                                        );
                                        self.upsert_observation("error", "Write", rendered.clone());
                                        let _ = self
                                            .manager_db_add_observation(
                                                "Write", &rendered, session_id,
                                            )
                                            .await;
                                        messages.push(ChatMessage {
                                            role: "user".to_string(),
                                            content: format!(
                                                "Tool execution blocked for safety: {}. Read the existing file first, then write a minimal update.",
                                                rendered
                                            ),
                                        });
                                        continue;
                                    }
                                    crate::config::WriteSafetyMode::Warn => {
                                        let rendered = format!(
                                            "tool_warning: tool=Write warning=writing_existing_file_without_prior_read path='{}'",
                                            path
                                        );
                                        self.upsert_observation(
                                            "warning",
                                            "Write",
                                            rendered.clone(),
                                        );
                                        let _ = self
                                            .manager_db_add_observation(
                                                "Write", &rendered, session_id,
                                            )
                                            .await;
                                        // Allow the write to proceed.
                                    }
                                    crate::config::WriteSafetyMode::Off => {
                                        // Allow writes without read.
                                    }
                                }
                            }
                        }
                    }

                    let sig = tool_call_signature(&canonical_tool, &args);
                    if sig == last_tool_sig {
                        redundant_tool_streak += 1;
                    } else {
                        redundant_tool_streak = 0;
                        last_tool_sig = sig.clone();
                    }

                    if redundant_tool_streak >= 3 {
                        let loop_breaker_prompt = self
                            .cfg
                            .prompt_loop_breaker
                            .as_deref()
                            .map(|template| Self::render_loop_breaker_prompt(template, &canonical_tool))
                            .unwrap_or_else(|| {
                                format!(
                                    "You are repeatedly calling '{}' with the same arguments and not making progress. Use a different tool/arguments and continue automatically.",
                                    canonical_tool
                                )
                            });
                        messages.push(ChatMessage {
                            role: "user".to_string(),
                            content: loop_breaker_prompt,
                        });
                        self.push_context_record(
                            ContextType::Error,
                            Some("redundant_tool_loop".to_string()),
                            self.agent_id.clone(),
                            None,
                            format!(
                                "Repeated tool call loop detected for '{}'; nudging model to change approach.",
                                canonical_tool
                            ),
                            serde_json::json!({ "tool": canonical_tool, "streak": redundant_tool_streak + 1 }),
                        );
                        redundant_tool_streak = 0;
                        continue;
                    }

                    if let Some(cached) = tool_cache.get(&sig) {
                        self.upsert_observation("tool", &canonical_tool, cached.model.clone());
                        // Cached tool call: keep context for the model, but don't keep re-logging
                        // identical observation rows to DB/UI.
                        messages.push(ChatMessage {
                            role: "user".to_string(),
                            content: Self::observation_text("tool", &canonical_tool, &cached.model),
                        });
                        // Don't re-run identical tool calls.
                        continue;
                    }

                    // Tell the UI what tool we're about to use (progress visibility).
                    if let Some(manager) = self.tools.get_manager() {
                        let from = self
                            .agent_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());
                        let target = self.outbound_target();
                        let _ = manager
                            .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                                agent_id: from.clone(),
                                status: "calling_tool".to_string(),
                                detail: Some(format!("Calling {}", canonical_tool)),
                            })
                            .await;
                        let _ = manager
                            .send_event(crate::agent_manager::AgentEvent::Message {
                                from: from.clone(),
                                to: target.clone(),
                                content: serde_json::json!({
                                    "type": "tool",
                                    "tool": canonical_tool.clone(),
                                    "args": safe_args.clone()
                                })
                                .to_string(),
                            })
                            .await;
                        let tool_msg = serde_json::json!({
                            "type": "tool",
                            "tool": canonical_tool.clone(),
                            "args": safe_args
                        })
                        .to_string();
                        let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                            repo_path: self.cfg.ws_root.to_string_lossy().to_string(),
                            session_id: session_id.unwrap_or("default").to_string(),
                            agent_id: from.clone(),
                            from_id: from,
                            to_id: target,
                            content: tool_msg,
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            is_observation: false,
                        });
                    }

                    let call = ToolCall {
                        tool: canonical_tool.clone(),
                        args: args.clone(),
                    };
                    match self.tools.execute(call) {
                        Ok(result) => {
                            let rendered_model = render_tool_result(&result);
                            let rendered_public = render_tool_result_public(&result);

                            tool_cache.insert(
                                sig,
                                CachedToolObs {
                                    model: rendered_model.clone(),
                                },
                            );
                            self.upsert_observation(
                                "tool",
                                &canonical_tool,
                                rendered_model.clone(),
                            );

                            // Record observation in DB
                            let _ = self
                                .manager_db_add_observation(
                                    &canonical_tool,
                                    &rendered_public,
                                    session_id,
                                )
                                .await;
                            if let Some(manager) = self.tools.get_manager() {
                                // Trigger a UI refresh via the server's SSE bridge.
                                manager
                                    .send_event(crate::agent_manager::AgentEvent::StateUpdated)
                                    .await;

                                // After tool completes, we're back to thinking.
                                let agent_id = self
                                    .agent_id
                                    .clone()
                                    .unwrap_or_else(|| "unknown".to_string());
                                manager
                                    .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                                        agent_id,
                                        status: "thinking".to_string(),
                                        detail: Some("Thinking".to_string()),
                                    })
                                    .await;
                            }

                            // For writes, emit a brief user-visible summary line immediately.
                            if canonical_tool == "Write"
                                && (rendered_public.starts_with("File written:")
                                    || rendered_public.starts_with("File unchanged"))
                            {
                                let msg = if rendered_public.starts_with("File unchanged") {
                                    if let Some(idx) = rendered_public.rfind(':') {
                                        let path = rendered_public[idx + 1..].trim();
                                        format!("No changes to `{}` (content identical).", path)
                                    } else {
                                        "No file changes (content identical).".to_string()
                                    }
                                } else if let Some(idx) = rendered_public.rfind(':') {
                                    let rest = rendered_public[idx + 1..].trim();
                                    let path = rest.split_whitespace().next().unwrap_or(rest);
                                    format!("Updated `{}`.", path)
                                } else {
                                    "File updated.".to_string()
                                };
                                let _ = self
                                    .manager_db_add_assistant_message(&msg, session_id)
                                    .await;
                            }

                            messages.push(ChatMessage {
                                role: "user".to_string(),
                                content: Self::observation_text(
                                    "tool",
                                    &canonical_tool,
                                    &rendered_model,
                                ),
                            });

                            if canonical_tool == "Grep"
                                && (rendered_model.contains("(no matches)")
                                    || rendered_model.contains("no file candidates found"))
                            {
                                empty_search_streak += 1;
                            } else {
                                empty_search_streak = 0;
                            }
                            if empty_search_streak >= 4 {
                                messages.push(ChatMessage {
                                    role: "user".to_string(),
                                    content: "Grep returned no matches repeatedly. Change strategy and continue automatically (for example: broaden terms, use Glob to inspect files, then Read likely paths).".to_string(),
                                });
                                self.push_context_record(
                                    ContextType::Error,
                                    Some("empty_search_loop".to_string()),
                                    self.agent_id.clone(),
                                    None,
                                    "Repeated no-match search loop detected; nudging model to change strategy.".to_string(),
                                    serde_json::json!({ "streak": empty_search_streak }),
                                );
                                empty_search_streak = 0;
                            }
                        }
                        Err(e) => {
                            warn!("Tool execution failed ({}): {}", canonical_tool, e);
                            let rendered =
                                format!("tool_error: tool={} error={}", canonical_tool, e);
                            tool_cache.insert(
                                sig,
                                CachedToolObs {
                                    model: rendered.clone(),
                                },
                            );
                            self.upsert_observation("error", &canonical_tool, rendered.clone());
                            let _ = self
                                .manager_db_add_observation(&canonical_tool, &rendered, session_id)
                                .await;
                            if let Some(manager) = self.tools.get_manager() {
                                let agent_id = self
                                    .agent_id
                                    .clone()
                                    .unwrap_or_else(|| "unknown".to_string());
                                manager
                                    .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                                        agent_id,
                                        status: "thinking".to_string(),
                                        detail: Some("Thinking".to_string()),
                                    })
                                    .await;
                            }
                            messages.push(ChatMessage {
                                role: "user".to_string(),
                                content: format!(
                                    "Tool execution failed for tool='{}'. Error: {}. Choose a valid tool+args from the tool schema and try again.",
                                    canonical_tool, e
                                ),
                            });
                        }
                    }
                }
                ModelAction::Patch { diff } => {
                    info!("Agent proposed a patch");
                    if !self.agent_allows_policy(AgentPolicyCapability::Patch) {
                        warn!("Agent tried to propose a patch without Patch policy");
                        self.push_context_record(
                            ContextType::Error,
                            Some("patch_not_allowed".to_string()),
                            self.agent_id.clone(),
                            None,
                            "Agent policy does not allow Patch.".to_string(),
                            serde_json::json!({
                                "required_policy": "Patch",
                                "agent": self.agent_id.clone(),
                            }),
                        );
                        messages.push(ChatMessage {
                            role: "user".to_string(),
                            content: "Error: This agent is not allowed to output 'patch'. Add `Patch` to the agent frontmatter `policy` to enable it.".to_string(),
                        });
                        continue;
                    }
                    let errs = validate_unified_diff(&diff);
                    if !errs.is_empty() {
                        warn!("Patch validation failed with {} errors", errs.len());
                        self.push_context_record(
                            ContextType::Error,
                            Some("patch_validation".to_string()),
                            self.agent_id.clone(),
                            None,
                            errs.join("\n"),
                            serde_json::json!({ "error_count": errs.len() }),
                        );
                        messages.push(ChatMessage {
                            role: "user".to_string(),
                            content: format!(
                                "The patch failed validation. Fix and respond with a new patch JSON. Errors:\n{}",
                                errs.join("\n")
                            ),
                        });
                        continue;
                    }

                    info!("Patch validated successfully");

                    // Record activity in DB for patched files
                    if let Some(manager) = self.tools.get_manager() {
                        if let Some(agent_id) = &self.agent_id {
                            // Simple parsing of diff to find files
                            for line in diff.lines() {
                                if line.starts_with("--- ") || line.starts_with("+++ ") {
                                    let path = line[4..].split_whitespace().next().unwrap_or("");
                                    if path != "/dev/null" && !path.is_empty() {
                                        let _ =
                                            manager.db.record_activity(crate::db::FileActivity {
                                                repo_path: self
                                                    .cfg
                                                    .ws_root
                                                    .to_string_lossy()
                                                    .to_string(),
                                                file_path: path.to_string(),
                                                agent_id: agent_id.clone(),
                                                status: "done".to_string(),
                                                last_modified: std::time::SystemTime::now()
                                                    .duration_since(std::time::UNIX_EPOCH)
                                                    .unwrap()
                                                    .as_secs(),
                                            });
                                    }
                                }
                            }
                        }
                    }

                    self.active_skill = None;
                    return Ok(AgentOutcome::Patch(diff));
                }
                ModelAction::FinalizeTask { packet } => {
                    info!("Agent finalized task: {}", packet.title);
                    if !self.agent_allows_policy(AgentPolicyCapability::Finalize) {
                        warn!("Agent tried to finalize task without Finalize policy");
                        self.push_context_record(
                            ContextType::Error,
                            Some("finalize_not_allowed".to_string()),
                            self.agent_id.clone(),
                            None,
                            "Agent policy does not allow Finalize.".to_string(),
                            serde_json::json!({
                                "required_policy": "Finalize",
                                "agent": self.agent_id.clone(),
                            }),
                        );
                        messages.push(ChatMessage {
                            role: "user".to_string(),
                            content: "Error: This agent is not allowed to output 'finalize_task'. Add `Finalize` to the agent frontmatter `policy` to enable it.".to_string(),
                        });
                        continue;
                    }
                    // Persist the structured final answer for the UI (DB-backed chat).
                    let msg = serde_json::json!({ "type": "finalize_task", "packet": packet })
                        .to_string();
                    let _ = self
                        .manager_db_add_assistant_message(&msg, session_id)
                        .await;
                    self.chat_history.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: msg.clone(),
                    });
                    self.push_context_record(
                        ContextType::AssistantReply,
                        Some("finalize_task".to_string()),
                        self.agent_id.clone(),
                        Some("user".to_string()),
                        msg,
                        serde_json::json!({ "kind": "finalize_task" }),
                    );
                    self.active_skill = None;
                    return Ok(AgentOutcome::Task(packet));
                }
            }
        }

        self.active_skill = None;
        let fallback = "I couldn't complete this automatically within the current tool loop limit. Please refine the request or switch to `/mode chat` for a direct answer."
            .to_string();
        self.push_context_record(
            ContextType::Status,
            Some("loop_limit_reached".to_string()),
            self.agent_id.clone(),
            Some("user".to_string()),
            fallback.clone(),
            serde_json::json!({ "max_iters": self.cfg.max_iters }),
        );
        let _ = self
            .manager_db_add_assistant_message(&fallback, session_id)
            .await;
        Ok(AgentOutcome::None)
    }

    async fn is_cancelled(&self) -> bool {
        let Some(run_id) = &self.run_id else {
            return false;
        };
        let Some(manager) = self.tools.get_manager() else {
            return false;
        };
        manager.is_run_cancelled(run_id).await
    }

    fn outbound_target(&self) -> String {
        let kind = self
            .spec
            .as_ref()
            .map(|s| s.kind)
            .unwrap_or(AgentKind::Main);

        if kind == AgentKind::Subagent {
            return self
                .parent_agent_id
                .clone()
                .unwrap_or_else(|| "user".to_string());
        }

        "user".to_string()
    }

    fn now_ts_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn estimate_tokens_for_text(text: &str) -> usize {
        let chars = text.chars().count();
        if chars == 0 {
            0
        } else {
            (chars + 3) / 4
        }
    }

    fn estimate_chars_for_messages(messages: &[ChatMessage]) -> usize {
        messages.iter().map(|m| m.content.chars().count()).sum()
    }

    fn estimate_tokens_for_messages(messages: &[ChatMessage]) -> usize {
        messages
            .iter()
            .map(|m| Self::estimate_tokens_for_text(&m.content))
            .sum()
    }

    fn summarize_message_window(window: &[ChatMessage]) -> String {
        let mut user_count = 0usize;
        let mut assistant_count = 0usize;
        let mut system_count = 0usize;
        for msg in window {
            match msg.role.as_str() {
                "user" => user_count += 1,
                "assistant" => assistant_count += 1,
                "system" => system_count += 1,
                _ => {}
            }
        }

        let highlights = window
            .iter()
            .rev()
            .filter_map(|msg| {
                let snippet = msg
                    .content
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())?;
                let mut short = snippet.to_string();
                if short.chars().count() > 140 {
                    short = short.chars().take(140).collect::<String>() + "...";
                }
                Some(format!("{}: {}", msg.role, short))
            })
            .take(5)
            .collect::<Vec<_>>();

        let mut summary = format!(
            "Context summary (compressed older messages).\nCounts: user={}, assistant={}, system={}.",
            user_count, assistant_count, system_count
        );
        if !highlights.is_empty() {
            summary.push_str("\nRecent highlights:");
            for h in highlights.into_iter().rev() {
                summary.push_str("\n- ");
                summary.push_str(&h);
            }
        }
        summary
    }

    fn maybe_compact_model_messages(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        stage: &str,
    ) -> usize {
        let mut summary_count = 0usize;

        loop {
            let token_est = Self::estimate_tokens_for_messages(messages);
            let over_budget =
                token_est > CONTEXT_SOFT_TOKEN_LIMIT || messages.len() > CONTEXT_SOFT_MESSAGE_LIMIT;
            if !over_budget || summary_count >= CONTEXT_MAX_SUMMARY_PASSES {
                break;
            }

            if messages.len() <= CONTEXT_KEEP_TAIL_MESSAGES + 2 {
                break;
            }

            let start = 1usize; // Keep the leading system prompt.
            let end = messages.len().saturating_sub(CONTEXT_KEEP_TAIL_MESSAGES);
            if end <= start {
                break;
            }

            let window = messages[start..end].to_vec();
            let dropped_messages = window.len();
            let dropped_chars: usize = window.iter().map(|m| m.content.chars().count()).sum();
            let dropped_tokens = Self::estimate_tokens_for_messages(&window);
            let summary = Self::summarize_message_window(&window);

            messages.drain(start..end);
            messages.insert(
                start,
                ChatMessage {
                    role: "user".to_string(),
                    content: summary.clone(),
                },
            );

            summary_count += 1;
            self.push_context_record(
                ContextType::Summary,
                Some(format!("{}_summary_{}", stage, summary_count)),
                Some("system".to_string()),
                self.agent_id.clone(),
                summary,
                serde_json::json!({
                    "stage": stage,
                    "dropped_messages": dropped_messages,
                    "dropped_chars": dropped_chars,
                    "dropped_estimated_tokens": dropped_tokens
                }),
            );
        }

        summary_count
    }

    async fn emit_context_usage_event(
        &self,
        stage: &str,
        messages: &[ChatMessage],
        summary_count: usize,
    ) {
        let Some(manager) = self.tools.get_manager() else {
            return;
        };
        let token_limit = self
            .model_manager
            .context_window(&self.model_id)
            .await
            .ok()
            .flatten();
        let _ = manager
            .send_event(crate::agent_manager::AgentEvent::ContextUsage {
                agent_id: self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                stage: stage.to_string(),
                message_count: messages.len(),
                char_count: Self::estimate_chars_for_messages(messages),
                estimated_tokens: Self::estimate_tokens_for_messages(messages),
                token_limit,
                compressed: summary_count > 0,
                summary_count,
            })
            .await;
    }

    fn push_context_record(
        &mut self,
        context_type: ContextType,
        name: Option<String>,
        from: Option<String>,
        to: Option<String>,
        content: String,
        meta: JsonValue,
    ) {
        let rec = ContextRecord {
            id: self.next_context_id,
            ts: Self::now_ts_secs(),
            context_type,
            name,
            from,
            to,
            content,
            meta,
        };
        self.next_context_id = self.next_context_id.saturating_add(1);
        self.context_records.push(rec);
    }

    fn upsert_context_record_by_type_name(
        &mut self,
        context_type: ContextType,
        name: &str,
        from: Option<String>,
        to: Option<String>,
        content: String,
        meta: JsonValue,
    ) {
        self.context_records.retain(|existing| {
            if existing.context_type != context_type {
                return true;
            }
            if let Some(existing_name) = &existing.name {
                !existing_name.eq_ignore_ascii_case(name)
            } else {
                true
            }
        });
        self.push_context_record(
            context_type,
            Some(name.to_string()),
            from,
            to,
            content,
            meta,
        );
    }

    fn observation_text(observation_type: &str, name: &str, content: &str) -> String {
        format!(
            "Observation:\ntype: {}\nname: {}\ncontent:\n{}",
            observation_type, name, content
        )
    }

    fn observation_for_model(obs: &ObservationRecord) -> String {
        Self::observation_text(&obs.observation_type, &obs.name, &obs.content)
    }

    fn render_loop_breaker_prompt(template: &str, tool: &str) -> String {
        let mut rendered = template.replace("{tool}", tool);
        if rendered.contains("{}") {
            rendered = rendered.replacen("{}", tool, 1);
        }
        rendered
    }

    pub(crate) fn upsert_observation(
        &mut self,
        observation_type: &str,
        name: &str,
        content: String,
    ) {
        let context_type = if observation_type.eq_ignore_ascii_case("tool") {
            ContextType::ToolResult
        } else if observation_type.eq_ignore_ascii_case("error") {
            ContextType::Error
        } else if observation_type.eq_ignore_ascii_case("status") {
            ContextType::Status
        } else if observation_type.eq_ignore_ascii_case("summary") {
            ContextType::Summary
        } else {
            ContextType::Observation
        };
        self.upsert_context_record_by_type_name(
            context_type,
            name,
            Some("system".to_string()),
            self.agent_id.clone(),
            content.clone(),
            serde_json::json!({ "observation_type": observation_type }),
        );
        self.observations.retain(|existing| {
            !(existing
                .observation_type
                .eq_ignore_ascii_case(observation_type)
                && existing.name.eq_ignore_ascii_case(name))
        });
        self.observations.push(ObservationRecord {
            observation_type: observation_type.to_string(),
            name: name.to_string(),
            content,
        });
    }

    fn allowed_tool_names(&self) -> Option<HashSet<String>> {
        let spec = self.spec.as_ref()?;
        if spec.tools.is_empty() {
            return None;
        }
        // Wildcard means unrestricted tool access for this agent.
        if spec.tools.iter().any(|tool| tool.trim() == "*") {
            return None;
        }

        let allowed = spec
            .tools
            .iter()
            .filter_map(|tool| tools::canonical_tool_name(tool).map(str::to_string))
            .collect::<HashSet<String>>();

        Some(allowed)
    }

    fn is_tool_allowed(&self, allowed: &HashSet<String>, requested_tool: &str) -> bool {
        tools::canonical_tool_name(requested_tool)
            .map(|tool| allowed.contains(tool))
            .unwrap_or(false)
    }

    fn agent_allows_policy(&self, capability: AgentPolicyCapability) -> bool {
        self.spec
            .as_ref()
            .map(|spec| spec.allows_policy(capability))
            .unwrap_or(false)
    }

    fn system_prompt(&self) -> String {
        let mut prompt = self
            .spec_system_prompt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "You are a helpful AI assistant.".to_string());

        if let Some(skill) = &self.active_skill {
            prompt.push_str("\n\n--- ACTIVE SKILL ---");
            prompt.push_str(&format!(
                "\nSkill: {}\nDescription: {}",
                skill.name, skill.description
            ));
            prompt.push_str(&format!("\n\n{}", skill.content));
            prompt.push_str("\n-------------------");
        }

        prompt
    }
}
