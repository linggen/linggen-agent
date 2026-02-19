pub mod marketplace;

use crate::engine::skill_tool::SkillToolDef;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::sync::Mutex;

/// Built-in skills available for one-click install from the linggen/skills repo.
#[derive(Debug, Clone, Serialize)]
pub struct BuiltInSkillInfo {
    pub name: String,
    pub description: String,
    pub installed: bool,
}

/// Returns the list of built-in skills with their install status.
pub fn list_builtin_skills() -> Vec<BuiltInSkillInfo> {
    let defs: &[(&str, &str)] = &[
        ("memory", "Semantic memory and RAG — index codebases, search semantically, store and retrieve memories."),
        ("linggen", "Cross-project code search, indexed context, prompt enhancement, and server management."),
    ];

    let global_skills_dir = dirs::home_dir().map(|h| h.join(".linggen/skills"));

    defs.iter()
        .map(|(name, desc)| {
            let installed = global_skills_dir
                .as_ref()
                .map(|d| d.join(name).join("SKILL.md").exists())
                .unwrap_or(false);
            BuiltInSkillInfo {
                name: name.to_string(),
                description: desc.to_string(),
                installed,
            }
        })
        .collect()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source: SkillSource,
    #[serde(default)]
    pub tool_defs: Vec<SkillToolDef>,
    #[serde(default)]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub disable_model_invocation: bool,
    #[serde(default = "default_user_invocable")]
    pub user_invocable: bool,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub trigger: Option<String>,
}

fn default_user_invocable() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum SkillSource {
    Global,
    Project,
    Compat,
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
    #[serde(default)]
    tools: Vec<SkillToolDef>,
    #[serde(default, rename = "argument-hint")]
    argument_hint: Option<String>,
    #[serde(default, rename = "disable-model-invocation")]
    disable_model_invocation: bool,
    #[serde(default = "default_user_invocable", rename = "user-invocable")]
    user_invocable: bool,
    #[serde(default, rename = "allowed-tools")]
    allowed_tools: Vec<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
}

pub struct SkillManager {
    skills: Mutex<HashMap<String, Skill>>,
    triggers: Mutex<HashMap<String, String>>,
}

impl SkillManager {
    pub fn new() -> Self {
        Self {
            skills: Mutex::new(HashMap::new()),
            triggers: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load_all(&self, project_root: Option<&Path>) -> Result<()> {
        let mut skills = self.skills.lock().await;
        skills.clear();

        // 1. Load Compat Skills (~/.claude/skills/, ~/.codex/skills/)
        if let Some(home) = dirs::home_dir() {
            for compat_dir_name in &[".claude/skills", ".codex/skills"] {
                let compat_dir = home.join(compat_dir_name);
                let _ = self
                    .load_from_dir_nested(&compat_dir, SkillSource::Compat, &mut *skills)
                    .await;
            }
        }

        // 2. Load Global Skills (~/.linggen/skills/)
        if let Some(home) = dirs::home_dir() {
            let global_dir = home.join(".linggen/skills");
            let _ = self
                .load_from_dir_nested(&global_dir, SkillSource::Global, &mut *skills)
                .await;
        }

        // 3. Load Project Skills (.linggen/skills/) — highest priority
        if let Some(root) = project_root {
            let project_dir = root.join(".linggen/skills");
            let _ = self
                .load_from_dir_nested(&project_dir, SkillSource::Project, &mut *skills)
                .await;
        }

        // Rebuild trigger index from skills with trigger field set.
        let mut triggers = self.triggers.lock().await;
        triggers.clear();
        for (name, skill) in skills.iter() {
            if let Some(trigger) = &skill.trigger {
                let trigger = trigger.trim().to_string();
                if !trigger.is_empty() {
                    triggers.insert(trigger, name.clone());
                }
            }
        }

        Ok(())
    }

    /// Load skills from a directory, supporting both flat .md files and
    /// subdirectories containing SKILL.md (e.g. `skills/<name>/SKILL.md`).
    async fn load_from_dir_nested(
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
            if path.is_file() && path.extension().map_or(false, |ext| ext == "md") {
                // Flat .md file
                let text = std::fs::read_to_string(&path)?;
                if let Ok(mut skill) = self.parse_skill(&text, source.clone()) {
                    if let Some(parent) = path.parent() {
                        for tool_def in &mut skill.tool_defs {
                            tool_def.skill_dir = Some(parent.to_path_buf());
                        }
                    }
                    skills.insert(skill.name.clone(), skill);
                }
            } else if path.is_dir() {
                // Subdirectory: look for SKILL.md inside
                let skill_md = path.join("SKILL.md");
                if skill_md.exists() {
                    let text = std::fs::read_to_string(&skill_md)?;
                    if let Ok(mut skill) = self.parse_skill(&text, source.clone()) {
                        for tool_def in &mut skill.tool_defs {
                            tool_def.skill_dir = Some(path.clone());
                        }
                        skills.insert(skill.name.clone(), skill);
                    }
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
            tool_defs: frontmatter.tools,
            argument_hint: frontmatter.argument_hint,
            disable_model_invocation: frontmatter.disable_model_invocation,
            user_invocable: frontmatter.user_invocable,
            allowed_tools: frontmatter.allowed_tools,
            model: frontmatter.model,
            context: frontmatter.context,
            agent: frontmatter.agent,
            trigger: frontmatter.trigger,
        })
    }

    /// Match a user message against registered triggers.
    /// Returns (skill_name, remaining_input) if a trigger prefix matches.
    /// Triggers are sorted longest-first so longer prefixes win.
    pub async fn match_trigger(&self, input: &str) -> Option<(String, String)> {
        let triggers = self.triggers.lock().await;
        if triggers.is_empty() {
            return None;
        }
        // Sort triggers longest-first for greedy matching.
        let mut sorted: Vec<(&String, &String)> = triggers.iter().collect();
        sorted.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        for (prefix, skill_name) in sorted {
            if input.starts_with(prefix.as_str()) {
                let remaining = input[prefix.len()..].trim_start().to_string();
                return Some((skill_name.clone(), remaining));
            }
        }
        None
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
