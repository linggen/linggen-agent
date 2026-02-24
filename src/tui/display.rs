/// Display block types for the TUI output area.
///
/// Each block represents a visual section in the terminal output:
/// user messages, agent messages (markdown-rendered), tool step groups,
/// subagent delegations, plan blocks, and system messages.

#[derive(Debug, Clone)]
pub enum DisplayBlock {
    UserMessage {
        text: String,
        image_count: usize,
    },
    AgentMessage {
        agent_id: String,
        text: String,
    },
    SystemMessage {
        text: String,
    },
    ToolGroup {
        agent_id: String,
        steps: Vec<ToolStep>,
        collapsed: bool,
        estimated_tokens: Option<usize>,
        duration_secs: Option<u64>,
    },
    SubagentGroup {
        entries: Vec<SubagentEntry>,
    },
    PlanBlock {
        summary: String,
        items: Vec<PlanDisplayItem>,
        status: String,
    },
    ChangeReport {
        files: Vec<ChangedFile>,
        truncated_count: usize,
    },
}

#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub summary: String,
    pub diff: String,
}

#[derive(Debug, Clone)]
pub struct ToolStep {
    /// SSE status_id — used to deduplicate "doing" updates for the same tool call.
    pub status_id: String,
    pub tool_name: String,
    pub args_summary: String,
    pub status: StepStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepStatus {
    InProgress,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct SubagentEntry {
    pub subagent_id: String,
    pub task: String,
    pub status: SubagentStatus,
    pub tool_count: usize,
    pub estimated_tokens: Option<usize>,
    pub current_activity: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SubagentStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct PlanDisplayItem {
    pub title: String,
    pub status: String,
}
