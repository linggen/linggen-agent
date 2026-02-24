pub mod path_encoding;
pub mod runs;

pub use runs::{AgentRunRecord, AgentRunStatus, RunStore};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::state_fs::SessionStore;
use path_encoding::encode_project_path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectInfo {
    pub path: String,
    pub name: String,
    pub added_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Mission {
    pub text: String,
    pub created_at: u64,
    pub active: bool,
    /// Per-agent idle configurations within this mission.
    #[serde(default)]
    pub agents: Vec<MissionAgent>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MissionAgent {
    pub id: String,
    #[serde(default)]
    pub idle_prompt: Option<String>,
    #[serde(default)]
    pub idle_interval_secs: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentOverride {
    pub agent_id: String,
    #[serde(default)]
    pub idle_prompt: Option<String>,
    #[serde(default)]
    pub idle_interval_secs: Option<u64>,
}

pub struct ProjectStore {
    root: PathBuf,
}

impl ProjectStore {
    pub fn new() -> Self {
        Self {
            root: crate::paths::projects_dir(),
        }
    }

    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    fn project_dir(&self, project_path: &str) -> PathBuf {
        self.root.join(encode_project_path(project_path))
    }

    pub fn add_project(&self, path: String, name: String) -> Result<()> {
        let dir = self.project_dir(&path);
        fs::create_dir_all(&dir)?;
        let info = ProjectInfo {
            path,
            name,
            added_at: crate::util::now_ts_secs(),
        };
        let json = serde_json::to_string_pretty(&info)?;
        fs::write(dir.join("project.json"), json)?;
        Ok(())
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut projects = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let json_path = entry.path().join("project.json");
            if !json_path.exists() {
                continue;
            }
            let content = match fs::read_to_string(&json_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            match serde_json::from_str::<ProjectInfo>(&content) {
                Ok(info) => projects.push(info),
                Err(e) => {
                    tracing::warn!(
                        "Skipping corrupt project.json at {}: {}",
                        json_path.display(),
                        e
                    );
                }
            }
        }
        Ok(projects)
    }

    pub fn remove_project(&self, path: &str) -> Result<()> {
        let dir = self.project_dir(path);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn session_store(&self, project_path: &str) -> SessionStore {
        let sessions_dir = self.project_dir(project_path).join("sessions");
        SessionStore::with_sessions_dir(sessions_dir)
    }

    pub fn run_store(&self, project_path: &str) -> RunStore {
        let runs_dir = self.project_dir(project_path).join("runs");
        RunStore::new(runs_dir)
    }

    pub fn memory_dir(&self, project_path: &str) -> PathBuf {
        self.project_dir(project_path).join("memory")
    }

    // ---- Mission storage ----
    // Missions are stored as individual JSON files in a `missions/` directory,
    // named by created_at timestamp: `missions/{timestamp}.json`.
    // This preserves full history — clearing marks the file inactive in place.

    fn missions_dir(&self, project_path: &str) -> PathBuf {
        self.project_dir(project_path).join("missions")
    }

    /// Migrate legacy single `mission.json` into the `missions/` directory.
    fn migrate_legacy_mission(&self, project_path: &str) -> Result<()> {
        let legacy = self.project_dir(project_path).join("mission.json");
        if !legacy.exists() {
            return Ok(());
        }
        let content = fs::read_to_string(&legacy)?;
        if let Ok(mission) = serde_json::from_str::<Mission>(&content) {
            let dir = self.missions_dir(project_path);
            fs::create_dir_all(&dir)?;
            let filename = format!("{}.json", mission.created_at);
            let dest = dir.join(&filename);
            if !dest.exists() {
                fs::write(&dest, serde_json::to_string_pretty(&mission)?)?;
            }
            fs::remove_file(&legacy)?;
        } else {
            tracing::warn!(
                "Legacy mission.json at {} could not be parsed; removing",
                legacy.display()
            );
            fs::remove_file(&legacy)?;
        }
        Ok(())
    }

    /// Return the currently active mission (if any).
    pub fn get_mission(&self, project_path: &str) -> Result<Option<Mission>> {
        let _ = self.migrate_legacy_mission(project_path);
        let dir = self.missions_dir(project_path);
        if !dir.exists() {
            return Ok(None);
        }
        // Scan files sorted descending by name (newest first) to find active mission.
        let mut files: Vec<_> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();
        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        for entry in files {
            if let Ok(content) = fs::read_to_string(entry.path()) {
                if let Ok(mission) = serde_json::from_str::<Mission>(&content) {
                    if mission.active {
                        return Ok(Some(mission));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Save a new mission. Deactivates any currently active mission first.
    pub fn set_mission(&self, project_path: &str, mission: &Mission) -> Result<()> {
        let _ = self.migrate_legacy_mission(project_path);
        // Deactivate current active mission
        if let Ok(Some(mut current)) = self.get_mission(project_path) {
            current.active = false;
            let dir = self.missions_dir(project_path);
            let path = dir.join(format!("{}.json", current.created_at));
            if path.exists() {
                fs::write(&path, serde_json::to_string_pretty(&current)?)?;
            }
        }
        let dir = self.missions_dir(project_path);
        fs::create_dir_all(&dir)?;
        let filename = format!("{}.json", mission.created_at);
        let json = serde_json::to_string_pretty(mission)?;
        fs::write(dir.join(filename), json)?;
        Ok(())
    }

    /// Clear (deactivate) the currently active mission.
    pub fn clear_mission(&self, project_path: &str) -> Result<()> {
        let _ = self.migrate_legacy_mission(project_path);
        let dir = self.missions_dir(project_path);
        if !dir.exists() {
            return Ok(());
        }
        let mut files: Vec<_> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();
        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        for entry in files {
            let path = entry.path();
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(mut mission) = serde_json::from_str::<Mission>(&content) {
                    if mission.active {
                        mission.active = false;
                        fs::write(&path, serde_json::to_string_pretty(&mission)?)?;
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }

    /// List all missions (active and inactive), newest first.
    pub fn list_missions(&self, project_path: &str) -> Result<Vec<Mission>> {
        let _ = self.migrate_legacy_mission(project_path);
        let dir = self.missions_dir(project_path);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut files: Vec<_> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();
        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        let mut missions = Vec::new();
        for entry in files {
            if let Ok(content) = fs::read_to_string(entry.path()) {
                if let Ok(mission) = serde_json::from_str::<Mission>(&content) {
                    missions.push(mission);
                }
            }
        }
        Ok(missions)
    }

    // ---- Agent override storage ----

    fn agent_overrides_dir(&self, project_path: &str) -> PathBuf {
        self.project_dir(project_path).join("agent_overrides")
    }

    pub fn get_agent_override(
        &self,
        project_path: &str,
        agent_id: &str,
    ) -> Result<Option<AgentOverride>> {
        anyhow::ensure!(
            !agent_id.contains('/') && !agent_id.contains('\\') && !agent_id.contains(".."),
            "invalid agent_id: must not contain path separators"
        );
        let path = self
            .agent_overrides_dir(project_path)
            .join(format!("{}.json", agent_id));
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        let overr: AgentOverride = serde_json::from_str(&content)?;
        Ok(Some(overr))
    }

    pub fn set_agent_override(
        &self,
        project_path: &str,
        overr: &AgentOverride,
    ) -> Result<()> {
        anyhow::ensure!(
            !overr.agent_id.contains('/')
                && !overr.agent_id.contains('\\')
                && !overr.agent_id.contains(".."),
            "invalid agent_id: must not contain path separators"
        );
        let dir = self.agent_overrides_dir(project_path);
        fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(overr)?;
        fs::write(dir.join(format!("{}.json", overr.agent_id)), json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (ProjectStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::with_root(dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn test_add_and_list_projects() {
        let (store, _dir) = temp_store();
        store.add_project("/tmp/project1".into(), "project1".into()).unwrap();
        store.add_project("/tmp/project2".into(), "project2".into()).unwrap();
        let projects = store.list_projects().unwrap();
        assert_eq!(projects.len(), 2);
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"project1"));
        assert!(names.contains(&"project2"));
    }

    #[test]
    fn test_remove_project() {
        let (store, _dir) = temp_store();
        store.add_project("/tmp/p".into(), "p".into()).unwrap();
        assert_eq!(store.list_projects().unwrap().len(), 1);
        store.remove_project("/tmp/p").unwrap();
        assert_eq!(store.list_projects().unwrap().len(), 0);
    }

    #[test]
    fn test_session_store_returns_valid_store() {
        let (store, _dir) = temp_store();
        store.add_project("/tmp/p".into(), "p".into()).unwrap();
        let sessions = store.session_store("/tmp/p");
        // Should be able to add a session
        sessions.add_session(&crate::state_fs::sessions::SessionMeta {
            id: "s1".into(),
            title: "test".into(),
            created_at: 1000,
        }).unwrap();
        let list = sessions.list_sessions().unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn test_run_store_returns_valid_store() {
        let (store, _dir) = temp_store();
        store.add_project("/tmp/p".into(), "p".into()).unwrap();
        let runs = store.run_store("/tmp/p");
        let record = AgentRunRecord {
            run_id: "r1".into(),
            repo_path: "/tmp/p".into(),
            session_id: "s1".into(),
            agent_id: "ling".into(),
            agent_kind: None,
            parent_run_id: None,
            status: AgentRunStatus::Running,
            detail: None,
            started_at: 1000,
            ended_at: None,
        };
        runs.add_run(&record).unwrap();
        let list = runs.list_runs(None).unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn test_mission_set_get_clear() {
        let (store, _dir) = temp_store();
        store.add_project("/tmp/p".into(), "p".into()).unwrap();

        // No mission initially
        assert!(store.get_mission("/tmp/p").unwrap().is_none());

        // Set mission
        let mission = Mission {
            text: "Monitor production".into(),
            created_at: 1000,
            active: true,
            agents: vec![
                MissionAgent {
                    id: "ling".into(),
                    idle_prompt: Some("Check status".into()),
                    idle_interval_secs: Some(60),
                },
            ],
        };
        store.set_mission("/tmp/p", &mission).unwrap();

        let loaded = store.get_mission("/tmp/p").unwrap().unwrap();
        assert_eq!(loaded.text, "Monitor production");
        assert!(loaded.active);
        assert_eq!(loaded.agents.len(), 1);

        // Clear mission
        store.clear_mission("/tmp/p").unwrap();
        assert!(store.get_mission("/tmp/p").unwrap().is_none());

        // History should still contain the cleared mission
        let history = store.list_missions("/tmp/p").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].text, "Monitor production");
        assert!(!history[0].active);
    }

    #[test]
    fn test_mission_history() {
        let (store, _dir) = temp_store();
        store.add_project("/tmp/p".into(), "p".into()).unwrap();

        // Set first mission
        let m1 = Mission {
            text: "First mission".into(),
            created_at: 1000,
            active: true,
            agents: vec![],
        };
        store.set_mission("/tmp/p", &m1).unwrap();

        // Set second mission (should deactivate first)
        let m2 = Mission {
            text: "Second mission".into(),
            created_at: 2000,
            active: true,
            agents: vec![],
        };
        store.set_mission("/tmp/p", &m2).unwrap();

        // Active should be the second one
        let active = store.get_mission("/tmp/p").unwrap().unwrap();
        assert_eq!(active.text, "Second mission");

        // History should have both, newest first
        let history = store.list_missions("/tmp/p").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].text, "Second mission");
        assert!(history[0].active);
        assert_eq!(history[1].text, "First mission");
        assert!(!history[1].active);
    }

    #[test]
    fn test_agent_override_set_get() {
        let (store, _dir) = temp_store();
        store.add_project("/tmp/p".into(), "p".into()).unwrap();

        // No override initially
        assert!(store.get_agent_override("/tmp/p", "ling").unwrap().is_none());

        // Set override
        let overr = AgentOverride {
            agent_id: "ling".into(),
            idle_prompt: Some("Custom idle prompt".into()),
            idle_interval_secs: Some(120),
        };
        store.set_agent_override("/tmp/p", &overr).unwrap();

        let loaded = store.get_agent_override("/tmp/p", "ling").unwrap().unwrap();
        assert_eq!(loaded.idle_prompt.as_deref(), Some("Custom idle prompt"));
        assert_eq!(loaded.idle_interval_secs, Some(120));
    }
}
