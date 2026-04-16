pub mod missions;
pub mod path_encoding;
pub mod runs;

pub use missions::{Mission as CronMission, MissionRunEntry, MissionStore};
pub use runs::{AgentRunRecord, AgentRunStatus, RunStore};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use path_encoding::encode_project_path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectInfo {
    pub path: String,
    pub name: String,
    pub added_at: u64,
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

    pub fn project_dir(&self, project_path: &str) -> PathBuf {
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
    fn test_in_memory_run_store() {
        let runs = RunStore::new();
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
        runs.add_run(&record);
        let list = runs.list_runs(None);
        assert_eq!(list.len(), 1);
    }
}
