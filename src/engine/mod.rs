pub mod patch;
pub mod tools;

use crate::agent_manager::models::ModelManager;
use crate::agent_manager::AgentManager;
use crate::config::AgentSpec;
use crate::engine::patch::validate_unified_diff;
use crate::engine::tools::{ToolCall, ToolResult, Tools};
use crate::ollama::ChatMessage;
use crate::skills::Skill;
use anyhow::Result;
use futures_util::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::de::Deserializer;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

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
    pub agent_id: Option<String>,
    // Rolling tool observations that we feed back to the model.
    pub observations: Vec<String>,
    // Conversational history for chat.
    pub chat_history: Vec<ChatMessage>,
    // Active skill if any
    pub active_skill: Option<Skill>,
    pub prompt_mode: PromptMode,
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
    #[serde(rename = "ask")]
    Ask(String),
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
            agent_id: None,
            observations: Vec::new(),
            chat_history: Vec::new(),
            active_skill: None,
            prompt_mode: PromptMode::Structured,
        })
    }

    pub fn set_spec(&mut self, agent_id: String, spec: AgentSpec) {
        self.agent_id = Some(agent_id);
        self.spec = Some(spec);
    }

    pub fn get_spec(&self) -> Option<&AgentSpec> {
        self.spec.as_ref()
    }

    pub fn set_manager_context(&mut self, manager: Arc<AgentManager>) {
        if let Some(agent_id) = &self.agent_id {
            self.tools.set_context(manager, agent_id.clone());
        }
    }

    pub fn set_role(&mut self, role: AgentRole) {
        self.role = role;
        self.observations.clear();
        self.chat_history.clear();
    }

    pub fn set_task(&mut self, task: String) {
        self.task = Some(task);
        self.observations.clear();
        self.chat_history.clear();
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
        info!(
            "Processing chat stream for role {:?}: {}",
            self.role, message
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
            content: self.system_prompt_with_mode(mode),
        }];

        // Add workspace context to the first message if history is short
        if self.chat_history.len() == 0 {
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "Workspace root: {}\nCurrent Role: {:?}\n\nTask: {}\n\nTool schema:\n{}",
                    self.cfg.ws_root.display(),
                    self.role,
                    self.task.as_deref().unwrap_or("No task set yet."),
                    tools::tool_schema_json()
                ),
            });
        }

        messages.extend(self.chat_history.clone());
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: clean_message,
        });

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
            content: clean_message,
        });

        let final_content = if mode == PromptMode::Chat {
            assistant_response.to_string()
        } else {
            // Try to parse the response as JSON to extract the question if the model followed the system prompt
            if let Ok(action) = serde_json::from_str::<ModelAction>(assistant_response) {
                match action {
                    ModelAction::Ask { question } => question,
                    ModelAction::FinalizeTask { packet } => {
                        format!(
                            "I've finalized the task: {}. You can review it in the Planning section.",
                            packet.title
                        )
                    }
                    ModelAction::Tool { tool, .. } => {
                        format!("I'm using the tool: {}. I will continue automatically.", tool)
                    }
                    ModelAction::Patch { .. } => {
                        "I've proposed a code patch. I will apply it now.".to_string()
                    }
                }
            } else {
                assistant_response.to_string()
            }
        };

        // Record assistant response in DB
        if let Some(manager) = self.tools.get_manager() {
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
                to_id: "user".to_string(),
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
            content: final_content,
        });

        Ok(())
    }

    pub async fn chat(&mut self, _message: &str, _session_id: Option<&str>) -> Result<String> {
        anyhow::bail!("chat() is deprecated, use chat_stream()")
    }

    pub async fn manager_db_add_observation(&self, tool: &str, rendered: &str, session_id: Option<&str>) -> Result<()> {
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
            let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                repo_path: self.cfg.ws_root.to_string_lossy().to_string(),
                session_id: session_id.unwrap_or("default").to_string(),
                agent_id: agent_id.clone(),
                from_id: agent_id.clone(),
                to_id: "user".to_string(),
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
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: system,
        }];

        // Provide tool schema + workspace info.
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: format!(
                "Workspace root: {}\n\nCurrent Role: {:?}\n\nTask: {}\n\nTool schema (respond with a single JSON object per turn):\n{}",
                self.cfg.ws_root.display(),
                self.role,
                task,
                tools::tool_schema_json()
            ),
        });

        // Include chat history so the model has context of the current conversation
        messages.extend(self.chat_history.clone());

        for obs in &self.observations {
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: format!("Observation:\n{}", obs),
            });
        }

        #[derive(Clone)]
        struct CachedToolObs {
            model: String,
            public: String,
        }

        // Cache tool results by (tool,args) to prevent repetition loops.
        let mut tool_cache: HashMap<String, CachedToolObs> = HashMap::new();

        for _ in 0..self.cfg.max_iters {
            // Ask model for the next action as JSON.
            let raw = self
                .model_manager
                .chat_json(&self.model_id, &messages)
                .await?;

            let action: ModelAction = match parse_first_action(&raw) {
                Ok(v) => v,
                Err(e) => {
                    messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: format!(
                            "Your previous response was not valid JSON ({e}). Respond again with ONE JSON object matching the tool schema. Raw was:\n{raw}"
                        ),
                    });
                    continue;
                }
            };

            match action {
                ModelAction::Tool { tool, args } => {
                    info!("Agent requested tool: {} with args: {}", tool, args);
                    let sig = format!("{}|{}", tool, args);
                    if let Some(cached) = tool_cache.get(&sig) {
                        self.observations.push(cached.model.clone());
                        let _ = self
                            .manager_db_add_observation(&tool, &cached.public, session_id)
                            .await;
                        if let Some(manager) = self.tools.get_manager() {
                            manager
                                .send_event(crate::agent_manager::AgentEvent::StateUpdated)
                                .await;
                        }
                        messages.push(ChatMessage {
                            role: "user".to_string(),
                            content: format!("Observation:\n{}", cached.model),
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
                        let _ = manager
                            .send_event(crate::agent_manager::AgentEvent::Message {
                                from: from.clone(),
                                to: "user".to_string(),
                                content: serde_json::json!({
                                    "type": "tool",
                                    "tool": tool.clone(),
                                    "args": args.clone()
                                })
                                .to_string(),
                            })
                            .await;
                        let tool_msg = serde_json::json!({
                            "type": "tool",
                            "tool": tool.clone(),
                            "args": args.clone()
                        })
                        .to_string();
                        let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                            repo_path: self.cfg.ws_root.to_string_lossy().to_string(),
                            session_id: session_id.unwrap_or("default").to_string(),
                            agent_id: from.clone(),
                            from_id: from,
                            to_id: "user".to_string(),
                            content: tool_msg,
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            is_observation: false,
                        });
                    }

                    let call = ToolCall {
                        tool: tool.clone(),
                        args,
                    };
                    match self.tools.execute(call) {
                        Ok(result) => {
                            let rendered_model = render_tool_result(&result);
                            let rendered_public = render_tool_result_public(&result);
                            tool_cache.insert(
                                sig,
                                CachedToolObs {
                                    model: rendered_model.clone(),
                                    public: rendered_public.clone(),
                                },
                            );
                            self.observations.push(rendered_model.clone());

                            // Record observation in DB
                            let _ = self
                                .manager_db_add_observation(&tool, &rendered_public, session_id)
                                .await;
                            if let Some(manager) = self.tools.get_manager() {
                                // Trigger a UI refresh via the server's SSE bridge.
                                manager
                                    .send_event(crate::agent_manager::AgentEvent::StateUpdated)
                                    .await;
                            }

                            messages.push(ChatMessage {
                                role: "user".to_string(),
                                content: format!("Observation:\n{}", rendered_model),
                            });
                        }
                        Err(e) => {
                            warn!("Tool execution failed ({}): {}", tool, e);
                            let rendered = format!("tool_error: tool={} error={}", tool, e);
                            tool_cache.insert(
                                sig,
                                CachedToolObs {
                                    model: rendered.clone(),
                                    public: rendered.clone(),
                                },
                            );
                            self.observations.push(rendered.clone());
                            let _ = self.manager_db_add_observation(&tool, &rendered, session_id).await;
                            messages.push(ChatMessage {
                                role: "user".to_string(),
                                content: format!(
                                    "Tool execution failed for tool='{tool}'. Error: {e}. Choose a valid tool+args from the tool schema and try again."
                                ),
                            });
                        }
                    }
                }
                ModelAction::Patch { diff } => {
                    info!("Agent proposed a patch");
                    if self.role != AgentRole::Coder {
                        warn!(
                            "Agent tried to propose a patch while in role {:?}",
                            self.role
                        );
                        messages.push(ChatMessage {
                            role: "user".to_string(),
                            content: "Error: Only the 'coder' role can propose patches. You are currently in the PM role. Use 'finalize_task' to finish planning.".to_string(),
                        });
                        continue;
                    }
                    let errs = validate_unified_diff(&diff);
                    if !errs.is_empty() {
                        warn!("Patch validation failed with {} errors", errs.len());
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
                    if self.role != AgentRole::Lead {
                        warn!("Agent tried to finalize task while in role {:?}", self.role);
                        messages.push(ChatMessage {
                            role: "user".to_string(),
                            content: "Error: Only the 'lead' role can finalize tasks.".to_string(),
                        });
                        continue;
                    }
                    // Persist the structured final answer for the UI (DB-backed chat).
                    let msg = serde_json::json!({ "type": "finalize_task", "packet": packet }).to_string();
                    let _ = self
                        .manager_db_add_assistant_message(&msg, session_id)
                        .await;
                    self.chat_history.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: msg,
                    });
                    self.active_skill = None;
                    return Ok(AgentOutcome::Task(packet));
                }
                ModelAction::Ask { question } => {
                    info!("Agent asked a question: {}", question);
                    let msg = serde_json::json!({ "type": "ask", "question": question }).to_string();
                    let _ = self
                        .manager_db_add_assistant_message(&msg, session_id)
                        .await;
                    self.chat_history.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: msg,
                    });
                    self.active_skill = None;
                    return Ok(AgentOutcome::Ask(question));
                }
            }
        }

        self.active_skill = None;
        Ok(AgentOutcome::None)
    }

    fn system_prompt(&self) -> String {
        self.system_prompt_with_mode(PromptMode::Structured)
    }

    fn system_prompt_with_mode(&self, mode: PromptMode) -> String {
        let mut prompt = if let Some(spec) = &self.spec {
            // Use the spec-defined system prompt if available
            let base = match self.role {
                AgentRole::Lead => [
                    "You are linggen-agent 'Lead'.",
                    "Your goal is to translate high-level human goals into structured user stories and acceptance criteria.",
                    "Rules:",
                    "- Use tools to inspect the repo to understand the current state before planning.",
                    "- When you have a clear plan, respond with a JSON object of type 'finalize_task' containing the TaskPacket.",
                    "- If UI is involved, include a Mermaid wireframe in the TaskPacket.",
                    if mode == PromptMode::Structured {
                        "- Respond with EXACTLY one JSON object each turn."
                    } else {
                        "- You may respond in plain text."
                    },
                    if mode == PromptMode::Structured {
                        "- Allowed JSON variants:"
                    } else {
                        "- If you need to use a tool, respond with EXACTLY one JSON object:"
                    },
                    "  {\"type\":\"tool\",\"tool\":<string>,\"args\":<object>}",
                    if mode == PromptMode::Structured {
                        "  {\"type\":\"finalize_task\",\"packet\":{\"title\":<string>,\"user_stories\":[<string>],\"acceptance_criteria\":[<string>],\"mermaid_wireframe\":<string|null>}}"
                    } else {
                        "  (Otherwise respond in plain text)"
                    },
                    if mode == PromptMode::Structured {
                        "  {\"type\":\"ask\",\"question\":<string>}"
                    } else {
                        ""
                    },
                ]
                .join("\n"),
                AgentRole::Coder => [
                    "You are linggen-agent 'coder'.",
                    "Rules:",
                    "- You can write files directly using the provided tools.",
                    "- Use tools to inspect the repo before making changes.",
                    if mode == PromptMode::Structured {
                        "- Respond with EXACTLY one JSON object each turn."
                    } else {
                        "- You may respond in plain text."
                    },
                    if mode == PromptMode::Structured {
                        "- Allowed JSON variants:"
                    } else {
                        "- If you need to use a tool, respond with EXACTLY one JSON object:"
                    },
                    "  {\"type\":\"tool\",\"tool\":<string>,\"args\":<object>}",
                    if mode == PromptMode::Structured {
                        "  {\"type\":\"ask\",\"question\":<string>}"
                    } else {
                        "  (Otherwise respond in plain text)"
                    },
                ]
                .join("\n"),
                AgentRole::Operator => [
                    "You are linggen-agent 'operator'.",
                    "Your goal is to verify implementations and handle releases.",
                    "Rules:",
                    "- Use 'run_command' to run tests and verify the build state.",
                    "- Use 'capture_screenshot' to verify UI requirements for web apps.",
                    "- Report success or failure clearly. If tests fail, provide logs to help the Coder.",
                    if mode == PromptMode::Structured {
                        "- Respond with EXACTLY one JSON object each turn."
                    } else {
                        "- You may respond in plain text."
                    },
                    if mode == PromptMode::Structured {
                        "- Allowed JSON variants:"
                    } else {
                        "- If you need to use a tool, respond with EXACTLY one JSON object:"
                    },
                    "  {\"type\":\"tool\",\"tool\":<string>,\"args\":<object>}",
                    if mode == PromptMode::Structured {
                        "  {\"type\":\"ask\",\"question\":<string>}"
                    } else {
                        "  (Otherwise respond in plain text)"
                    },
                ]
                .join("\n"),
            };
            format!(
                "{}\n\nAgent ID: {}\nDescription: {}",
                base,
                self.agent_id.as_deref().unwrap_or("unknown"),
                spec.description
            )
        } else {
            "You are a helpful AI assistant.".to_string()
        };

        // Inject runtime context
        prompt.push_str("\n\n--- RUNTIME CONTEXT ---");
        prompt.push_str(&format!("\nWorkspaceRoot: {}", self.cfg.ws_root.display()));
        if let Some(spec) = &self.spec {
            prompt.push_str(&format!(
                "\nWorkScope (allowed write globs): {:?}",
                spec.work_globs
            ));
        }

        // Dynamic held locks
        let held_locks = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                if let (Some(manager), Some(agent_id)) = (self.tools.get_manager(), &self.agent_id)
                {
                    let locks = manager.locks.lock().await;
                    // Filter locks owned by this agent
                    locks
                        .locks
                        .iter()
                        .filter(|(_, info)| &info.owner_id == agent_id)
                        .map(|(glob, _): (&String, _)| glob.clone())
                        .collect::<Vec<String>>()
                } else {
                    Vec::new()
                }
            })
        });

        prompt.push_str(&format!("\nHeldLocks: {:?}", held_locks));
        prompt.push_str("\n-----------------------");

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

