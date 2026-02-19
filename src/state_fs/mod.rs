use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StateFile {
    #[serde(rename = "pm_task")]
    PmTask {
        id: String,
        status: String,
        #[serde(default)]
        assigned_tasks: Vec<String>,
    },
    #[serde(rename = "user_stories")]
    UserStories { id: String },
    #[serde(rename = "coder_task")]
    CoderTask {
        id: String,
        status: String,
        story_id: Option<String>,
        assigned_to: String,
    },
    #[serde(rename = "message")]
    Message {
        id: String,
        from: String,
        to: String,
        ts: u64,
        #[serde(default)]
        task_id: Option<String>,
    },
}

pub struct StateFs {
    root: PathBuf,
}

impl StateFs {
    pub fn new(ws_root: PathBuf) -> Self {
        let root = ws_root.join(".linggen-agent/workspace");
        Self { root }
    }

    /// Resolve a relative path and verify it stays within the state root.
    fn safe_resolve(&self, rel_path: &str) -> Result<PathBuf> {
        let path = self.root.join(rel_path);
        let canonical = path
            .canonicalize()
            .or_else(|_| {
                // File might not exist yet (write case). Canonicalize the parent instead.
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                    let canon_parent = parent.canonicalize()?;
                    Ok(canon_parent.join(path.file_name().unwrap_or_default()))
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "cannot resolve path",
                    ))
                }
            })?;
        let canon_root = self.root.canonicalize().unwrap_or_else(|_| self.root.clone());
        if !canonical.starts_with(&canon_root) {
            anyhow::bail!(
                "Path traversal rejected: {} escapes state root {}",
                rel_path,
                canon_root.display()
            );
        }
        Ok(canonical)
    }

    pub fn read_file(&self, rel_path: &str) -> Result<(StateFile, String)> {
        let path = self.safe_resolve(rel_path)?;
        let content = fs::read_to_string(&path)?;
        self.parse_markdown(&content)
    }

    pub fn write_file(&self, rel_path: &str, state: &StateFile, body: &str) -> Result<()> {
        let path = self.safe_resolve(rel_path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let yaml = serde_yml::to_string(state)?;
        let content = format!("---\n{}---\n\n{}", yaml, body);
        fs::write(path, content)?;
        Ok(())
    }

    pub fn append_message(
        &self,
        from: &str,
        to: &str,
        content: &str,
        task_id: Option<String>,
        session_id: Option<&str>,
    ) -> Result<()> {
        let path = if let Some(sid) = session_id {
            self.root.join("sessions").join(format!("{}.md", sid))
        } else {
            self.root.join("messages.md")
        };
        let ts = crate::util::now_ts_secs();
        let id = format!("msg-{}", ts);

        let msg = StateFile::Message {
            id,
            from: from.to_string(),
            to: to.to_string(),
            ts,
            task_id,
        };

        let yaml = serde_yml::to_string(&msg)?;
        let entry = format!("\n---\n{}---\n\n{}\n", yaml, content);

        use std::fs::OpenOptions;
        use std::io::Write;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(entry.as_bytes())?;
        Ok(())
    }

    pub fn list_tasks(&self) -> Result<Vec<(StateFile, String)>> {
        let tasks_dir = self.root.join("tasks");
        if !tasks_dir.exists() {
            return Ok(Vec::new());
        }

        let mut tasks = Vec::new();
        for entry in fs::read_dir(tasks_dir)? {
            let entry = entry?;
            if entry.path().extension().map_or(false, |ext| ext == "md") {
                let content = fs::read_to_string(entry.path())?;
                if let Ok(parsed) = self.parse_markdown(&content) {
                    tasks.push(parsed);
                }
            }
        }
        Ok(tasks)
    }

    fn parse_markdown(&self, content: &str) -> Result<(StateFile, String)> {
        if !content.starts_with("---") {
            anyhow::bail!("Markdown must start with YAML frontmatter (---)");
        }
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            anyhow::bail!("Markdown missing closing frontmatter delimiter (---)");
        }
        let state: StateFile = serde_yml::from_str(parts[1])?;
        let body = parts[2].trim().to_string();
        Ok((state, body))
    }
}
