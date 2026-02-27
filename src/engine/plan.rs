use super::types::*;
use crate::config::AgentPolicyCapability;
use crate::engine::actions::PlanItemUpdate;
use crate::engine::patch::validate_unified_diff;
use crate::ollama::ChatMessage;
use tracing::{info, warn};

impl AgentEngine {
    pub(crate) async fn handle_patch_action(
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

    pub(crate) async fn handle_update_plan_action(
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
            // If we're NOT in plan mode but the existing plan was "Planned" (stale
            // from a previous /plan run), promote it to "Executing" so the agent
            // continues working instead of blocking on approval.
            if !self.plan_mode && existing.status == PlanStatus::Planned {
                existing.status = PlanStatus::Executing;
            }
            existing.clone()
        } else {
            // Create new plan.
            // User-requested plans (plan_mode) need approval; model task lists execute immediately.
            let status = if self.plan_mode {
                PlanStatus::Planned
            } else {
                PlanStatus::Executing
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
                plan_text: None,
            };
            self.plan = Some(plan.clone());
            plan
        };

        // Persist plan to session dir
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
    pub(crate) async fn auto_complete_plan(&mut self) {
        let completed = {
            let plan = match &mut self.plan {
                Some(p) if p.status == PlanStatus::Executing => p,
                _ => return,
            };
            for item in &mut plan.items {
                if item.status == PlanItemStatus::Pending
                    || item.status == PlanItemStatus::InProgress
                {
                    item.status = PlanItemStatus::Skipped;
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

    /// Called when the model signals plan completion (via ExitPlanMode tool or
    /// fallback: done in plan_mode). Extracts the plan text, persists it, and
    /// returns the Plan outcome for user approval.
    pub(crate) async fn finalize_plan_mode(&mut self, plan_text: String) -> AgentOutcome {
        let summary = Self::extract_plan_summary(&plan_text);
        let plan = Plan {
            summary,
            items: vec![],
            status: PlanStatus::Planned,
            plan_text: Some(plan_text),
        };
        self.plan = Some(plan.clone());
        self.write_plan_file(&plan);

        // Emit PlanUpdate event
        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            manager
                .send_event(crate::agent_manager::AgentEvent::PlanUpdate {
                    agent_id,
                    plan: plan.clone(),
                })
                .await;
        }

        AgentOutcome::Plan(plan)
    }

    /// Extract a summary from the plan text (first heading or first non-empty line).
    pub(crate) fn extract_plan_summary(text: &str) -> String {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") {
                return trimmed.strip_prefix("# ").unwrap_or(trimmed).to_string();
            }
            if !trimmed.is_empty() {
                return trimmed.chars().take(80).collect();
            }
        }
        "Plan".to_string()
    }

    pub(crate) async fn handle_finalize_action(
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

    // -----------------------------------------------------------------------
    // Per-session plan file persistence
    // -----------------------------------------------------------------------

    pub(crate) fn write_plan_file(&self, plan: &Plan) {
        let Some(dir) = &self.session_plan_dir else { return };
        let _ = std::fs::create_dir_all(dir);
        let path = dir.join("plan.md");
        let md = if let Some(text) = &plan.plan_text {
            text.clone()
        } else {
            // Item-based fallback (for update_plan progress tracking)
            let status_icon = |s: &PlanItemStatus| match s {
                PlanItemStatus::Pending => "[ ]",
                PlanItemStatus::InProgress => "[~]",
                PlanItemStatus::Done => "[x]",
                PlanItemStatus::Skipped => "[-]",
            };
            let mut out = format!("# {}\n\n", plan.summary);
            for item in &plan.items {
                out.push_str(&format!("- {} {}\n", status_icon(&item.status), item.title));
                if let Some(desc) = &item.description {
                    for line in desc.lines() {
                        out.push_str(&format!("  {}\n", line));
                    }
                }
            }
            out
        };
        let _ = std::fs::write(&path, &md);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_test_engine(tmp: &std::path::Path) -> AgentEngine {
        let model_manager = Arc::new(
            crate::agent_manager::models::ModelManager::new(vec![]),
        );
        AgentEngine::new(
            EngineConfig {
                ws_root: tmp.to_path_buf(),
                max_iters: 1,
                write_safety_mode: crate::config::WriteSafetyMode::Off,
                tool_permission_mode: crate::config::ToolPermissionMode::Auto,
                prompt_loop_breaker: None,
            },
            model_manager,
            "test".to_string(),
            AgentRole::Coder,
        )
        .unwrap()
    }

    #[test]
    fn write_plan_file_per_session() {
        let tmp = tempfile::tempdir().unwrap();
        let mut engine = make_test_engine(tmp.path());
        let session_dir = tmp.path().join("sessions").join("s1");
        engine.session_plan_dir = Some(session_dir.clone());

        let plan = Plan {
            summary: "Refactor logging".to_string(),
            items: vec![
                PlanItem {
                    title: "Read code".to_string(),
                    description: Some("Understand structure".to_string()),
                    status: PlanItemStatus::Done,
                },
                PlanItem {
                    title: "Update tests".to_string(),
                    description: None,
                    status: PlanItemStatus::Pending,
                },
            ],
            status: PlanStatus::Executing,
            plan_text: None,
        };

        engine.write_plan_file(&plan);

        let plan_path = session_dir.join("plan.md");
        assert!(plan_path.exists());
        let content = std::fs::read_to_string(&plan_path).unwrap();
        assert!(content.contains("# Refactor logging"));
        assert!(content.contains("[x] Read code"));
        assert!(content.contains("[ ] Update tests"));
        assert!(content.contains("  Understand structure"));
    }

    #[test]
    fn write_plan_file_free_form() {
        let tmp = tempfile::tempdir().unwrap();
        let mut engine = make_test_engine(tmp.path());
        let session_dir = tmp.path().join("sessions").join("s2");
        engine.session_plan_dir = Some(session_dir.clone());

        let plan_text = "# Add avatar upload\n\n1. Add endpoint\n2. Add model\n";
        let plan = Plan {
            summary: "Add avatar upload".to_string(),
            items: vec![],
            status: PlanStatus::Planned,
            plan_text: Some(plan_text.to_string()),
        };

        engine.write_plan_file(&plan);

        let plan_path = session_dir.join("plan.md");
        assert!(plan_path.exists());
        let content = std::fs::read_to_string(&plan_path).unwrap();
        assert_eq!(content, plan_text);
    }

    #[test]
    fn write_plan_file_no_dir_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = make_test_engine(tmp.path());
        // session_plan_dir is None — should not panic or write anything

        let plan = Plan {
            summary: "Test".to_string(),
            items: vec![],
            status: PlanStatus::Executing,
            plan_text: None,
        };

        engine.write_plan_file(&plan);
        // No assertion needed — just verify no panic
    }

    #[test]
    fn extract_plan_summary_examples() {
        assert_eq!(
            AgentEngine::extract_plan_summary("# My Plan\n\nSome details"),
            "My Plan"
        );
        assert_eq!(
            AgentEngine::extract_plan_summary("First line without heading"),
            "First line without heading"
        );
        assert_eq!(AgentEngine::extract_plan_summary(""), "Plan");
        assert_eq!(AgentEngine::extract_plan_summary("   \n  "), "Plan");
    }
}
