use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentRunRecord {
    pub run_id: String,
    pub repo_path: String,
    pub session_id: String,
    pub agent_id: String,
    #[serde(default)]
    pub agent_kind: Option<String>,
    pub parent_run_id: Option<String>,
    pub status: AgentRunStatus,
    pub detail: Option<String>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
}

pub struct RunStore {
    runs_dir: PathBuf,
}

impl RunStore {
    pub fn new(runs_dir: PathBuf) -> Self {
        Self { runs_dir }
    }

    pub fn add_run(&self, record: &AgentRunRecord) -> Result<()> {
        fs::create_dir_all(&self.runs_dir)?;
        let path = self.runs_dir.join(format!("{}.json", record.run_id));
        let json = serde_json::to_string_pretty(record)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn update_run(
        &self,
        run_id: &str,
        status: AgentRunStatus,
        detail: Option<String>,
        ended_at: Option<u64>,
    ) -> Result<()> {
        let path = self.runs_dir.join(format!("{}.json", run_id));
        if !path.exists() {
            return Ok(());
        }
        let json = fs::read_to_string(&path)?;
        let mut run: AgentRunRecord = serde_json::from_str(&json)?;
        run.status = status;
        if detail.is_some() {
            run.detail = detail;
        }
        if ended_at.is_some() {
            run.ended_at = ended_at;
        }
        let updated = serde_json::to_string_pretty(&run)?;
        fs::write(path, updated)?;
        Ok(())
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<AgentRunRecord>> {
        let path = self.runs_dir.join(format!("{}.json", run_id));
        if !path.exists() {
            return Ok(None);
        }
        let json = fs::read_to_string(&path)?;
        let run: AgentRunRecord = serde_json::from_str(&json)?;
        Ok(Some(run))
    }

    pub fn list_runs(&self, session_id: Option<&str>) -> Result<Vec<AgentRunRecord>> {
        if !self.runs_dir.exists() {
            return Ok(Vec::new());
        }
        let mut runs = Vec::new();
        for entry in fs::read_dir(&self.runs_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "json") {
                continue;
            }
            let json = match fs::read_to_string(&path) {
                Ok(j) => j,
                Err(_) => continue,
            };
            let run: AgentRunRecord = match serde_json::from_str(&json) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Skipping corrupt run file {}: {}", path.display(), e);
                    continue;
                }
            };
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

    pub fn list_children(&self, parent_run_id: &str) -> Result<Vec<AgentRunRecord>> {
        if !self.runs_dir.exists() {
            return Ok(Vec::new());
        }
        let mut runs = Vec::new();
        for entry in fs::read_dir(&self.runs_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "json") {
                continue;
            }
            let json = match fs::read_to_string(&path) {
                Ok(j) => j,
                Err(_) => continue,
            };
            let run: AgentRunRecord = match serde_json::from_str(&json) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Skipping corrupt run file {}: {}", path.display(), e);
                    continue;
                }
            };
            if run.parent_run_id.as_deref() == Some(parent_run_id) {
                runs.push(run);
            }
        }
        runs.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        Ok(runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_run_store() -> (RunStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let runs_dir = dir.path().join("runs");
        let store = RunStore::new(runs_dir);
        (store, dir)
    }

    fn make_run(run_id: &str, session_id: &str, parent: Option<&str>, started_at: u64) -> AgentRunRecord {
        AgentRunRecord {
            run_id: run_id.into(),
            repo_path: "/tmp/p".into(),
            session_id: session_id.into(),
            agent_id: "ling".into(),
            agent_kind: None,
            parent_run_id: parent.map(|s| s.into()),
            status: AgentRunStatus::Running,
            detail: None,
            started_at,
            ended_at: None,
        }
    }

    #[test]
    fn test_add_and_get_run() {
        let (store, _dir) = temp_run_store();
        let run = make_run("r1", "s1", None, 1000);
        store.add_run(&run).unwrap();

        let fetched = store.get_run("r1").unwrap().unwrap();
        assert_eq!(fetched.status, AgentRunStatus::Running);
        assert_eq!(fetched.run_id, "r1");
    }

    #[test]
    fn test_update_run() {
        let (store, _dir) = temp_run_store();
        let run = make_run("r1", "s1", None, 1000);
        store.add_run(&run).unwrap();

        store.update_run("r1", AgentRunStatus::Completed, Some("done".into()), Some(2000)).unwrap();
        let fetched = store.get_run("r1").unwrap().unwrap();
        assert_eq!(fetched.status, AgentRunStatus::Completed);
        assert_eq!(fetched.detail.as_deref(), Some("done"));
        assert_eq!(fetched.ended_at, Some(2000));
    }

    #[test]
    fn test_list_runs() {
        let (store, _dir) = temp_run_store();
        store.add_run(&make_run("r1", "s1", None, 1000)).unwrap();
        store.add_run(&make_run("r2", "s1", None, 2000)).unwrap();
        store.add_run(&make_run("r3", "s2", None, 3000)).unwrap();

        let all = store.list_runs(None).unwrap();
        assert_eq!(all.len(), 3);
        // Sorted desc by started_at
        assert_eq!(all[0].run_id, "r3");

        let s1_runs = store.list_runs(Some("s1")).unwrap();
        assert_eq!(s1_runs.len(), 2);

        let s2_runs = store.list_runs(Some("s2")).unwrap();
        assert_eq!(s2_runs.len(), 1);

        let empty = store.list_runs(Some("nonexistent")).unwrap();
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn test_list_children() {
        let (store, _dir) = temp_run_store();
        store.add_run(&make_run("parent", "s1", None, 1000)).unwrap();
        store.add_run(&make_run("child1", "s1", Some("parent"), 1001)).unwrap();
        store.add_run(&make_run("child2", "s1", Some("parent"), 1002)).unwrap();
        store.add_run(&make_run("other", "s1", None, 1003)).unwrap();

        let children = store.list_children("parent").unwrap();
        assert_eq!(children.len(), 2);
        // Sorted asc by started_at
        assert_eq!(children[0].run_id, "child1");
        assert_eq!(children[1].run_id, "child2");
    }

    #[test]
    fn test_get_nonexistent_run() {
        let (store, _dir) = temp_run_store();
        assert!(store.get_run("nonexistent").unwrap().is_none());
    }
}
