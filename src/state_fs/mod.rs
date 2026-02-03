use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
        let root = ws_root.join(".linggen-agent/lead");
        Self { root }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(self.root.join("tasks"))?;
        fs::create_dir_all(self.root.join("sessions"))?;
        Ok(())
    }

    pub fn read_file(&self, rel_path: &str) -> Result<(StateFile, String)> {
        let path = self.root.join(rel_path);
        let content = fs::read_to_string(&path)?;
        self.parse_markdown(&content)
    }

    pub fn write_file(&self, rel_path: &str, state: &StateFile, body: &str) -> Result<()> {
        let path = self.root.join(rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let yaml = serde_yaml::to_string(state)?;
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
        let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let id = format!("msg-{}", ts);

        let msg = StateFile::Message {
            id,
            from: from.to_string(),
            to: to.to_string(),
            ts,
            task_id,
        };

        let yaml = serde_yaml::to_string(&msg)?;
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
        let state: StateFile = serde_yaml::from_str(parts[1])?;
        let body = parts[2].trim().to_string();
        Ok((state, body))
    }

    pub fn read_messages(&self, session_id: Option<&str>) -> Result<Vec<(StateFile, String)>> {
        let path = if let Some(sid) = session_id {
            self.root.join("sessions").join(format!("{}.md", sid))
        } else {
            self.root.join("messages.md")
        };
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(path)?;
        let mut messages = Vec::new();

        // Split by "---" but handle the fact that each message has two "---"
        // A better way is to split and then group
        let parts: Vec<&str> = content.split("---").collect();
        // parts[0] is likely empty or whitespace before first ---
        // parts[1] is yaml, parts[2] is body, parts[3] is yaml, parts[4] is body...
        let mut i = 1;
        while i + 1 < parts.len() {
            let yaml = parts[i];
            let body = parts[i + 1].trim();
            if let Ok(state) = serde_yaml::from_str::<StateFile>(yaml) {
                messages.push((state, body.to_string()));
            }
            i += 2;
        }

        Ok(messages)
    }
}
