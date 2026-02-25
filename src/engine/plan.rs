use super::types::*;
use crate::config::AgentPolicyCapability;
use crate::engine::actions::PlanItemUpdate;
use crate::engine::patch::validate_unified_diff;
use crate::ollama::ChatMessage;
use std::path::{Path, PathBuf};
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
                existing.origin = PlanOrigin::ModelManaged;
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
    // Plan file persistence (.linggen/plans/<slug>.md)
    // -----------------------------------------------------------------------

    pub(crate) fn plans_dir(&self) -> PathBuf {
        self.plans_dir_override
            .clone()
            .unwrap_or_else(|| crate::paths::plans_dir())
    }

    /// Convert a plan summary into a filesystem-safe slug.
    /// Takes first few meaningful words, lowercased, joined by hyphens.
    /// e.g. "Refactor logging module" → "refactor-logging-module"
    pub(crate) fn slugify_summary(summary: &str) -> String {
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

    pub(crate) fn write_plan_file(&mut self, plan: &Plan) {
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
    pub(crate) fn parse_plan_file(path: &Path) -> Option<Plan> {
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
    pub(crate) fn load_latest_plan(&mut self) -> Option<Plan> {
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
        // Find the most recent non-completed, non-planned plan.
        // Plans with status "Planned" are waiting for explicit user approval
        // and should not be auto-resumed.
        for entry in entries {
            let path = entry.path();
            if let Some(plan) = Self::parse_plan_file(&path) {
                if plan.status != PlanStatus::Completed
                    && plan.status != PlanStatus::Planned
                {
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
    use std::sync::Arc;

    fn make_test_engine(tmp: &std::path::Path) -> AgentEngine {
        let model_manager = Arc::new(
            crate::agent_manager::models::ModelManager::new(vec![]),
        );
        let mut engine = AgentEngine::new(
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
