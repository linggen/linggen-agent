use super::types::*;
use crate::engine::actions::ModelAction;
use crate::engine::render::render_tool_result;
use crate::engine::streaming::check_context_staleness;
use crate::engine::tools::{self, ToolCall};
use crate::ollama::ChatMessage;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{info, warn};

impl AgentEngine {
    /// Handle a batch of consecutive `delegate_to_agent` actions.
    /// Returns `Some(outcome)` if the loop should exit early.
    pub(crate) async fn handle_delegation_batch(
        &mut self,
        batch: Vec<ModelAction>,
        allowed_tools: &Option<HashSet<String>>,
        messages: &mut Vec<ChatMessage>,
        session_id: Option<&str>,
    ) -> Option<AgentOutcome> {
        use crate::agent_manager::AgentManager;

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
            return None;
        }

        // Permission check (once for the whole batch).
        if let Some(allowed) = allowed_tools {
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
                return None;
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
                    self.upsert_observation("error", "delegate_to_agent", rendered.clone());
                    messages.push(ChatMessage::new(
                        "user",
                        format!("Delegation to '{}' failed validation: {}", da.target_agent_id, e),
                    ));
                }
            }
        }

        if spawns.is_empty() {
            return None;
        }

        let ws_root = self.cfg.ws_root.clone();

        // Spawn each delegation on a blocking thread with its own tokio runtime.
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
                        spawn.manager, ws, spawn.caller_id, spawn.target_agent_id,
                        spawn.task, spawn.parent_run_id, spawn.depth, spawn.max_depth,
                    )
                    .await
                });
                (spawn_idx, target, result)
            });
        }

        // Await all and collect results.
        let mut results: Vec<(usize, String, anyhow::Result<tools::ToolResult>)> = Vec::new();
        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((idx, target, result)) => results.push((idx, target, result)),
                Err(join_err) => warn!("Delegation task panicked: {}", join_err),
            }
        }
        results.sort_by_key(|(idx, _, _)| *idx);

        // Merge results into messages.
        for (_idx, target, result) in results {
            match result {
                Ok(tool_result) => {
                    let rendered = render_tool_result(&tool_result);
                    self.upsert_observation("tool", "delegate_to_agent", rendered.clone());
                    let _ = self
                        .manager_db_add_observation("delegate_to_agent", &rendered, session_id)
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
                        "tool_error: tool=delegate_to_agent target={} error={}", target, e
                    );
                    self.upsert_observation("error", "delegate_to_agent", rendered.clone());
                    let _ = self
                        .manager_db_add_observation("delegate_to_agent", &rendered, session_id)
                        .await;
                    messages.push(ChatMessage::new(
                        "user",
                        format!("Delegation to '{}' failed: {}", target, e),
                    ));
                }
            }
        }
        None
    }

    /// Handle a batch of parallelizable tool actions (read-only + non-conflicting writes).
    /// Returns `Some(outcome)` if the loop should exit early.
    pub(crate) async fn handle_parallel_batch(
        &mut self,
        batch: Vec<ModelAction>,
        state: &mut LoopState,
        session_id: Option<&str>,
    ) -> Option<AgentOutcome> {
        // Phase 1: pre-execute each tool sequentially (needs &mut self).
        let mut ready: Vec<(usize, ToolCall, ReadyExec)> = Vec::new();
        for (idx, action) in batch.into_iter().enumerate() {
            if let ModelAction::Tool { tool, args } = action {
                match self
                    .pre_execute_tool(
                        tool, args, &state.allowed_tools, &mut state.messages,
                        &mut state.tool_cache, &mut state.read_paths,
                        &mut state.last_tool_sig, &mut state.redundant_tool_streak,
                        session_id,
                    )
                    .await
                {
                    PreExecOutcome::Blocked(LoopControl::Return(outcome)) => {
                        return Some(outcome);
                    }
                    PreExecOutcome::Blocked(LoopControl::Continue) => {}
                    PreExecOutcome::Ready(call, exec) => {
                        ready.push((idx, call, exec));
                    }
                }
            }
        }
        if ready.is_empty() {
            return None;
        }

        // Phase 2: execute tools in parallel via std::thread::scope.
        let tools_ref = &self.tools;
        let cached_hash = self.cached_system_prompt.as_ref().map(|c| c.input_hash);
        let staleness_ws_root = &self.cfg.ws_root;
        let staleness_memory_dir = self.tools.memory_dir().map(|p| p.as_path());
        let (results, prompt_stale): (Vec<(usize, ReadyExec, anyhow::Result<tools::ToolResult>)>, bool) =
            tokio::task::block_in_place(|| {
                std::thread::scope(|scope| {
                    let staleness_handle = scope.spawn(|| {
                        check_context_staleness(cached_hash, staleness_ws_root, staleness_memory_dir)
                    });
                    let handles: Vec<_> = ready
                        .into_iter()
                        .map(|(idx, call, exec)| {
                            let handle = scope.spawn(move || {
                                let rt = tokio::runtime::Builder::new_current_thread()
                                    .enable_all()
                                    .build()
                                    .expect("failed to create tool runtime");
                                let _guard = rt.enter();
                                tools_ref.execute(call)
                            });
                            (idx, exec, handle)
                        })
                        .collect();
                    let tool_results: Vec<_> = handles
                        .into_iter()
                        .map(|(idx, exec, handle)| {
                            let result = handle.join().unwrap_or_else(|_| {
                                Err(anyhow::anyhow!("tool execution panicked"))
                            });
                            (idx, exec, result)
                        })
                        .collect();
                    let stale = staleness_handle.join().unwrap_or(false);
                    (tool_results, stale)
                })
            });

        // If context files changed, rebuild the system prompt cache.
        if prompt_stale {
            info!("Context files changed during tool execution; rebuilding system prompt cache");
            let (new_stable, new_hash) = self.build_stable_system_content();
            self.cached_system_prompt = Some(CachedSystemPrompt {
                input_hash: new_hash,
                content: new_stable.clone(),
            });
            if !state.messages.is_empty() {
                let mut sys = new_stable;
                if let Some(dyn_start) = state.messages[0].content.find("\n\n## Task List") {
                    sys.push_str(&state.messages[0].content[dyn_start..]);
                } else if let Some(dyn_start) = state.messages[0].content.find("\n\nYou are in PLAN MODE") {
                    sys.push_str(&state.messages[0].content[dyn_start..]);
                }
                state.messages[0] = ChatMessage::new("system", sys);
            }
        }

        // Phase 3: post-execute each result sequentially.
        let mut results = results;
        results.sort_by_key(|(idx, _, _)| *idx);
        for (_idx, exec, result) in results {
            match self
                .post_execute_tool(
                    exec, result, &mut state.messages,
                    &mut state.tool_cache, &mut state.empty_search_streak, session_id,
                )
                .await
            {
                LoopControl::Return(outcome) => return Some(outcome),
                LoopControl::Continue => {}
            }
        }
        None
    }

    /// Dispatch a single non-delegation, non-parallel action.
    /// Returns `Some(outcome)` if the loop should exit early.
    pub(crate) async fn dispatch_sequential_action(
        &mut self,
        action: ModelAction,
        state: &mut LoopState,
        actions_remaining: bool,
        session_id: Option<&str>,
    ) -> Option<AgentOutcome> {
        match action {
            ModelAction::Tool { tool, args } => {
                match self
                    .handle_tool_action(
                        tool, args, &state.allowed_tools, &mut state.messages,
                        &mut state.tool_cache, &mut state.read_paths,
                        &mut state.last_tool_sig, &mut state.redundant_tool_streak,
                        &mut state.empty_search_streak, session_id,
                    )
                    .await
                {
                    LoopControl::Return(outcome) => return Some(outcome),
                    LoopControl::Continue => {}
                }
            }
            ModelAction::Patch { diff } => {
                match self.handle_patch_action(diff, &mut state.messages).await {
                    LoopControl::Return(outcome) => return Some(outcome),
                    LoopControl::Continue => {}
                }
            }
            ModelAction::FinalizeTask { packet } => {
                match self.handle_finalize_action(packet, &mut state.messages, session_id).await {
                    LoopControl::Return(outcome) => return Some(outcome),
                    LoopControl::Continue => {}
                }
            }
            ModelAction::UpdatePlan { summary, items } => {
                match self.handle_update_plan_action(summary, items, session_id).await {
                    LoopControl::Return(outcome) => return Some(outcome),
                    LoopControl::Continue => {
                        if !actions_remaining {
                            state.messages.push(ChatMessage::new(
                                "user",
                                self.nudge(crate::prompts::NUDGE_PLAN_ONLY, &[]),
                            ));
                        }
                    }
                }
            }
            ModelAction::Done { message } => {
                let msg = message.unwrap_or_else(|| "Task completed.".to_string());
                info!("Agent signalled done: {}", msg);
                self.auto_complete_plan().await;
                self.push_context_record(
                    ContextType::Status, Some("done".to_string()),
                    self.agent_id.clone(), Some("user".to_string()),
                    msg.clone(), serde_json::json!({ "kind": "done" }),
                );
                let _ = self.manager_db_add_assistant_message(&msg, session_id).await;
                self.active_skill = None;
                if self.plan_mode {
                    if let Some(plan) = &mut self.plan {
                        plan.status = PlanStatus::Planned;
                        return Some(AgentOutcome::Plan(plan.clone()));
                    }
                }
                return Some(AgentOutcome::None);
            }
            ModelAction::EnterPlanMode { reason } => {
                info!("Agent requested plan mode: {:?}", reason);
                return Some(AgentOutcome::PlanModeRequested { reason });
            }
        }
        None
    }
}
