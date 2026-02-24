pub mod actions;
pub mod patch;
pub mod render;
pub mod skill_tool;
pub mod tool_registry;
pub mod tools;
pub mod web_search;

use crate::agent_manager::models::ModelManager;
use crate::agent_manager::AgentManager;
use crate::config::{AgentPolicyCapability, AgentSpec};
use crate::engine::patch::validate_unified_diff;
use crate::engine::tool_registry::ToolRegistry;
use crate::engine::tools::ToolCall;
use crate::ollama::ChatMessage;
use crate::skills::Skill;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::StreamExt as TokioStreamExt;
use tracing::{info, warn};

pub use actions::{model_message_log_parts, parse_all_actions, ModelAction, PlanItemUpdate};

// ---------------------------------------------------------------------------
// Plan mode data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItem {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: PlanItemStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlanItemStatus {
    Pending,
    InProgress,
    Done,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub summary: String,
    pub items: Vec<PlanItem>,
    pub status: PlanStatus,
    #[serde(default)]
    pub origin: PlanOrigin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Planned,
    Approved,
    Executing,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlanOrigin {
    UserRequested,
    ModelManaged,
}

impl Default for PlanOrigin {
    fn default() -> Self {
        PlanOrigin::ModelManaged
    }
}

#[derive(Debug, Clone)]
pub enum ThinkingEvent {
    Token(String),
    Done,
}

#[derive(Debug, Clone)]
pub enum ReplEvent {
    Status { status: String, detail: Option<String> },
    Iteration { current: usize, max: usize },
}
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
    pub tools: ToolRegistry,
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
    pub parent_agent_id: Option<String>,
    pub run_id: Option<String>,
    pub thinking_tx: Option<mpsc::UnboundedSender<ThinkingEvent>>,
    pub repl_events_tx: Option<mpsc::UnboundedSender<ReplEvent>>,
    /// Receiver for user interrupt messages injected while the agent loop is running.
    pub interrupt_rx: Option<mpsc::UnboundedReceiver<String>>,
    // Plan mode
    pub plan_mode: bool,
    pub plan: Option<Plan>,
    /// Path to the current plan's file in ~/.linggen/plans/
    pub plan_file: Option<PathBuf>,
    /// Override for plans directory (used in tests for isolation).
    pub plans_dir_override: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum AgentOutcome {
    #[serde(rename = "task")]
    Task(TaskPacket),
    #[serde(rename = "patch")]
    Patch(String),
    #[serde(rename = "plan")]
    Plan(Plan),
    #[serde(rename = "plan_mode_requested")]
    PlanModeRequested {
        #[serde(default)]
        reason: Option<String>,
    },
    #[serde(rename = "none")]
    None,
}

/// Control flow returned by extracted loop helpers.
enum LoopControl {
    /// Continue to the next iteration of the agent loop.
    Continue,
    /// Exit the loop and return this outcome.
    Return(AgentOutcome),
}

#[derive(Clone)]
struct CachedToolObs {
    model: String,
}

impl AgentEngine {
    pub fn new(
        cfg: EngineConfig,
        model_manager: Arc<ModelManager>,
        model_id: String,
        role: AgentRole,
    ) -> Result<Self> {
        let builtins = tools::Tools::new(cfg.ws_root.clone())?;
        let tools = ToolRegistry::new(builtins);
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
            parent_agent_id: None,
            run_id: None,
            thinking_tx: None,
            repl_events_tx: None,
            interrupt_rx: None,
            plan_mode: false,
            plan: None,
            plan_file: None,
            plans_dir_override: None,
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
            self.tools.set_context(manager, agent_id.clone());
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

    pub fn set_delegation_depth(&mut self, depth: usize, max_depth: usize) {
        self.tools.builtins.set_delegation_depth(depth);
        self.tools.builtins.set_max_delegation_depth(max_depth);
    }

    pub fn set_run_id(&mut self, run_id: Option<String>) {
        self.run_id = run_id.clone();
        self.tools.set_run_id(run_id);
    }

    pub fn set_memory_dir(&mut self, dir: PathBuf) {
        self.tools.set_memory_dir(dir);
    }

    pub fn get_task(&self) -> Option<String> {
        self.task.clone()
    }

    /// Load skill-defined tools into the registry based on the agent spec's skills list.
    /// Skills with `disable_model_invocation == true` are skipped (their tools are not
    /// registered in the model-facing tool schema).
    pub async fn load_skill_tools(&mut self, skill_manager: &crate::skills::SkillManager) {
        let Some(spec) = &self.spec else { return };
        let skill_names = spec.skills.clone();
        for skill_name in &skill_names {
            if let Some(skill) = skill_manager.get_skill(skill_name).await {
                if skill.disable_model_invocation {
                    continue;
                }
                for tool_def in skill.tool_defs {
                    self.tools.register_skill_tool(tool_def);
                }
            }
        }
    }

    pub async fn manager_db_add_observation(
        &self,
        tool: &str,
        rendered: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        if let Some(manager) = self.tools.get_manager() {
            let aid = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            manager
                .add_chat_message(
                    &self.cfg.ws_root,
                    session_id.unwrap_or("default"),
                    &crate::state_fs::sessions::ChatMsg {
                        agent_id: aid.clone(),
                        from_id: "system".to_string(),
                        to_id: aid,
                        content: format!("Tool {}: {}", tool, rendered),
                        timestamp: crate::util::now_ts_secs(),
                        is_observation: true,
                    },
                )
                .await;
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
            manager
                .add_chat_message(
                    &self.cfg.ws_root,
                    session_id.unwrap_or("default"),
                    &crate::state_fs::sessions::ChatMsg {
                        agent_id: agent_id.clone(),
                        from_id: agent_id.clone(),
                        to_id: target,
                        content: content.to_string(),
                        timestamp: crate::util::now_ts_secs(),
                        is_observation: false,
                    },
                )
                .await;

            // Nudge UI to refresh immediately.
            manager
                .send_event(crate::agent_manager::AgentEvent::StateUpdated)
                .await;
        }
        Ok(())
    }

    /// Validate, dispatch, and record a single tool call from the model.
    #[allow(clippy::too_many_arguments)]
    async fn handle_tool_action(
        &mut self,
        tool: String,
        args: JsonValue,
        allowed_tools: &Option<HashSet<String>>,
        messages: &mut Vec<ChatMessage>,
        tool_cache: &mut HashMap<String, CachedToolObs>,
        read_paths: &mut HashSet<String>,
        last_tool_sig: &mut String,
        redundant_tool_streak: &mut usize,
        empty_search_streak: &mut usize,
        session_id: Option<&str>,
    ) -> LoopControl {
        let canonical_tool = self
            .tools
            .canonical_tool_name(&tool)
            .unwrap_or(tool.as_str())
            .to_string();

        // --- permission gate ---
        if let Some(allowed) = allowed_tools {
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
                messages.push(ChatMessage::new(
                    "user",
                    format!(
                        "Tool '{}' is not allowed for this agent. Use one of [{}].",
                        tool,
                        allowed_list.join(", ")
                    ),
                ));
                return LoopControl::Continue;
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

        // --- write-safety gate ---
        if matches!(canonical_tool.as_str(), "Write" | "Edit") {
            if let Some(path) = normalize_tool_path_arg(&self.cfg.ws_root, &args) {
                let existing = self.cfg.ws_root.join(&path).exists();
                if existing && !read_paths.contains(&path) {
                    let action = if canonical_tool == "Edit" {
                        "Edit"
                    } else {
                        "Write"
                    };
                    match self.cfg.write_safety_mode {
                        crate::config::WriteSafetyMode::Strict => {
                            let rendered = format!(
                                "tool_error: tool={} error=precondition_failed: must call Read on '{}' before {} for existing files",
                                action, path, action
                            );
                            self.upsert_observation("error", action, rendered.clone());
                            let _ = self
                                .manager_db_add_observation(action, &rendered, session_id)
                                .await;
                            messages.push(ChatMessage::new(
                                "user",
                                format!(
                                    "Tool execution blocked for safety: {}. Read the existing file first, then apply a minimal update.",
                                    rendered,
                                ),
                            ));
                            return LoopControl::Continue;
                        }
                        crate::config::WriteSafetyMode::Warn => {
                            let rendered = format!(
                                "tool_warning: tool={} warning=writing_existing_file_without_prior_read path='{}'",
                                action, path
                            );
                            self.upsert_observation("warning", action, rendered.clone());
                            let _ = self
                                .manager_db_add_observation(action, &rendered, session_id)
                                .await;
                        }
                        crate::config::WriteSafetyMode::Off => {}
                    }
                }
            }
        }

        // --- redundancy / cache gates ---
        let sig = tool_call_signature(&canonical_tool, &args);
        if sig == *last_tool_sig {
            *redundant_tool_streak += 1;
        } else {
            *redundant_tool_streak = 0;
            *last_tool_sig = sig.clone();
        }

        if *redundant_tool_streak >= 3 {
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
            messages.push(ChatMessage::new("user", loop_breaker_prompt));
            self.push_context_record(
                ContextType::Error,
                Some("redundant_tool_loop".to_string()),
                self.agent_id.clone(),
                None,
                format!(
                    "Repeated tool call loop detected for '{}'; nudging model to change approach.",
                    canonical_tool
                ),
                serde_json::json!({ "tool": canonical_tool, "streak": *redundant_tool_streak + 1 }),
            );
            *redundant_tool_streak = 0;
            return LoopControl::Continue;
        }

        if let Some(cached) = tool_cache.get(&sig) {
            self.upsert_observation("tool", &canonical_tool, cached.model.clone());
            messages.push(ChatMessage::new(
                "user",
                Self::observation_text("tool", &canonical_tool, &cached.model),
            ));
            return LoopControl::Continue;
        }

        // --- status lines ---
        let tool_start_status = crate::server::chat_helpers::tool_status_line(
            &canonical_tool,
            Some(&args),
            crate::server::chat_helpers::ToolStatusPhase::Start,
        );
        let tool_done_status = crate::server::chat_helpers::tool_status_line(
            &canonical_tool,
            Some(&args),
            crate::server::chat_helpers::ToolStatusPhase::Done,
        );
        let tool_failed_status = crate::server::chat_helpers::tool_status_line(
            &canonical_tool,
            Some(&args),
            crate::server::chat_helpers::ToolStatusPhase::Failed,
        );

        // Tell the UI what tool we're about to use.
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
                    detail: Some(tool_start_status.clone()),
                })
                .await;
            if let Some(tx) = &self.repl_events_tx {
                let _ = tx.send(ReplEvent::Status {
                    status: "calling_tool".to_string(),
                    detail: Some(tool_start_status.clone()),
                });
            }
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
            manager
                .add_chat_message(
                    &self.cfg.ws_root,
                    session_id.unwrap_or("default"),
                    &crate::state_fs::sessions::ChatMsg {
                        agent_id: from.clone(),
                        from_id: from,
                        to_id: target,
                        content: tool_msg,
                        timestamp: crate::util::now_ts_secs(),
                        is_observation: false,
                    },
                )
                .await;
        }

