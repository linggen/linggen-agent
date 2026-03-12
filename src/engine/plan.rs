use super::types::*;
use tracing::info;

impl AgentEngine {
    /// Called when the model signals plan completion (via ExitPlanMode tool or
    /// fallback: done in plan_mode). Emits a PlanUpdate SSE event so the
    /// PlanBlock renders in the UI, and returns `AgentOutcome::Plan` for the
    /// server to store as pending. The user reviews and approves via PlanBlock
    /// buttons (CC-aligned — no modal AskUser dialog).
    pub(crate) async fn finalize_plan_mode(&mut self, plan_text: String) -> AgentOutcome {
        let summary = Self::extract_plan_summary(&plan_text);
        // Preserve items from any prior UpdatePlan call during plan mode.
        let items = self.plan.as_ref()
            .map(|p| p.items.clone())
            .unwrap_or_default();
        let plan = Plan {
            summary,
            status: PlanStatus::Planned,
            plan_text,
            items,
        };
        info!("finalize_plan_mode: status={:?} items={}", plan.status, plan.items.len());
        self.persist_and_emit_plan(plan.clone()).await;
        AgentOutcome::Plan(plan)
    }

    /// Store the plan in memory and emit a PlanUpdate SSE event.
    pub(crate) async fn persist_and_emit_plan(&mut self, plan: Plan) {
        info!("persist_and_emit_plan: status={:?} items={}", plan.status, plan.items.len());
        self.plan = Some(plan);

        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let plan = self.plan.clone().unwrap();
            manager
                .send_event(crate::agent_manager::AgentEvent::PlanUpdate {
                    agent_id,
                    plan,
                })
                .await;
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
