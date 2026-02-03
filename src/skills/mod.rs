use anyhow::Result;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::sync::Mutex;

#[derive(RustEmbed)]
#[folder = "src/skills/embedded/"]
pub struct EmbeddedSkills;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source: SkillSource,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum SkillSource {
    Embedded,
    Global,
    Project,
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
}

pub struct SkillManager {
    skills: Mutex<HashMap<String, Skill>>,
}

impl SkillManager {
    pub fn new() -> Self {
        Self {
            skills: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load_all(&self, project_root: Option<&Path>) -> Result<()> {
        let mut skills = self.skills.lock().await;
        skills.clear();

        // 1. Load Embedded Skills
        for file in EmbeddedSkills::iter() {
            if let Some(content) = EmbeddedSkills::get(&file) {
                let text = String::from_utf8_lossy(&content.data).to_string();
                if let Ok(skill) = self.parse_skill(&text, SkillSource::Embedded) {
                    skills.insert(skill.name.clone(), skill);
                }
            }
        }

        // 2. Load Global Skills (~/.linggen/skills/)
        if let Some(home) = dirs::home_dir() {
            let global_dir = home.join(".linggen/skills");
            let _ = self
                .load_from_dir(&global_dir, SkillSource::Global, &mut *skills)
                .await;
        }

        // 3. Load Project Skills (.linggen/skills/)
        if let Some(root) = project_root {
            let project_dir = root.join(".linggen/skills");
            let _ = self
                .load_from_dir(&project_dir, SkillSource::Project, &mut *skills)
                .await;
        }

        Ok(())
    }

    async fn load_from_dir(
        &self,
        dir: &Path,
        source: SkillSource,
        skills: &mut HashMap<String, Skill>,
    ) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "md") {
                let text = std::fs::read_to_string(&path)?;
                if let Ok(skill) = self.parse_skill(&text, source.clone()) {
                    skills.insert(skill.name.clone(), skill);
                }
            }
        }
        Ok(())
    }

    fn parse_skill(&self, text: &str, source: SkillSource) -> Result<Skill> {
        if !text.starts_with("---") {
            anyhow::bail!("Skill must start with YAML frontmatter");
        }
        let parts: Vec<&str> = text.splitn(3, "---").collect();
        if parts.len() < 3 {
            anyhow::bail!("Skill missing closing frontmatter delimiter");
        }
        let frontmatter: SkillFrontmatter = serde_yaml::from_str(parts[1])?;
        let content = parts[2].trim().to_string();

        Ok(Skill {
            name: frontmatter.name,
            description: frontmatter.description,
            content,
            source,
        })
    }

    pub async fn get_skill(&self, name: &str) -> Option<Skill> {
        let skills = self.skills.lock().await;
        skills.get(name).cloned()
    }

    pub async fn list_skills(&self) -> Vec<Skill> {
        let skills = self.skills.lock().await;
        skills.values().cloned().collect()
    }
}
