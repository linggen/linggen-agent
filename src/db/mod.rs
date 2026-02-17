use anyhow::Result;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// Table definitions
const PROJECTS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("projects");
const FILE_ACTIVITY_TABLE: TableDefinition<&str, &str> = TableDefinition::new("file_activity");
const SESSIONS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("sessions");
const CHAT_HISTORY_TABLE: TableDefinition<&str, &str> = TableDefinition::new("chat_history");
const PROJECT_SETTINGS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("project_settings");
const AGENT_RUNS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("agent_runs");

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectInfo {
    pub path: String,
    pub name: String,
    pub added_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub repo_path: String,
    pub title: String,
    pub created_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessageRecord {
    pub repo_path: String,
    pub session_id: String,
    pub agent_id: String,
    pub from_id: String,
    pub to_id: String,
    pub content: String,
    pub timestamp: u64,
    pub is_observation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileActivityStatus {
    Working,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectMode {
    Chat,
    Auto,
}

impl std::fmt::Display for ProjectMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chat => write!(f, "chat"),
            Self::Auto => write!(f, "auto"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileActivity {
    pub repo_path: String,
    pub file_path: String,
    pub agent_id: String,
    pub status: FileActivityStatus,
    pub last_modified: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectSettings {
    pub repo_path: String,
    pub mode: ProjectMode,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentRunRecord {
    pub run_id: String,
    pub repo_path: String,
    pub session_id: String,
    pub agent_id: String,
    pub agent_kind: crate::config::AgentKind,
    pub parent_run_id: Option<String>,
    pub status: AgentRunStatus,
    pub detail: Option<String>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
}

pub struct Db {
    db: Arc<Database>,
}

impl Db {
    pub fn new() -> Result<Self> {
        let config_dir = dirs::data_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
            .ok_or_else(|| anyhow::anyhow!("Could not find data directory"))?
            .join("linggen-agent");

        std::fs::create_dir_all(&config_dir)?;
        let db_path = config_dir.join("agent_state.redb");

        let db = Database::create(db_path)?;

        // Initialize tables
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(PROJECTS_TABLE)?;
            let _ = write_txn.open_table(FILE_ACTIVITY_TABLE)?;
            let _ = write_txn.open_table(SESSIONS_TABLE)?;
            let _ = write_txn.open_table(CHAT_HISTORY_TABLE)?;
            let _ = write_txn.open_table(PROJECT_SETTINGS_TABLE)?;
            let _ = write_txn.open_table(AGENT_RUNS_TABLE)?;
        }
        write_txn.commit()?;

        Ok(Self { db: Arc::new(db) })
    }

    pub fn add_project(&self, path: String, name: String) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(PROJECTS_TABLE)?;
            let now = crate::util::now_ts_secs();
            let info = ProjectInfo {
                path: path.clone(),
                name,
                added_at: now,
            };
            let val = serde_json::to_string(&info)?;
            table.insert(path.as_str(), val.as_str())?;

            let mut settings_table = write_txn.open_table(PROJECT_SETTINGS_TABLE)?;
            if settings_table.get(path.as_str())?.is_none() {
                let settings = ProjectSettings {
                    repo_path: path.clone(),
                    mode: ProjectMode::Auto,
                };
                let settings_val = serde_json::to_string(&settings)?;
                settings_table.insert(path.as_str(), settings_val.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(PROJECTS_TABLE)?;
        let mut projects = Vec::new();
        for res in table.iter()? {
            let (_key, val) = res?;
            let info: ProjectInfo = serde_json::from_str(val.value())?;
            projects.push(info);
        }
        Ok(projects)
    }

    pub fn remove_project(&self, path: &str) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(PROJECTS_TABLE)?;
            table.remove(path)?;
            let mut settings_table = write_txn.open_table(PROJECT_SETTINGS_TABLE)?;
            settings_table.remove(path)?;

            // Also remove all activity for this project
            let mut act_table = write_txn.open_table(FILE_ACTIVITY_TABLE)?;
            let mut to_remove = Vec::new();
            for res in act_table.iter()? {
                let (key, val) = res?;
                let activity: FileActivity = serde_json::from_str(val.value())?;
                if activity.repo_path == path {
                    to_remove.push(key.value().to_string());
                }
            }
            for key in to_remove {
                act_table.remove(key.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_project_settings(&self, repo_path: &str) -> Result<ProjectSettings> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(PROJECT_SETTINGS_TABLE)?;
        if let Some(val) = table.get(repo_path)? {
            let settings: ProjectSettings = serde_json::from_str(val.value())?;
            Ok(settings)
        } else {
            Ok(ProjectSettings {
                repo_path: repo_path.to_string(),
                mode: ProjectMode::Auto,
            })
        }
    }

    pub fn set_project_mode(&self, repo_path: &str, mode: ProjectMode) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(PROJECT_SETTINGS_TABLE)?;
            let settings = ProjectSettings {
                repo_path: repo_path.to_string(),
                mode,
            };
            let val = serde_json::to_string(&settings)?;
            table.insert(repo_path, val.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn remove_activity(&self, repo_path: &str, file_path: &str) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(FILE_ACTIVITY_TABLE)?;
            let key = format!("{}:{}", repo_path, file_path);
            table.remove(key.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn rename_activity(&self, repo_path: &str, old_path: &str, new_path: &str) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(FILE_ACTIVITY_TABLE)?;
            let old_key = format!("{}:{}", repo_path, old_path);

            let activity_json = if let Some(val) = table.remove(old_key.as_str())? {
                let mut activity: FileActivity = serde_json::from_str(val.value())?;
                activity.file_path = new_path.to_string();
                Some(serde_json::to_string(&activity)?)
            } else {
                None
            };

            if let Some(new_val) = activity_json {
                let new_key = format!("{}:{}", repo_path, new_path);
                table.insert(new_key.as_str(), new_val.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn record_activity(&self, activity: FileActivity) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(FILE_ACTIVITY_TABLE)?;
            let key = format!("{}:{}", activity.repo_path, activity.file_path);
            let val = serde_json::to_string(&activity)?;
            table.insert(key.as_str(), val.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_repo_activity(&self, repo_path: &str) -> Result<Vec<FileActivity>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(FILE_ACTIVITY_TABLE)?;
        let mut activities = Vec::new();
        for res in table.iter()? {
            let (_key, val) = res?;
            let activity: FileActivity = serde_json::from_str(val.value())?;
            if activity.repo_path == repo_path {
                activities.push(activity);
            }
        }
        Ok(activities)
    }

    pub fn add_session(&self, session: SessionInfo) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SESSIONS_TABLE)?;
            let key = format!("{}:{}", session.repo_path, session.id);
            let val = serde_json::to_string(&session)?;
            table.insert(key.as_str(), val.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn list_sessions(&self, repo_path: &str) -> Result<Vec<SessionInfo>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(SESSIONS_TABLE)?;
        let mut sessions = Vec::new();
        for res in table.iter()? {
            let (_key, val) = res?;
            let session: SessionInfo = serde_json::from_str(val.value())?;
            if session.repo_path == repo_path {
                sessions.push(session);
            }
        }
        // Sort by created_at descending
        sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(sessions)
    }

    pub fn remove_session(&self, repo_path: &str, session_id: &str) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SESSIONS_TABLE)?;
            let key = format!("{}:{}", repo_path, session_id);
            table.remove(key.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn clear_chat_history(&self, repo_path: &str, session_id: &str) -> Result<usize> {
        let write_txn = self.db.begin_write()?;
        let mut removed = 0usize;
        {
            let mut table = write_txn.open_table(CHAT_HISTORY_TABLE)?;
            let prefix = format!("{}:{}:", repo_path, session_id);
            let mut to_remove = Vec::new();
            for res in table.iter()? {
                let (key, _val) = res?;
                let k = key.value();
                if k.starts_with(&prefix) {
                    to_remove.push(k.to_string());
                }
            }
            for k in to_remove {
                table.remove(k.as_str())?;
                removed += 1;
            }
        }
        write_txn.commit()?;
        Ok(removed)
    }

    pub fn add_chat_message(&self, msg: ChatMessageRecord) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(CHAT_HISTORY_TABLE)?;
            // Key: repo:session:agent:timestamp_nanos
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
            let nanos = now.as_nanos();
            let key = format!(
                "{}:{}:{}:{:020}",
                msg.repo_path, msg.session_id, msg.agent_id, nanos
            );
            let val = serde_json::to_string(&msg)?;
            table.insert(key.as_str(), val.as_str())?;
        }
        write_txn.commit()?;

        // Log to console for debugging
        tracing::debug!("Chat Message Recorded: {} -> {}", msg.from_id, msg.to_id);

        Ok(())
    }

    pub fn add_agent_run(&self, run: AgentRunRecord) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(AGENT_RUNS_TABLE)?;
            let val = serde_json::to_string(&run)?;
            table.insert(run.run_id.as_str(), val.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn update_agent_run(
        &self,
        run_id: &str,
        status: AgentRunStatus,
        detail: Option<String>,
        ended_at: Option<u64>,
    ) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(AGENT_RUNS_TABLE)?;
            let existing = table.get(run_id)?.map(|val| val.value().to_string());
            if let Some(json) = existing {
                let mut run: AgentRunRecord = serde_json::from_str(&json)?;
                run.status = status;
                if detail.is_some() {
                    run.detail = detail;
                }
                if ended_at.is_some() {
                    run.ended_at = ended_at;
                }
                let next = serde_json::to_string(&run)?;
                table.insert(run_id, next.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn list_agent_runs(
        &self,
        repo_path: &str,
        session_id: Option<&str>,
    ) -> Result<Vec<AgentRunRecord>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(AGENT_RUNS_TABLE)?;
        let mut runs = Vec::new();
        for res in table.iter()? {
            let (_key, val) = res?;
            let run: AgentRunRecord = serde_json::from_str(val.value())?;
            if run.repo_path != repo_path {
                continue;
            }
            if let Some(sid) = session_id {
                if run.session_id != sid {
                    continue;
                }
            }
            runs.push(run);
        }
        runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(runs)
    }

    pub fn list_agent_children(&self, parent_run_id: &str) -> Result<Vec<AgentRunRecord>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(AGENT_RUNS_TABLE)?;
        let mut runs = Vec::new();
        for res in table.iter()? {
            let (_key, val) = res?;
            let run: AgentRunRecord = serde_json::from_str(val.value())?;
            if run.parent_run_id.as_deref() == Some(parent_run_id) {
                runs.push(run);
            }
        }
        runs.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        Ok(runs)
    }

    pub fn get_agent_run(&self, run_id: &str) -> Result<Option<AgentRunRecord>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(AGENT_RUNS_TABLE)?;
        if let Some(val) = table.get(run_id)? {
            let run: AgentRunRecord = serde_json::from_str(val.value())?;
            Ok(Some(run))
        } else {
            Ok(None)
        }
    }

    pub fn get_chat_history(
        &self,
        repo_path: &str,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Result<Vec<ChatMessageRecord>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(CHAT_HISTORY_TABLE)?;
        let mut history = Vec::new();

        // Simple prefix scan for now
        let prefix = if let Some(aid) = agent_id {
            format!("{}:{}:{}:", repo_path, session_id, aid)
        } else {
            format!("{}:{}:", repo_path, session_id)
        };

        for res in table.iter()? {
            let (key, val) = res?;
            if key.value().starts_with(&prefix) {
                let msg: ChatMessageRecord = serde_json::from_str(val.value())?;
                history.push(msg);
            }
        }
        Ok(history)
    }
}
