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
        ("skiller", "Search, install, and manage skills from the marketplace and skills.sh registry."),
        ("discord", "Social messaging with Discord friends — send messages, poll for new messages, manage contacts."),
        ("linggen", "Cross-project code search, indexed context, prompt enhancement, and server management."),
    ];

    let global_skills_dir = Some(crate::paths::global_skills_dir());

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

        // 1. Load Global Skills (~/.linggen/skills/)
        {
            let global_dir = crate::paths::global_skills_dir();
            let _ = self
                .load_from_dir_nested(&global_dir, SkillSource::Global, &mut *skills)
                .await;
        }

        // 2. Load Compat Skills (~/.claude/skills/, ~/.codex/skills/)
        for compat_dir in crate::paths::compat_skills_dirs() {
            let _ = self
                .load_from_dir_nested(&compat_dir, SkillSource::Compat, &mut *skills)
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
        let frontmatter: SkillFrontmatter = serde_yml::from_str(parts[1])?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager() -> SkillManager {
        SkillManager::new()
    }

    #[test]
    fn test_parse_skill_valid() {
        let mgr = make_manager();
        let text = r#"---
name: test-skill
description: A test skill
---
This is the skill content."#;
        let skill = mgr.parse_skill(text, SkillSource::Global).unwrap();
        assert_eq!(skill.name, "test-skill");
        assert_eq!(skill.description, "A test skill");
        assert_eq!(skill.content, "This is the skill content.");
        assert!(skill.user_invocable); // default true
    }

    #[test]
    fn test_parse_skill_no_frontmatter() {
        let mgr = make_manager();
        let err = mgr
            .parse_skill("no frontmatter here", SkillSource::Global)
            .unwrap_err();
        assert!(err.to_string().contains("YAML frontmatter"));
    }

    #[test]
    fn test_parse_skill_missing_closing() {
        let mgr = make_manager();
        let err = mgr
            .parse_skill("---\nname: x\ndescription: y\n", SkillSource::Global)
            .unwrap_err();
        assert!(err.to_string().contains("closing frontmatter"));
    }

    #[test]
    fn test_parse_skill_with_trigger() {
        let mgr = make_manager();
        let text = r#"---
name: commit
description: Commit helper
trigger: "/commit"
---
Help commit."#;
        let skill = mgr.parse_skill(text, SkillSource::Project).unwrap();
        assert_eq!(skill.trigger.as_deref(), Some("/commit"));
    }

    #[test]
    fn test_parse_skill_with_all_optional_fields() {
        let mgr = make_manager();
        let text = r#"---
name: advanced
description: Advanced skill
user-invocable: false
disable-model-invocation: true
argument-hint: "project name"
allowed-tools: [Read, Write]
model: gpt-4
context: my-context
agent: coder
---
Content."#;
        let skill = mgr.parse_skill(text, SkillSource::Global).unwrap();
        assert!(!skill.user_invocable);
        assert!(skill.disable_model_invocation);
        assert_eq!(skill.argument_hint.as_deref(), Some("project name"));
        assert_eq!(skill.allowed_tools, vec!["Read", "Write"]);
        assert_eq!(skill.model.as_deref(), Some("gpt-4"));
        assert_eq!(skill.context.as_deref(), Some("my-context"));
        assert_eq!(skill.agent.as_deref(), Some("coder"));
    }

    #[tokio::test]
    async fn test_match_trigger_empty() {
        let mgr = make_manager();
        assert!(mgr.match_trigger("hello").await.is_none());
    }

    #[tokio::test]
    async fn test_match_trigger_basic() {
        let mgr = make_manager();
        {
            let mut triggers = mgr.triggers.lock().await;
            triggers.insert("/commit".to_string(), "commit-skill".to_string());
        }
        let result = mgr.match_trigger("/commit fix bug").await;
        assert!(result.is_some());
        let (name, remaining) = result.unwrap();
        assert_eq!(name, "commit-skill");
        assert_eq!(remaining, "fix bug");
    }

    #[tokio::test]
    async fn test_match_trigger_longest_wins() {
        let mgr = make_manager();
        {
            let mut triggers = mgr.triggers.lock().await;
            triggers.insert("/c".to_string(), "short".to_string());
            triggers.insert("/commit".to_string(), "long".to_string());
        }
        let result = mgr.match_trigger("/commit message").await;
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "long");
    }

    #[tokio::test]
    async fn test_match_trigger_no_match() {
        let mgr = make_manager();
        {
            let mut triggers = mgr.triggers.lock().await;
            triggers.insert("/commit".to_string(), "commit-skill".to_string());
        }
        assert!(mgr.match_trigger("hello world").await.is_none());
    }

    #[tokio::test]
    async fn test_load_from_dir_nested() {
        let dir = tempfile::tempdir().unwrap();

        // Create a flat .md skill
        let flat_skill = r#"---
name: flat-skill
description: Flat skill
---
Flat content."#;
        std::fs::write(dir.path().join("flat.md"), flat_skill).unwrap();

        // Create a nested skill
        let nested_dir = dir.path().join("nested-skill");
        std::fs::create_dir(&nested_dir).unwrap();
        let nested_skill = r#"---
name: nested-skill
description: Nested skill
---
Nested content."#;
        std::fs::write(nested_dir.join("SKILL.md"), nested_skill).unwrap();

        let mgr = make_manager();
        let mut skills = std::collections::HashMap::new();
        mgr.load_from_dir_nested(dir.path(), SkillSource::Global, &mut skills)
            .await
            .unwrap();

        assert_eq!(skills.len(), 2);
        assert!(skills.contains_key("flat-skill"));
        assert!(skills.contains_key("nested-skill"));
    }

    #[test]
    fn test_list_builtin_skills() {
        let builtins = list_builtin_skills();
        assert_eq!(builtins.len(), 4);
        let names: Vec<&str> = builtins.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"memory"));
        assert!(names.contains(&"skiller"));
        assert!(names.contains(&"discord"));
        assert!(names.contains(&"linggen"));
    }
}