pub fn render_tool_result(r: &ToolResult) -> String {
    match r {
        ToolResult::RepoInfo(v) => format!("repo_info: {}", v),
        ToolResult::FileList(v) => format!("files:\n{}", v.join("\n")),
        ToolResult::FileContent {
            path,
            content,
            truncated,
        } => {
            format!(
                "read_file: {} (truncated: {})\n{}",
                path, truncated, content
            )
        }
        ToolResult::SearchMatches(v) => {
            let mut out = String::new();
            out.push_str("search_matches:\n");
            for m in v {
                out.push_str(&format!("{}:{}:{}\n", m.path, m.line, m.snippet));
            }
            out
        }
        ToolResult::CommandOutput {
            exit_code,
            stdout,
            stderr,
        } => {
            format!(
                "command_output (exit_code: {:?}):\nSTDOUT:\n{}\nSTDERR:\n{}",
                exit_code, stdout, stderr
            )
        }
        ToolResult::Screenshot { url, base64 } => {
            format!(
                "screenshot_captured: {} (base64 length: {})",
                url,
                base64.len()
            )
        }
        ToolResult::Success(msg) => format!("success: {}", msg),
        ToolResult::LockResult { acquired, denied } => {
            format!("lock_result: acquired={:?}, denied={:?}", acquired, denied)
        }
        ToolResult::AgentOutcome(outcome) => {
            format!("agent_outcome: {:?}", outcome)
        }
    }
}

