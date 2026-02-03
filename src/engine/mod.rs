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
use serde::{Deserialize, Serialize};
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
    cfg: EngineConfig,
    model_manager: Arc<ModelManager>,
    model_id: String,
    tools: Tools,
    role: AgentRole,
    task: Option<String>,
    // Agent spec and runtime context
    spec: Option<AgentSpec>,
    agent_id: Option<String>,
    // Rolling tool observations that we feed back to the model.
    observations: Vec<String>,
    // Conversational history for chat.
    chat_history: Vec<ChatMessage>,
    // Active skill if any
    active_skill: Option<Skill>,
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

    pub async fn chat(&mut self, message: &str, session_id: Option<&str>) -> Result<String> {
        info!(
            "Processing chat message for role {:?}: {}",
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
        }

        // Record message in DB
        if let Some(manager) = self.tools.get_manager() {
            let _ = manager.db.add_chat_message(crate::db::ChatMessageRecord {
                repo_path: self.cfg.ws_root.to_string_lossy().to_string(),
                session_id: session_id.unwrap_or("default").to_string(),
                agent_id: self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                from_id: "user".to_string(),
                to_id: self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                content: clean_message.clone(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                is_observation: false,
            });
        }

        self.chat_history.push(ChatMessage {
            role: "user".to_string(),
            content: clean_message,
        });

        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: self.system_prompt(),
        }];

        // Add workspace context to the first message if history is short
        if self.chat_history.len() == 1 {
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

        // Use chat_text for conversational turns (no JSON enforcement unless we want tools in chat)
        let response = self
            .model_manager
            .chat_text(&self.model_id, &messages)
            .await?;

        // Try to parse the response as JSON to extract the question if the model followed the system prompt
        let final_content = if let Ok(action) = serde_json::from_str::<ModelAction>(&response) {
            match action {
                ModelAction::Ask { question } => question,
                ModelAction::FinalizeTask { packet } => {
                    format!("I've finalized the task: {}. You can review it in the Planning section.", packet.title)
                }
                ModelAction::Tool { tool, .. } => {
                    format!("I'm using the tool: {}. Please use the 'Execute Loop' button to let me continue with tool execution.", tool)
                }
                ModelAction::Patch { .. } => {
                    "I've proposed a code patch. Please use the 'Execute Loop' button to see the details.".to_string()
                }
            }
        } else {
            response
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
            content: final_content.clone(),
        });

        Ok(final_content)
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

        for obs in &self.observations {
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: format!("Observation:\n{}", obs),
            });
        }

        for _ in 0..self.cfg.max_iters {
            // Ask model for the next action as JSON.
            let raw = self
                .model_manager
                .chat_json(&self.model_id, &messages)
                .await?;

            let action: ModelAction = match serde_json::from_str(&raw) {
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
                    let call = ToolCall {
                        tool: tool.clone(),
                        args,
                    };
                    let result = self.tools.execute(call)?;
                    let rendered = render_tool_result(&result);
                    self.observations.push(rendered.clone());

                    // Record observation in DB
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

                    messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: format!("Observation:\n{}", rendered),
                    });
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
                    return Ok(AgentOutcome::Task(packet));
                }
                ModelAction::Ask { question } => {
                    info!("Agent asked a question: {}", question);
                    return Ok(AgentOutcome::Ask(question));
                }
            }
        }

        Ok(AgentOutcome::None)
    }

    fn system_prompt(&self) -> String {
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
                    "- Respond with EXACTLY one JSON object each turn.",
                    "- Allowed JSON variants:",
                    "  {\"type\":\"tool\",\"tool\":<string>,\"args\":<object>}",
                    "  {\"type\":\"finalize_task\",\"packet\":{\"title\":<string>,\"user_stories\":[<string>],\"acceptance_criteria\":[<string>],\"mermaid_wireframe\":<string|null>}}",
                    "  {\"type\":\"ask\",\"question\":<string>}",
                ]
                .join("\n"),
                AgentRole::Coder => [
                    "You are linggen-agent 'coder'.",
                    "Rules:",
                    "- You can write files directly using the provided tools.",
                    "- Use tools to inspect the repo before making changes.",
                    "- Respond with EXACTLY one JSON object each turn.",
                    "- Allowed JSON variants:",
                    "  {\"type\":\"tool\",\"tool\":<string>,\"args\":<object>}",
                    "  {\"type\":\"ask\",\"question\":<string>}",
                ]
                .join("\n"),
                AgentRole::Operator => [
                    "You are linggen-agent 'operator'.",
                    "Your goal is to verify implementations and handle releases.",
                    "Rules:",
                    "- Use 'run_command' to run tests and verify the build state.",
                    "- Use 'capture_screenshot' to verify UI requirements for web apps.",
                    "- Report success or failure clearly. If tests fail, provide logs to help the Coder.",
                    "- Respond with EXACTLY one JSON object each turn.",
                    "- Allowed JSON variants:",
                    "  {\"type\":\"tool\",\"tool\":<string>,\"args\":<object>}",
                    "  {\"type\":\"ask\",\"question\":<string>}",
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

fn render_tool_result(r: &ToolResult) -> String {
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ModelAction {
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