        // --- execute ---
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

                // Invalidate cached Read results for the same file after a successful mutation.
                // The Read cache key format is: Read|{"path":"<path>"}  (default sig).
                // We invalidate any Read cache entry whose key contains the file path.
                if matches!(canonical_tool.as_str(), "Write" | "Edit") {
                    if let Some(path) = normalize_tool_path_arg(&self.cfg.ws_root, &args) {
                        tool_cache.retain(|key, _| {
                            if !key.starts_with("Read|") {
                                return true;
                            }
                            !key.contains(&format!("\"{}\"", path))
                        });
                    }
                }

                self.upsert_observation("tool", &canonical_tool, rendered_model.clone());

                let _ = self
                    .manager_db_add_observation(&canonical_tool, &rendered_public, session_id)
                    .await;
                if let Some(manager) = self.tools.get_manager() {
                    let agent_id = self
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    manager
                        .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                            agent_id: agent_id.clone(),
                            status: "calling_tool".to_string(),
                            detail: Some(tool_done_status.clone()),
                        })
                        .await;
                    manager
                        .send_event(crate::agent_manager::AgentEvent::StateUpdated)
                        .await;
                    manager
                        .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                            agent_id,
                            status: "thinking".to_string(),
                            detail: Some("Thinking".to_string()),
                        })
                        .await;
                    if let Some(tx) = &self.repl_events_tx {
                        let _ = tx.send(ReplEvent::Status {
                            status: "thinking".to_string(),
                            detail: Some("Thinking".to_string()),
                        });
                    }
                }

                // For file mutations, emit a brief user-visible summary line.
                if matches!(canonical_tool.as_str(), "Write" | "Edit")
                    && (rendered_public.starts_with("File written:")
                        || rendered_public.starts_with("Edited file:")
                        || rendered_public.starts_with("File unchanged"))
                {
                    let msg = if rendered_public.starts_with("File unchanged") {
                        if let Some(idx) = rendered_public.rfind(':') {
                            let path = rendered_public[idx + 1..].trim();
                            format!("No changes to `{}`.", path)
                        } else {
                            "No file changes.".to_string()
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

                messages.push(ChatMessage::new(
                    "user",
                    Self::observation_text("tool", &canonical_tool, &rendered_model),
                ));

                if canonical_tool == "Grep"
                    && (rendered_model.contains("(no matches)")
                        || rendered_model.contains("no file candidates found"))
                {
                    *empty_search_streak += 1;
                } else {
                    *empty_search_streak = 0;
                }
                if *empty_search_streak >= 4 {
                    messages.push(ChatMessage::new(
                        "user",
                        "Grep returned no matches repeatedly. Change strategy and continue automatically (for example: broaden terms, use Glob to inspect files, then Read likely paths).",
                    ));
                    self.push_context_record(
                        ContextType::Error,
                        Some("empty_search_loop".to_string()),
                        self.agent_id.clone(),
                        None,
                        "Repeated no-match search loop detected; nudging model to change strategy."
                            .to_string(),
                        serde_json::json!({ "streak": *empty_search_streak }),
                    );
                    *empty_search_streak = 0;
                }
            }
            Err(e) => {
                warn!("Tool execution failed ({}): {}", canonical_tool, e);
                let rendered = format!("tool_error: tool={} error={}", canonical_tool, e);
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
                            agent_id: agent_id.clone(),
                            status: "calling_tool".to_string(),
                            detail: Some(tool_failed_status.clone()),
                        })
                        .await;
                    manager
                        .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                            agent_id,
                            status: "thinking".to_string(),
                            detail: Some("Thinking".to_string()),
                        })
                        .await;
                    if let Some(tx) = &self.repl_events_tx {
                        let _ = tx.send(ReplEvent::Status {
                            status: "thinking".to_string(),
                            detail: Some("Thinking".to_string()),
                        });
                    }
                }
                messages.push(ChatMessage::new(
                    "user",
                    format!(
                        "Tool execution failed for tool='{}'. Error: {}. Choose a valid tool+args from the tool schema and try again.",
                        canonical_tool, e
                    ),
                ));
            }
        }
        LoopControl::Continue
    }

    /// Detect identical model responses and nudge or bail.
    async fn handle_repetition_check(
        &mut self,
        raw: &str,
        last_response: &mut String,
        streak: &mut usize,
        nudge_count: &mut usize,
        messages: &mut Vec<ChatMessage>,
        session_id: Option<&str>,
    ) -> Option<LoopControl> {
        if raw == last_response.as_str() {
            *streak += 1;
        } else {
            *streak = 0;
            *last_response = raw.to_string();
        }

        if *streak < 3 {
            return None;
        }

        *nudge_count += 1;
        if *nudge_count >= 2 {
            let message = format!(
                "I couldn't continue automatically because I got stuck in a repetition loop (same response {} times).",
                *streak + 1
            );
            let _ = self
                .manager_db_add_assistant_message(&message, session_id)
                .await;
            self.active_skill = None;
            return Some(LoopControl::Return(AgentOutcome::None));
        }

        messages.push(ChatMessage::new(
            "user",
            "It looks like you are trapped in a loop, sending the same response multiple times. Please try a different approach or tool to make progress.",
        ));
        self.push_context_record(
            ContextType::Error,
            Some("loop_detected".to_string()),
            self.agent_id.clone(),
            None,
            "Model trapped in a loop. Nudging with a warning message.".to_string(),
            serde_json::json!({ "streak": *streak + 1 }),
        );
        *streak = 0;
        Some(LoopControl::Continue)
    }

    /// Build the initial message list and read-paths set for the structured agent loop.
    fn prepare_loop_messages(
        &mut self,
        task: &str,
    ) -> (Vec<ChatMessage>, Option<HashSet<String>>, HashSet<String>) {
        let mut system = self.system_prompt();

        // --- Project context files (AGENTS.md, CLAUDE.md, .cursorrules) ---
        {
            let context_filenames = ["AGENTS.md", "CLAUDE.md", ".cursorrules"];
            let mut seen: std::collections::HashSet<std::path::PathBuf> =
                std::collections::HashSet::new();
            let mut sections: Vec<(String, String)> = Vec::new();

            let mut dir: Option<&std::path::Path> = Some(self.cfg.ws_root.as_path());
            while let Some(current) = dir {
                for filename in &context_filenames {
                    let filepath = current.join(filename);
                    if let Ok(canonical) = filepath.canonicalize() {
                        if seen.contains(&canonical) {
                            continue;
                        }
                        if let Ok(content) = std::fs::read_to_string(&filepath) {
                            let content = content.trim().to_string();
                            if !content.is_empty() {
                                let label = if current == self.cfg.ws_root.as_path() {
                                    filename.to_string()
                                } else {
                                    format!("{} (from {})", filename, current.display())
                                };
                                sections.push((label, content));
                                seen.insert(canonical);
                            }
                        }
                    }
                }
                dir = current.parent();
            }

            // Outermost (general) first, workspace root (specific) last
            sections.reverse();

            if !sections.is_empty() {
                system.push_str("\n\n--- PROJECT INSTRUCTIONS ---");
                for (label, content) in &sections {
                    system.push_str(&format!("\n\n# {}\n\n{}", label, content));
                }
                system.push_str("\n\n--- END PROJECT INSTRUCTIONS ---");
            }
        }

        // --- Auto Memory ---
        if let Some(memory_dir) = self.tools.memory_dir() {
            let memory_path = memory_dir.join("MEMORY.md");
            let mem_dir_display = memory_dir.display().to_string();
            if let Ok(content) = std::fs::read_to_string(&memory_path) {
                let content = content.trim();
                if !content.is_empty() {
                    let truncated: String =
                        content.lines().take(200).collect::<Vec<_>>().join("\n");
                    system.push_str(&format!(
                        "\n\n--- AUTO MEMORY ---\n\
                         You have a persistent memory directory at `{}`.\n\
                         Its contents persist across sessions. Use Write/Edit tools to update MEMORY.md.\n\
                         \n\
                         Guidelines:\n\
                         - Save stable patterns, user preferences, key architecture decisions, project structure\n\
                         - Do NOT save session-specific context, in-progress work, or unverified conclusions\n\
                         - Keep MEMORY.md concise (under 200 lines)\n\
                         - When user says \"remember X\", save it immediately\n\
                         \n## MEMORY.md\n\n{}\n\
                         --- END AUTO MEMORY ---",
                        mem_dir_display, truncated
                    ));
                }
            }
            // Even if MEMORY.md doesn't exist yet, tell the agent about the memory dir
            if !system.contains("AUTO MEMORY") {
                system.push_str(&format!(
                    "\n\n--- AUTO MEMORY ---\n\
                     You have a persistent memory directory at `{}`.\n\
                     Its contents persist across sessions. Create MEMORY.md with Write to start saving memories.\n\
                     \n\
                     Guidelines:\n\
                     - Save stable patterns, user preferences, key architecture decisions, project structure\n\
                     - Do NOT save session-specific context, in-progress work, or unverified conclusions\n\
                     - Keep MEMORY.md concise (under 200 lines)\n\
                     - When user says \"remember X\", save it immediately\n\
                     --- END AUTO MEMORY ---",
                    mem_dir_display
                ));
            }
        }

        // Task list guidance (always present, not just in plan mode).
        if !self.plan_mode {
            system.push_str(
                "\n\n## Task List\n\
                 For complex multi-step tasks, you can create a task list to track progress:\n\
                 {\"type\":\"update_plan\",\"summary\":\"<brief description>\",\"items\":[{\"title\":\"Step 1\",\"status\":\"pending\"},{\"title\":\"Step 2\",\"status\":\"pending\"}]}\n\
                 Update items as you complete them:\n\
                 {\"type\":\"update_plan\",\"items\":[{\"title\":\"Step 1\",\"status\":\"done\"}]}\n\
                 For large tasks that need upfront research and user approval before execution, use:\n\
                 {\"type\":\"enter_plan_mode\",\"reason\":\"<why planning is needed>\"}\n\
                 This enters plan mode where you research with read-only tools, produce a plan, and await user approval.\n\
                 Skip both for simple single-step tasks."
            );
        }

        // Plan mode: restrict to read-only tools and instruct the model to produce a plan.
        if self.plan_mode {
            system.push_str(
                "\n\nYou are in PLAN MODE. Your goal is to research the codebase and produce a detailed structured plan.\n\
                 Do NOT write, edit, or create any files. Only use Read, Glob, Grep, and Bash (read-only commands like ls, git status).\n\n\
                 When your plan is ready, emit an update_plan action with:\n\
                 - summary: A clear title for the plan\n\
                 - items: Each item MUST include a description with:\n\
                   - Which file(s) to modify (full relative paths)\n\
                   - What specific changes to make\n\
                   - Any relevant code patterns or context discovered during research\n\n\
                 The plan must be detailed enough that someone with no prior context can execute it.\n\
                 Then emit a done action."
            );
        }

        // If executing an approved plan, inject the plan items into the prompt.
        if let Some(plan) = &self.plan {
            if plan.status == PlanStatus::Approved || plan.status == PlanStatus::Executing {
                let items_text = plan
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let desc = item.description.as_deref().unwrap_or("");
                        format!("{}. [{}] {} {}", i + 1, serde_json::to_string(&item.status).unwrap_or_default().trim_matches('"'), item.title, desc)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                system.push_str(&format!(
                    "\n\nExecute the following approved plan. After completing each item, emit an update_plan action to mark it done.\n\nPlan: {}\n{}",
                    plan.summary, items_text
                ));
            }
        }

        let mut allowed_tools = self.allowed_tool_names();

        // In plan mode, restrict to read-only tools.
        if self.plan_mode {
            let read_only: HashSet<String> = [
                "Read", "Glob", "Grep", "Bash",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            allowed_tools = Some(match allowed_tools {
                Some(existing) => existing.intersection(&read_only).cloned().collect(),
                None => read_only,
            });
        }

        let mut messages = vec![ChatMessage::new("system", system)];
        self.push_context_record(
            ContextType::System,
            Some("structured_loop_prompt".to_string()),
            None,
            None,
            messages[0].content.clone(),
            serde_json::json!({ "mode": "structured" }),
        );

        // Include chat history so the model has context of the current conversation.
        messages.extend(self.chat_history.clone());

        for obs in &self.observations {
            messages.push(ChatMessage::new("user", Self::observation_for_model(obs)));
        }

        // Provide tool schema + workspace info (last user message).
        messages.push(ChatMessage::new(
            "user",
            format!(
                "Autonomous agent loop started. Ignore any prior greetings or small talk.\n\nWorkspace root: {}\nPlatform: {}\n\nCurrent Role: {:?}\n\nTask: {}\n\nTool schema (respond with one or more JSON tool call objects per turn):\n{}\n\nWhen the task is fully complete, respond with: {{\"type\":\"done\",\"message\":\"<brief summary>\"}}",
                self.cfg.ws_root.display(),
                std::env::consts::OS,
                self.role,
                task,
                self.tools.tool_schema_json(allowed_tools.as_ref())
            ),
        ));
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

        // Pre-populate read_paths from prior context.
        let mut read_paths: HashSet<String> = HashSet::new();
        let ws_root = self.cfg.ws_root.clone();
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
                if let Ok(abs) = ws_root.join(&clean_path).canonicalize() {
                    if let Ok(rel) = abs.strip_prefix(&ws_root) {
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

        (messages, allowed_tools, read_paths)
    }

    async fn handle_patch_action(
        &mut self,
        diff: String,
        messages: &mut Vec<ChatMessage>,
    ) -> LoopControl {
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
            messages.push(ChatMessage::new(
                "user",
                "Error: This agent is not allowed to output 'patch'. Add `Patch` to the agent frontmatter `policy` to enable it.",
            ));
            return LoopControl::Continue;
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
            messages.push(ChatMessage::new(
                "user",
                format!(
                    "The patch failed validation. Fix and respond with a new patch JSON. Errors:\n{}",
                    errs.join("\n")
                ),
            ));
            return LoopControl::Continue;
        }

        info!("Patch validated successfully");

        self.active_skill = None;
        LoopControl::Return(AgentOutcome::Patch(diff))
    }

    async fn handle_update_plan_action(
        &mut self,
        summary: Option<String>,
        items: Vec<PlanItemUpdate>,
        session_id: Option<&str>,
    ) -> LoopControl {
        info!("Agent emitted update_plan with {} items", items.len());

        let plan = if let Some(existing) = &mut self.plan {
            // Update existing plan: merge item statuses
            if let Some(s) = summary {
                existing.summary = s;
            }

            // Check if every update item matches an existing title.
            // If any item is new, the model is sending a revised plan —
            // replace all items to avoid duplicates from rewording.
            let all_match = items.iter().all(|u| {
                existing.items.iter().any(|i| i.title == u.title)
            });

            if all_match {
                // Pure status update: merge into existing items.
                for update in &items {
                    if let Some(item) = existing
                        .items
                        .iter_mut()
                        .find(|i| i.title == update.title)
                    {
                        if let Some(status) = &update.status {
                            item.status = status.clone();
                        }
                        if update.description.is_some() {
                            item.description = update.description.clone();
                        }
                    }
                }
            } else {
                // Revised plan: replace items entirely, preserving status
                // of items that still match by title.
                let new_items: Vec<PlanItem> = items
                    .iter()
                    .map(|u| {
                        let prev_status = existing
                            .items
                            .iter()
                            .find(|i| i.title == u.title)
                            .map(|i| i.status.clone());
                        PlanItem {
                            title: u.title.clone(),
                            description: u.description.clone(),
                            status: u.status.clone()
                                .or(prev_status)
                                .unwrap_or(PlanItemStatus::Pending),
                        }
                    })
                    .collect();
                existing.items = new_items;
            }
            existing.clone()
        } else {
            // Create new plan.
            // User-requested plans (plan_mode) need approval; model task lists execute immediately.
            let (origin, status) = if self.plan_mode {
                (PlanOrigin::UserRequested, PlanStatus::Planned)
            } else {
                (PlanOrigin::ModelManaged, PlanStatus::Executing)
            };
            let plan = Plan {
                summary: summary.unwrap_or_else(|| "Plan".to_string()),
                items: items
                    .iter()
                    .map(|u| PlanItem {
                        title: u.title.clone(),
                        description: u.description.clone(),
                        status: u.status.clone().unwrap_or(PlanItemStatus::Pending),
                    })
                    .collect(),
                status,
                origin,
            };
            self.plan = Some(plan.clone());
            plan
        };

        // Persist plan to .linggen-agent/plan.md
        self.write_plan_file(&plan);

        // Emit PlanUpdate event via manager
        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            manager
                .send_event(crate::agent_manager::AgentEvent::PlanUpdate {
                    agent_id: agent_id.clone(),
                    plan: plan.clone(),
                })
                .await;
        }

        // Persist the plan as a structured chat message
        let msg = serde_json::json!({ "type": "plan", "plan": plan }).to_string();
        let _ = self
            .manager_db_add_assistant_message(&msg, session_id)
            .await;

        self.push_context_record(
            ContextType::Status,
            Some("update_plan".to_string()),
            self.agent_id.clone(),
            Some("user".to_string()),
            msg,
            serde_json::json!({ "kind": "update_plan", "item_count": plan.items.len() }),
        );

        // New plan requires user approval — exit the loop immediately.
        if plan.status == PlanStatus::Planned {
            return LoopControl::Return(AgentOutcome::Plan(plan));
        }

        // Check if all items are done — if so, mark plan as completed
        if plan.status == PlanStatus::Executing
            && plan.items.iter().all(|i| {
                i.status == PlanItemStatus::Done || i.status == PlanItemStatus::Skipped
            })
        {
            if let Some(p) = &mut self.plan {
                p.status = PlanStatus::Completed;
            }
            if let Some(completed) = self.plan.clone() {
                self.write_plan_file(&completed);
                // Send final PlanUpdate event so the UI shows completed status
                if let Some(manager) = self.tools.get_manager() {
                    let agent_id = self
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    manager
                        .send_event(crate::agent_manager::AgentEvent::PlanUpdate {
                            agent_id,
                            plan: completed,
                        })
                        .await;
                }
            }
        }

        LoopControl::Continue
    }

    /// Mark all pending/in-progress plan items as done and emit a final
    /// PlanUpdate event.  Called when the agent signals `done` but hasn't
    /// explicitly completed every plan item.
    async fn auto_complete_plan(&mut self) {
        let completed = {
            let plan = match &mut self.plan {
                Some(p) if p.status == PlanStatus::Executing => p,
                _ => return,
            };
            for item in &mut plan.items {
                if item.status == PlanItemStatus::Pending
                    || item.status == PlanItemStatus::InProgress
                {
                    item.status = PlanItemStatus::Done;
                }
            }
            plan.status = PlanStatus::Completed;
            plan.clone()
        };
        self.write_plan_file(&completed);
        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            manager
                .send_event(crate::agent_manager::AgentEvent::PlanUpdate {
                    agent_id,
                    plan: completed,
                })
                .await;
        }
    }

    async fn handle_finalize_action(
        &mut self,
        packet: TaskPacket,
        _messages: &mut Vec<ChatMessage>,
        session_id: Option<&str>,
    ) -> LoopControl {
        info!("Agent finalized task: {}", packet.title);
        // Persist the structured final answer for the UI (DB-backed chat).
        let msg = serde_json::json!({ "type": "finalize_task", "packet": packet }).to_string();
        let _ = self
            .manager_db_add_assistant_message(&msg, session_id)
            .await;
        self.chat_history.push(ChatMessage::new("assistant", msg.clone()));
        self.push_context_record(
            ContextType::AssistantReply,
            Some("finalize_task".to_string()),
            self.agent_id.clone(),
            Some("user".to_string()),
            msg,
            serde_json::json!({ "kind": "finalize_task" }),
        );
        self.active_skill = None;
        LoopControl::Return(AgentOutcome::Task(packet))
    }

    /// Stream model output with thinking-token forwarding.
    ///
    /// Uses `chat_text_stream` (no format constraint) instead of `chat_json`
    /// so the model can emit prose "thinking" tokens before the JSON action.
    /// Thinking tokens are forwarded via `self.thinking_tx` and the full
    /// accumulated text is returned for action parsing.
    async fn stream_with_thinking(&self, messages: &[ChatMessage]) -> Result<String> {
        let mut stream = self
            .model_manager
            .chat_text_stream(&self.model_id, messages)
            .await?;
        let mut accumulated = String::new();
        let mut thinking_ended = false;

        while let Some(token_result) = TokioStreamExt::next(&mut stream).await {
            let token = token_result?;
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
        }

        // If thinking never ended (entire response was prose), signal done.
        if !thinking_ended {
            if let Some(tx) = &self.thinking_tx {
                let _ = tx.send(ThinkingEvent::Done);
            }
        }

        Ok(accumulated)
    }

    fn looks_like_json_action_start(text: &str) -> bool {
        if let Some(brace_idx) = text.rfind('{') {
            text[brace_idx..].contains("\"type\"")
        } else {
            false
        }
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
        if let Some(tx) = &self.repl_events_tx {
            let _ = tx.send(ReplEvent::Status {
                status: "working".to_string(),
                detail: Some("Running".to_string()),
            });
        }

        // Load plan from file if not already set (session resume).
        if self.plan.is_none() {
            if let Some(plan) = self.load_latest_plan() {
                info!("Loaded plan from file: {} ({} items)", plan.summary, plan.items.len());
                self.plan = Some(plan);
            }
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

        // Record start of loop in session store
        if let Some(manager) = self.tools.get_manager() {
            let aid = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            manager
                .add_chat_message(
                    &self.cfg.ws_root,
                    session_id.unwrap_or("default"),
                    &crate::state_fs::sessions::ChatMsg {
                        agent_id: aid.clone(),
                        from_id: "system".to_string(),
                        to_id: aid,
                        content: format!("Starting autonomous loop for task: {}", task),
                        timestamp: crate::util::now_ts_secs(),
                        is_observation: true,
                    },
                )
                .await;
        }

        let (mut messages, allowed_tools, mut read_paths) =
            self.prepare_loop_messages(&task);

        let mut tool_cache: HashMap<String, CachedToolObs> = HashMap::new();

        // Guardrail: repeated search with no matches means no progress.
        let mut empty_search_streak = 0usize;
        // Guardrail: repeated redundant tool calls.
        let mut redundant_tool_streak = 0usize;
        let mut last_tool_sig = String::new();
        // Guardrail: malformed action JSON can cause endless retries.
        let mut invalid_json_streak = 0usize;
        // Guardrail: identical assistant responses (looping).
        let mut last_assistant_response = String::new();
        let mut identical_response_streak = 0usize;
        let mut loop_nudge_count = 0usize;

        for iter_num in 0..self.cfg.max_iters {
            if let Some(tx) = &self.repl_events_tx {
                let _ = tx.send(ReplEvent::Iteration {
                    current: iter_num + 1,
                    max: self.cfg.max_iters,
                });
            }

            if self.is_cancelled().await {
                anyhow::bail!("run cancelled");
            }

            // Drain any user interrupt messages that arrived while we were working.
            // Cap at 5 per iteration to prevent context explosion from rapid user input.
            if let Some(rx) = &mut self.interrupt_rx {
                let mut interrupt_count = 0;
                while interrupt_count < 5 {
                    match rx.try_recv() {
                        Ok(msg) => {
                            info!("Injecting user interrupt message into loop context");
                            messages.push(ChatMessage::new(
                                "user",
                                format!("[User message received while you are working]\n{}", msg),
                            ));
                            interrupt_count += 1;
                        }
                        Err(_) => break,
                    }
                }
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
            if let Some(tx) = &self.repl_events_tx {
                let _ = tx.send(ReplEvent::Status {
                    status: "thinking".to_string(),
                    detail: Some("Thinking".to_string()),
                });
            }

            let summary_count = self.maybe_compact_model_messages(&mut messages, "loop_iter");
            self.emit_context_usage_event("loop_iter", &messages, summary_count)
                .await;

            // Ask model for the next action, streaming thinking tokens.
            let raw = self.stream_with_thinking(&messages).await?;

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
            if let Some(ctrl) = self
                .handle_repetition_check(
                    &raw,
                    &mut last_assistant_response,
                    &mut identical_response_streak,
                    &mut loop_nudge_count,
                    &mut messages,
                    session_id,
                )
                .await
            {
                match ctrl {
                    LoopControl::Return(outcome) => return Ok(outcome),
                    LoopControl::Continue => continue,
                }
            }

            let actions = match parse_all_actions(&raw) {
                Ok(v) => v,
                Err(e) => {
                    // Pure prose (no recognizable JSON action) → treat as Done message.
                    // This handles thinking models that sometimes return plain text
                    // without a JSON action wrapper.
                    if !raw.contains('{') {
                        vec![ModelAction::Done {
                            message: Some(raw.clone()),
                        }]
                    } else {
                        invalid_json_streak += 1;
                        if invalid_json_streak >= 4 {
                            let message = "I couldn't continue automatically because the model kept returning malformed structured output.".to_string();
                            let _ = self
                                .manager_db_add_assistant_message(&message, session_id)
                                .await;
                            self.active_skill = None;
                            return Ok(AgentOutcome::None);
                        }
                        messages.push(ChatMessage::new(
                            "user",
                            format!(
                                "Your previous response was not valid JSON ({e}). Respond with one or more JSON objects matching the tool schema. Raw was:\n{raw}"
                            ),
                        ));
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
                }
            };
            invalid_json_streak = 0;

            // Execute actions with parallel delegation support.
            //
            // Consecutive `delegate_to_agent` tool calls are batched and run
            // concurrently via `tokio::task::JoinSet`, each on a fresh engine
            // instance.  All other actions execute sequentially as before.
            let mut early_return: Option<AgentOutcome> = None;
            let mut actions = actions; // take ownership for drain/remove

            while !actions.is_empty() && early_return.is_none() {
                // Check if the front action is a delegate_to_agent tool call.
                let front_is_delegation = match &actions[0] {
                    ModelAction::Tool { tool, .. } => {
                        self.tools
                            .canonical_tool_name(tool)
                            .unwrap_or(tool.as_str())
                            == "delegate_to_agent"
                    }
                    _ => false,
                };

                if front_is_delegation {
                    // Collect a run of consecutive delegate_to_agent actions.
                    let batch_size = actions
                        .iter()
                        .take_while(|a| match a {
                            ModelAction::Tool { tool, .. } => {
                                self.tools
                                    .canonical_tool_name(tool)
                                    .unwrap_or(tool.as_str())
                                    == "delegate_to_agent"
                            }
                            _ => false,
                        })
                        .count();
                    let batch: Vec<ModelAction> = actions.drain(..batch_size).collect();

                    // Parse DelegateToAgentArgs from each action.
                    let mut delegation_args: Vec<tools::DelegateToAgentArgs> = Vec::new();
                    for action in batch {
                        if let ModelAction::Tool { tool, args } = action {
                            let normalized = tools::normalize_tool_args(&tool, &args);
                            match serde_json::from_value::<tools::DelegateToAgentArgs>(normalized) {
                                Ok(da) => delegation_args.push(da),
                                Err(e) => {
                                    messages.push(ChatMessage::new(
                                        "user",
                                        format!("Invalid delegate_to_agent args: {}", e),
                                    ));
                                }
                            }
                        }
                    }

                    if delegation_args.is_empty() {
                        continue;
                    }

                    // Permission check (once for the whole batch).
                    if let Some(allowed) = &allowed_tools {
                        if !self.is_tool_allowed(allowed, "delegate_to_agent") {
                            for da in &delegation_args {
                                messages.push(ChatMessage::new(
                                    "user",
                                    format!(
                                        "Tool 'delegate_to_agent' is not allowed for this agent. Delegation to '{}' blocked.",
                                        da.target_agent_id
                                    ),
                                ));
                            }
                            continue;
                        }
                    }

                    // Validate all delegations and collect spawn params.
                    struct DelegationSpawn {
                        manager: Arc<AgentManager>,
                        caller_id: String,
                        target_agent_id: String,
                        task: String,
                        parent_run_id: Option<String>,
                        depth: usize,
                        max_depth: usize,
                    }
                    let mut spawns: Vec<DelegationSpawn> = Vec::new();

                    for da in delegation_args {
                        match self.tools.builtins.validate_delegation(&da) {
                            Ok((manager, caller_id)) => {
                                spawns.push(DelegationSpawn {
                                    manager,
                                    caller_id,
                                    target_agent_id: da.target_agent_id,
                                    task: da.task,
                                    parent_run_id: self.run_id.clone(),
                                    depth: self.tools.builtins.delegation_depth(),
                                    max_depth: self.tools.builtins.max_delegation_depth(),
                                });
                            }
                            Err(e) => {
                                let rendered = format!(
                                    "tool_error: tool=delegate_to_agent target={} error={}",
                                    da.target_agent_id, e
                                );
                                self.upsert_observation(
                                    "error",
                                    "delegate_to_agent",
                                    rendered.clone(),
                                );
                                messages.push(ChatMessage::new(
                                    "user",
                                    format!(
                                        "Delegation to '{}' failed validation: {}",
                                        da.target_agent_id, e
                                    ),
                                ));
                            }
                        }
                    }

                    if spawns.is_empty() {
                        continue;
                    }

                    let ws_root = self.cfg.ws_root.clone();

                    // Spawn each delegation on a blocking thread with its own
                    // tokio runtime.  This sidesteps the non-Send future issue
                    // (run_agent_loop's future is !Send due to the model stream)
                    // while still allowing `block_in_place` inside tool execution.
                    let mut join_set = tokio::task::JoinSet::new();
                    for (spawn_idx, spawn) in spawns.into_iter().enumerate() {
                        let ws = ws_root.clone();
                        join_set.spawn_blocking(move || {
                            let rt = tokio::runtime::Builder::new_multi_thread()
                                .enable_all()
                                .worker_threads(1)
                                .build()
                                .expect("failed to create delegation runtime");
                            let target = spawn.target_agent_id.clone();
                            let result = rt.block_on(async move {
                                tools::run_delegation(
                                    spawn.manager,
                                    ws,
                                    spawn.caller_id,
                                    spawn.target_agent_id,
                                    spawn.task,
                                    spawn.parent_run_id,
                                    spawn.depth,
                                    spawn.max_depth,
                                )
                                .await
                            });
                            (spawn_idx, target, result)
                        });
                    }

                    // Await all and collect results.
                    let mut results: Vec<(usize, String, Result<tools::ToolResult>)> = Vec::new();
                    while let Some(join_result) = join_set.join_next().await {
                        match join_result {
                            Ok((idx, target, result)) => {
                                results.push((idx, target, result));
                            }
                            Err(join_err) => {
                                warn!("Delegation task panicked: {}", join_err);
                            }
                        }
                    }

                    // Sort by original spawn index for deterministic ordering.
                    results.sort_by_key(|(idx, _, _)| *idx);

                    // Merge results into messages.
                    for (_idx, target, result) in results {
                        match result {
                            Ok(tool_result) => {
                                let rendered = render_tool_result(&tool_result);
                                self.upsert_observation(
                                    "tool",
                                    "delegate_to_agent",
                                    rendered.clone(),
                                );
                                let _ = self
                                    .manager_db_add_observation(
                                        "delegate_to_agent",
                                        &rendered,
                                        session_id,
                                    )
                                    .await;
                                messages.push(ChatMessage::new(
                                    "user",
                                    Self::observation_text(
                                        "tool",
                                        &format!("delegate_to_agent({})", target),
                                        &rendered,
                                    ),
                                ));
                            }
                            Err(e) => {
                                let rendered = format!(
                                    "tool_error: tool=delegate_to_agent target={} error={}",
                                    target, e
                                );
                                self.upsert_observation(
                                    "error",
                                    "delegate_to_agent",
                                    rendered.clone(),
                                );
                                let _ = self
                                    .manager_db_add_observation(
                                        "delegate_to_agent",
                                        &rendered,
                                        session_id,
                                    )
                                    .await;
                                messages.push(ChatMessage::new(
                                    "user",
                                    format!("Delegation to '{}' failed: {}", target, e),
                                ));
                            }
                        }
                    }
                } else {
                    // Non-delegation action — handle sequentially.
                    let action = actions.remove(0);
                    match action {
                        ModelAction::Tool { tool, args } => {
                            match self
                                .handle_tool_action(
                                    tool,
                                    args,
                                    &allowed_tools,
                                    &mut messages,
                                    &mut tool_cache,
                                    &mut read_paths,
                                    &mut last_tool_sig,
                                    &mut redundant_tool_streak,
                                    &mut empty_search_streak,
                                    session_id,
                                )
                                .await
                            {
                                LoopControl::Return(outcome) => {
                                    early_return = Some(outcome);
                                }
                                LoopControl::Continue => {}
                            }
                        }
                        ModelAction::Patch { diff } => {
                            match self.handle_patch_action(diff, &mut messages).await {
                                LoopControl::Return(outcome) => {
                                    early_return = Some(outcome);
                                }
                                LoopControl::Continue => {}
                            }
                        }
                        ModelAction::FinalizeTask { packet } => {
                            match self
                                .handle_finalize_action(packet, &mut messages, session_id)
                                .await
                            {
                                LoopControl::Return(outcome) => {
                                    early_return = Some(outcome);
                                }
                                LoopControl::Continue => {}
                            }
                        }
                        ModelAction::UpdatePlan { summary, items } => {
                            match self
                                .handle_update_plan_action(summary, items, session_id)
                                .await
                            {
                                LoopControl::Return(outcome) => {
                                    early_return = Some(outcome);
                                }
                                LoopControl::Continue => {}
                            }
                        }
                        ModelAction::Done { message } => {
                            let msg =
                                message.unwrap_or_else(|| "Task completed.".to_string());
                            info!("Agent signalled done: {}", msg);

                            // Auto-complete any remaining plan items when the
                            // agent finishes — the model often forgets to emit
                            // a final update_plan marking everything done.
                            self.auto_complete_plan().await;

                            self.push_context_record(
                                ContextType::Status,
                                Some("done".to_string()),
                                self.agent_id.clone(),
                                Some("user".to_string()),
                                msg.clone(),
                                serde_json::json!({ "kind": "done" }),
                            );
                            let _ = self
                                .manager_db_add_assistant_message(&msg, session_id)
                                .await;
                            self.active_skill = None;
                            // In plan mode, return the plan for user approval.
                            if self.plan_mode {
                                if let Some(plan) = &mut self.plan {
                                    plan.status = PlanStatus::Planned;
                                    early_return = Some(AgentOutcome::Plan(plan.clone()));
                                    continue;
                                }
                            }
                            early_return = Some(AgentOutcome::None);
                        }
                        ModelAction::EnterPlanMode { reason } => {
                            info!("Agent requested plan mode: {:?}", reason);
                            early_return = Some(AgentOutcome::PlanModeRequested { reason });
                        }
                    }
                }
            }
            if let Some(outcome) = early_return {
                return Ok(outcome);
            }
        }

        self.active_skill = None;
        let fallback = "I couldn't complete this automatically within the current tool loop limit. Please refine the request and try again."
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
        self.parent_agent_id
            .clone()
            .unwrap_or_else(|| "user".to_string())
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
            messages.insert(start, ChatMessage::new("user", summary.clone()));

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
            ts: crate::util::now_ts_secs(),
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
        // When a skill is active and declares allowed-tools, those take
        // precedence — the agent can only use the tools the skill permits.
        if let Some(skill) = &self.active_skill {
            if !skill.allowed_tools.is_empty() {
                let allowed = skill
                    .allowed_tools
                    .iter()
                    .filter_map(|tool| {
                        if let Some(name) = tools::canonical_tool_name(tool) {
                            return Some(name.to_string());
                        }
                        if self.tools.has_skill_tool(tool) {
                            return Some(tool.to_string());
                        }
                        None
                    })
                    .collect::<HashSet<String>>();
                return Some(allowed);
            }
        }

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
            .filter_map(|tool| {
                // Builtin tools are resolved via canonical_tool_name.
                if let Some(name) = tools::canonical_tool_name(tool) {
                    return Some(name.to_string());
                }
                // Skill tools are recognised by the registry.
                if self.tools.has_skill_tool(tool) {
                    return Some(tool.to_string());
                }
                None
            })
            .collect::<HashSet<String>>();

        Some(allowed)
    }

    fn is_tool_allowed(&self, allowed: &HashSet<String>, requested_tool: &str) -> bool {
        // Builtin tools: check via canonical name.
        if let Some(canonical) = tools::canonical_tool_name(requested_tool) {
            return allowed.contains(canonical);
        }
        // Skill tools: check by exact name.
        allowed.contains(requested_tool)
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

    // -----------------------------------------------------------------------
    // Plan file persistence (.linggen/plans/<slug>.md)
    // -----------------------------------------------------------------------

    fn plans_dir(&self) -> PathBuf {
        self.plans_dir_override
            .clone()
            .unwrap_or_else(|| crate::paths::plans_dir())
    }

    /// Convert a plan summary into a filesystem-safe slug.
    /// Takes first few meaningful words, lowercased, joined by hyphens.
    /// e.g. "Refactor logging module" → "refactor-logging-module"
    fn slugify_summary(summary: &str) -> String {
        let slug: String = summary
            .chars()
            .map(|c| if c.is_alphanumeric() || c == ' ' { c.to_ascii_lowercase() } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .take(5) // max 5 words
            .collect::<Vec<_>>()
            .join("-");
        if slug.is_empty() { "plan".to_string() } else { slug }
    }

    /// Find a unique file path in the plans directory for the given slug.
    /// Returns `<slug>.md`, or `<slug>-2.md`, `<slug>-3.md`, etc. on collision.
    fn unique_plan_path(&self, slug: &str) -> PathBuf {
        let dir = self.plans_dir();
        let base = dir.join(format!("{}.md", slug));
        if !base.exists() {
            return base;
        }
        for i in 2.. {
            let path = dir.join(format!("{}-{}.md", slug, i));
            if !path.exists() {
                return path;
            }
        }
        unreachable!()
    }

    fn write_plan_file(&mut self, plan: &Plan) {
        let dir = self.plans_dir();
        let _ = std::fs::create_dir_all(&dir);

        // Determine file path: reuse existing if we already have one, otherwise generate new.
        let path = if let Some(existing) = &self.plan_file {
            existing.clone()
        } else {
            let slug = Self::slugify_summary(&plan.summary);
            let p = self.unique_plan_path(&slug);
            self.plan_file = Some(p.clone());
            p
        };

        let status_icon = |s: &PlanItemStatus| match s {
            PlanItemStatus::Pending => "[ ]",
            PlanItemStatus::InProgress => "[~]",
            PlanItemStatus::Done => "[x]",
            PlanItemStatus::Skipped => "[-]",
        };

        let origin_str = match plan.origin {
            PlanOrigin::UserRequested => "user_requested",
            PlanOrigin::ModelManaged => "model_managed",
        };

        let mut md = format!("# Plan: {}\n\n", plan.summary);
        md.push_str(&format!("**Status:** {}\n\n", serde_json::to_string(&plan.status)
            .unwrap_or_default().trim_matches('"')));
        md.push_str(&format!("**Origin:** {}\n\n", origin_str));
        for item in &plan.items {
            md.push_str(&format!("- {} {}\n", status_icon(&item.status), item.title));
            if let Some(desc) = &item.description {
                md.push_str(&format!("  {}\n", desc));
            }
        }

        if let Err(e) = std::fs::write(&path, &md) {
            warn!("Failed to write plan file {}: {}", path.display(), e);
        }
    }

    /// Parse a single plan markdown file into a Plan + its path.
    fn parse_plan_file(path: &Path) -> Option<Plan> {
        let content = std::fs::read_to_string(path).ok()?;

        let mut summary = String::new();
        let mut status = PlanStatus::Executing;
        let mut origin = PlanOrigin::ModelManaged;
        let mut items = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# Plan: ") {
                summary = trimmed.strip_prefix("# Plan: ").unwrap_or("").to_string();
            } else if trimmed.starts_with("**Status:**") {
                let s = trimmed
                    .strip_prefix("**Status:**")
                    .unwrap_or("")
                    .trim();
                status = match s {
                    "planned" => PlanStatus::Planned,
                    "approved" => PlanStatus::Approved,
                    "executing" => PlanStatus::Executing,
                    "completed" => PlanStatus::Completed,
                    _ => PlanStatus::Executing,
                };
            } else if trimmed.starts_with("**Origin:**") {
                let o = trimmed
                    .strip_prefix("**Origin:**")
                    .unwrap_or("")
                    .trim();
                origin = match o {
                    "user_requested" => PlanOrigin::UserRequested,
                    _ => PlanOrigin::ModelManaged,
                };
            } else if trimmed.starts_with("- [") {
                let (item_status, title) = if trimmed.starts_with("- [x] ") {
                    (PlanItemStatus::Done, trimmed.strip_prefix("- [x] ").unwrap_or(""))
                } else if trimmed.starts_with("- [~] ") {
                    (PlanItemStatus::InProgress, trimmed.strip_prefix("- [~] ").unwrap_or(""))
                } else if trimmed.starts_with("- [-] ") {
                    (PlanItemStatus::Skipped, trimmed.strip_prefix("- [-] ").unwrap_or(""))
                } else if trimmed.starts_with("- [ ] ") {
                    (PlanItemStatus::Pending, trimmed.strip_prefix("- [ ] ").unwrap_or(""))
                } else {
                    continue;
                };
                items.push(PlanItem {
                    title: title.to_string(),
                    description: None,
                    status: item_status,
                });
            } else if line.starts_with("  ") && !items.is_empty() {
                // Description line for the last item (indented with 2+ spaces).
                if let Some(last) = items.last_mut() {
                    last.description = Some(line.trim().to_string());
                }
            }
        }

        if summary.is_empty() && items.is_empty() {
            return None;
        }

        Some(Plan { summary, items, status, origin })
    }

    /// Load the most recent non-completed plan from ~/.linggen/plans/.
    /// Sets `self.plan_file` so subsequent writes update the same file.
    fn load_latest_plan(&mut self) -> Option<Plan> {
        let dir = self.plans_dir();
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
            .collect();
        // Sort by modified time descending (most recent first).
        entries.sort_by(|a, b| {
            let ta = a.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let tb = b.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            tb.cmp(&ta)
        });
        // Find the most recent non-completed plan.
        for entry in entries {
            let path = entry.path();
            if let Some(plan) = Self::parse_plan_file(&path) {
                if plan.status != PlanStatus::Completed {
                    self.plan_file = Some(path);
                    return Some(plan);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_engine(tmp: &std::path::Path) -> AgentEngine {
        let model_manager = Arc::new(
            crate::agent_manager::models::ModelManager::new(vec![]),
        );
        let mut engine = AgentEngine::new(
            EngineConfig {
                ws_root: tmp.to_path_buf(),
                max_iters: 1,
                write_safety_mode: crate::config::WriteSafetyMode::Off,
                prompt_loop_breaker: None,
            },
            model_manager,
            "test".to_string(),
            AgentRole::Coder,
        )
        .unwrap();
        engine.plans_dir_override = Some(tmp.join(".linggen").join("plans"));
        engine
    }

    #[test]
    fn plan_file_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut engine = make_test_engine(tmp.path());

        let plan = Plan {
            summary: "Refactor logging module".to_string(),
            items: vec![
                PlanItem {
                    title: "Read existing code".to_string(),
                    description: Some("Understand the current structure".to_string()),
                    status: PlanItemStatus::Done,
                },
                PlanItem {
                    title: "Extract helper function".to_string(),
                    description: None,
                    status: PlanItemStatus::InProgress,
                },
                PlanItem {
                    title: "Update tests".to_string(),
                    description: None,
                    status: PlanItemStatus::Pending,
                },
                PlanItem {
                    title: "Old migration step".to_string(),
                    description: None,
                    status: PlanItemStatus::Skipped,
                },
            ],
            status: PlanStatus::Executing,
            origin: PlanOrigin::ModelManaged,
        };

        engine.write_plan_file(&plan);

        // Verify file was written to plans dir as <slug>.md
        let plan_path = engine.plan_file.as_ref().expect("plan_file should be set");
        assert!(plan_path.exists());
        assert_eq!(plan_path.file_name().unwrap(), "refactor-logging-module.md");
        assert!(plan_path.parent().unwrap().ends_with(".linggen/plans"));

        // Load it back via load_latest_plan
        let mut engine2 = make_test_engine(tmp.path());
        let loaded = engine2.load_latest_plan().expect("should load plan");

        assert_eq!(loaded.summary, plan.summary);
        assert_eq!(loaded.status, PlanStatus::Executing);
        assert_eq!(loaded.origin, PlanOrigin::ModelManaged);
        assert_eq!(loaded.items.len(), 4);
        assert_eq!(loaded.items[0].title, "Read existing code");
        assert_eq!(loaded.items[0].status, PlanItemStatus::Done);
        assert_eq!(loaded.items[0].description.as_deref(), Some("Understand the current structure"));
        assert_eq!(loaded.items[1].title, "Extract helper function");
        assert_eq!(loaded.items[1].status, PlanItemStatus::InProgress);
        assert_eq!(loaded.items[2].status, PlanItemStatus::Pending);
        assert_eq!(loaded.items[3].status, PlanItemStatus::Skipped);
    }

    #[test]
    fn plan_file_slug_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let mut engine = make_test_engine(tmp.path());

        let plan1 = Plan {
            summary: "Fix auth".to_string(),
            items: vec![PlanItem {
                title: "Step 1".to_string(),
                description: None,
                status: PlanItemStatus::Done,
            }],
            status: PlanStatus::Completed,
            origin: PlanOrigin::ModelManaged,
        };
        engine.write_plan_file(&plan1);
        let path1 = engine.plan_file.clone().unwrap();
        assert_eq!(path1.file_name().unwrap(), "fix-auth.md");

        // Reset plan_file to force a new file for the second plan.
        engine.plan_file = None;
        let plan2 = Plan {
            summary: "Fix auth".to_string(),
            items: vec![PlanItem {
                title: "Step A".to_string(),
                description: None,
                status: PlanItemStatus::Pending,
            }],
            status: PlanStatus::Executing,
            origin: PlanOrigin::ModelManaged,
        };
        engine.write_plan_file(&plan2);
        let path2 = engine.plan_file.clone().unwrap();
        assert_eq!(path2.file_name().unwrap(), "fix-auth-2.md");
        assert_ne!(path1, path2);
    }

    #[test]
    fn slugify_summary_examples() {
        assert_eq!(AgentEngine::slugify_summary("Refactor logging module"), "refactor-logging-module");
        assert_eq!(AgentEngine::slugify_summary("Fix the auth bug!"), "fix-the-auth-bug");
        assert_eq!(AgentEngine::slugify_summary("Add user authentication & session mgmt for v2"), "add-user-authentication-session-mgmt");
        assert_eq!(AgentEngine::slugify_summary(""), "plan");
        assert_eq!(AgentEngine::slugify_summary("   "), "plan");
    }

    #[test]
    fn load_latest_plan_returns_none_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let mut engine = make_test_engine(tmp.path());
        assert!(engine.load_latest_plan().is_none());
    }

    #[test]
    fn load_latest_plan_skips_completed() {
        let tmp = tempfile::tempdir().unwrap();
        let mut engine = make_test_engine(tmp.path());

        // Write a completed plan.
        let plan = Plan {
            summary: "Old task".to_string(),
            items: vec![PlanItem {
                title: "Done".to_string(),
                description: None,
                status: PlanItemStatus::Done,
            }],
            status: PlanStatus::Completed,
            origin: PlanOrigin::ModelManaged,
        };
        engine.write_plan_file(&plan);

        // A fresh engine should NOT load the completed plan.
        let mut engine2 = make_test_engine(tmp.path());
        assert!(engine2.load_latest_plan().is_none());
    }
}