fn preview_text(content: &str, max_lines: usize, max_chars: usize) -> (String, bool) {
    let mut out = String::new();
    let mut lines = 0usize;
    let mut truncated = false;

    for line in content.lines() {
        if lines >= max_lines {
            truncated = true;
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
        lines += 1;

        if out.len() >= max_chars {
            truncated = true;
            out.truncate(max_chars);
            break;
        }
    }

    (out, truncated)
}

/// Render tool results for DB/UI without dumping large payloads (e.g. full files).
pub fn render_tool_result_public(r: &ToolResult) -> String {
    match r {
        ToolResult::FileContent {
            path,
            content,
            truncated,
        } => {
            let (preview, preview_truncated) = preview_text(content, 20, 1200);
            let shown_note = if preview_truncated { " (preview)" } else { "" };
            format!(
                "read_file: {} (truncated: {}){}\n{}\n\n(content omitted in chat; open the file viewer for full text)",
                path, truncated, shown_note, preview
            )
        }
        other => render_tool_result(other),
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ModelAction {
    #[serde(rename = "tool")]
    Tool {
        tool: String,
        args: serde_json::Value,
    },
    #[serde(rename = "patch")]
    Patch { diff: String },
    #[serde(rename = "finalize_task")]
    FinalizeTask { packet: TaskPacket },
    #[serde(rename = "ask")]
    Ask { question: String },
}

pub fn parse_first_action(raw: &str) -> Result<ModelAction> {
    let mut de = Deserializer::from_str(raw);
    let action = ModelAction::deserialize(&mut de)?;
    Ok(action)
}
