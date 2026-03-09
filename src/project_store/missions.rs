use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// The agent that always runs missions.
pub const MISSION_AGENT_ID: &str = "mission";

/// YAML frontmatter fields for a mission `.md` file.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct MissionFrontmatter {
    #[serde(default)]
    schedule: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    #[serde(default)]
    created_at: u64,
}

/// A cron-scheduled mission stored as `~/.linggen/missions/<id>/mission.md`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Mission {
    /// The mission ID — directory name under `~/.linggen/missions/`.
    pub id: String,
    /// Display name (same as id for md-based missions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub schedule: String,
    /// Always "mission".
    #[serde(default = "default_mission_agent")]
    pub agent_id: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional project path this mission is scoped to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub enabled: bool,
    pub created_at: u64,
}

fn default_mission_agent() -> String {
    MISSION_AGENT_ID.to_string()
}

/// A single entry in a mission's run history (`<id>/runs.jsonl`).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MissionRunEntry {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub triggered_at: u64,
    pub status: String,
    pub skipped: bool,
}

// ---------------------------------------------------------------------------
// Cron helpers
// ---------------------------------------------------------------------------

/// Convert a 5-field cron expression to the 7-field format the `cron` crate expects.
fn to_seven_field(schedule: &str) -> Result<String> {
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    if fields.len() != 5 {
        bail!(
            "Invalid cron expression '{}': expected 5 fields (min hour dom month dow)",
            schedule
        );
    }
    let dow = fields[4]
        .split(',')
        .flat_map(|part| {
            if let Some((start_s, end_s)) = part.split_once('-') {
                let start_num = start_s.trim().parse::<u8>().ok();
                let end_num = end_s.trim().parse::<u8>().ok();
                match (start_num, end_num) {
                    (Some(0), Some(e)) if e >= 1 => {
                        vec![format!("1-{}", e), "7".to_string()]
                    }
                    (Some(s), Some(0)) if s >= 1 => {
                        vec![format!("{}-7", s)]
                    }
                    _ => vec![part.to_string()],
                }
            } else if part.trim() == "0" {
                vec!["7".to_string()]
            } else {
                vec![part.to_string()]
            }
        })
        .collect::<Vec<_>>()
        .join(",");

    Ok(format!(
        "0 {} {} {} {} {} *",
        fields[0], fields[1], fields[2], fields[3], dow
    ))
}

/// Validate a 5-field cron expression.
pub fn validate_cron(schedule: &str) -> Result<()> {
    let seven = to_seven_field(schedule)?;
    seven.parse::<cron::Schedule>().map_err(|e| {
        anyhow::anyhow!("Invalid cron expression '{}': {}", schedule, e)
    })?;
    Ok(())
}

/// Parse a 5-field cron expression into a `cron::Schedule`.
pub fn parse_cron(schedule: &str) -> Result<cron::Schedule> {
    let seven = to_seven_field(schedule)?;
    seven
        .parse::<cron::Schedule>()
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{}': {}", schedule, e))
}

// ---------------------------------------------------------------------------
// Markdown serialisation helpers
// ---------------------------------------------------------------------------

/// Parse a mission `.md` file: YAML frontmatter + markdown body = prompt.
fn parse_mission_md(id: &str, content: &str) -> Result<Mission> {
    let id = id.to_string();

    let (fm, body) = if content.starts_with("---") {
        // Find closing ---
        if let Some(end) = content[3..].find("\n---") {
            let yaml = &content[3..3 + end];
            let body = &content[3 + end + 4..]; // skip "\n---"
            let fm: MissionFrontmatter = serde_yml::from_str(yaml.trim())
                .map_err(|e| anyhow::anyhow!("Bad frontmatter in {}: {}", id, e))?;
            (fm, body.trim().to_string())
        } else {
            // No closing --- — treat entire file as prompt
            (
                MissionFrontmatter {
                    schedule: String::new(),
                    enabled: false,
                    model: None,
                    project: None,
                    created_at: 0,
                },
                content.to_string(),
            )
        }
    } else {
        // No frontmatter at all
        (
            MissionFrontmatter {
                schedule: String::new(),
                enabled: false,
                model: None,
                project: None,
                created_at: 0,
            },
            content.to_string(),
        )
    };

    Ok(Mission {
        name: Some(id_to_display_name(&id)),
        id,
        schedule: fm.schedule,
        agent_id: MISSION_AGENT_ID.to_string(),
        prompt: body,
        model: fm.model,
        project: fm.project,
        enabled: fm.enabled,
        created_at: fm.created_at,
    })
}

/// Convert a mission to its `.md` file content.
fn mission_to_md(mission: &Mission) -> String {
    let fm = MissionFrontmatter {
        schedule: mission.schedule.clone(),
        enabled: mission.enabled,
        model: mission.model.clone(),
        project: mission.project.clone(),
        created_at: mission.created_at,
    };
    let yaml = serde_yml::to_string(&fm).unwrap_or_default();
    format!("---\n{}---\n\n{}\n", yaml, mission.prompt)
}

/// Convert id like "daily-code-review" to "Daily Code Review".
fn id_to_display_name(id: &str) -> String {
    id.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + &chars.collect::<String>(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Sanitize a name to a safe filename (lowercase, hyphens, no special chars).
fn name_to_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else if c == ' ' || c == '_' {
                '-'
            } else {
                '-'
            }
        })
        .collect();
    // Collapse multiple hyphens
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in sanitized.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    result.trim_matches('-').to_string()
}

// ---------------------------------------------------------------------------
// MissionStore — global mission storage at ~/.linggen/missions/
// ---------------------------------------------------------------------------

pub struct MissionStore {
    dir: PathBuf,
}

impl MissionStore {
    pub fn new() -> Self {
        Self {
            dir: crate::paths::global_missions_dir(),
        }
    }

    #[cfg(test)]
    pub fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    fn mission_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    fn mission_path(&self, id: &str) -> PathBuf {
        self.dir.join(id).join("mission.md")
    }

    fn runs_path(&self, id: &str) -> PathBuf {
        self.dir.join(id).join("runs.jsonl")
    }

    pub fn create_mission(
        &self,
        name: Option<String>,
        schedule: &str,
        prompt: &str,
        model: Option<String>,
        project: Option<String>,
    ) -> Result<Mission> {
        validate_cron(schedule)?;
        self.ensure_dir()?;

        let display_name = name.unwrap_or_else(|| "new-mission".to_string());
        let mut id = name_to_filename(&display_name);
        if id.is_empty() {
            id = format!("mission-{}", crate::util::now_ts_secs());
        }

        // Ensure unique directory name
        if self.mission_dir(&id).exists() {
            let base = id.clone();
            let mut n = 2;
            loop {
                id = format!("{}-{}", base, n);
                if !self.mission_dir(&id).exists() {
                    break;
                }
                n += 1;
            }
        }

        let mission = Mission {
            id: id.clone(),
            name: Some(display_name),
            schedule: schedule.to_string(),
            agent_id: MISSION_AGENT_ID.to_string(),
            prompt: prompt.to_string(),
            model,
            project,
            enabled: true,
            created_at: crate::util::now_ts_secs(),
        };

        fs::create_dir_all(self.mission_dir(&id))?;
        let content = mission_to_md(&mission);
        fs::write(self.mission_path(&id), content)?;

        Ok(mission)
    }

    pub fn get_mission(&self, mission_id: &str) -> Result<Option<Mission>> {
        let path = self.mission_path(mission_id);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        let mission = parse_mission_md(mission_id, &content)?;
        Ok(Some(mission))
    }

    pub fn update_mission(
        &self,
        mission_id: &str,
        name: Option<Option<String>>,
        schedule: Option<&str>,
        prompt: Option<&str>,
        model: Option<Option<String>>,
        project: Option<Option<String>>,
        enabled: Option<bool>,
    ) -> Result<Mission> {
        let Some(mut mission) = self.get_mission(mission_id)? else {
            bail!("Mission '{}' not found", mission_id);
        };

        if let Some(n) = name {
            mission.name = n;
        }
        if let Some(s) = schedule {
            validate_cron(s)?;
            mission.schedule = s.to_string();
        }
        if let Some(p) = prompt {
            mission.prompt = p.to_string();
        }
        if let Some(m) = model {
            mission.model = m;
        }
        if let Some(p) = project {
            mission.project = p;
        }
        if let Some(e) = enabled {
            mission.enabled = e;
        }

        let content = mission_to_md(&mission);
        fs::write(self.mission_path(mission_id), content)?;

        Ok(mission)
    }

    pub fn delete_mission(&self, mission_id: &str) -> Result<()> {
        let dir = self.mission_dir(mission_id);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    pub fn list_all_missions(&self) -> Result<Vec<Mission>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut missions = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let mission_file = path.join("mission.md");
            if !mission_file.exists() {
                continue;
            }
            let id = path.file_name().unwrap().to_string_lossy().to_string();
            let content = match fs::read_to_string(&mission_file) {
                Ok(c) => c,
                Err(_) => continue,
            };
            match parse_mission_md(&id, &content) {
                Ok(m) => missions.push(m),
                Err(e) => {
                    tracing::warn!("Skipping corrupt mission dir {}: {}", id, e);
                }
            }
        }
        missions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(missions)
    }

    pub fn list_enabled_missions(&self) -> Result<Vec<Mission>> {
        Ok(self
            .list_all_missions()?
            .into_iter()
            .filter(|m| m.enabled)
            .collect())
    }

    pub fn append_mission_run(
        &self,
        mission_id: &str,
        entry: &MissionRunEntry,
    ) -> Result<()> {
        fs::create_dir_all(self.mission_dir(mission_id))?;
        let path = self.runs_path(mission_id);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let line = serde_json::to_string(entry)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    pub fn list_mission_runs(&self, mission_id: &str) -> Result<Vec<MissionRunEntry>> {
        let path = self.runs_path(mission_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path)?;
        let reader = std::io::BufReader::new(file);
        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<MissionRunEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    tracing::warn!("Skipping corrupt mission run entry: {}", e);
                }
            }
        }
        Ok(entries)
    }
}

// ---------------------------------------------------------------------------
// Migration from per-project JSON to global markdown
// ---------------------------------------------------------------------------

use super::ProjectStore;

/// Old JSON-based mission for migration.
#[derive(Deserialize)]
struct OldJsonMission {
    id: String,
    #[serde(default)]
    name: Option<String>,
    schedule: String,
    #[serde(default)]
    agent_id: String,
    prompt: String,
    #[serde(default)]
    model: Option<String>,
    enabled: bool,
    created_at: u64,
}

/// Legacy mission format (even older).
#[derive(Deserialize)]
struct OldLegacyMission {
    text: String,
    created_at: u64,
    #[allow(dead_code)]
    active: bool,
    #[serde(default)]
    #[allow(dead_code)]
    agents: Vec<OldLegacyAgent>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct OldLegacyAgent {
    id: String,
    #[serde(default)]
    idle_prompt: Option<String>,
    #[serde(default)]
    idle_interval_secs: Option<u64>,
}

impl MissionStore {
    /// Migrate missions from old per-project storage to global markdown files.
    /// Scans `~/.linggen/projects/*/missions/` for JSON missions.
    pub fn migrate_from_project_store(&self, store: &ProjectStore) -> Result<()> {
        let projects = store.list_projects().unwrap_or_default();
        for project in &projects {
            self.migrate_project_missions(store, &project.path)?;
        }
        Ok(())
    }

    fn migrate_project_missions(&self, store: &ProjectStore, project_path: &str) -> Result<()> {
        let old_dir = store.project_dir(project_path).join("missions");
        if !old_dir.exists() {
            return Ok(());
        }

        // Check for legacy flat mission.json at project root
        let legacy = store.project_dir(project_path).join("mission.json");
        if legacy.exists() {
            if let Ok(content) = fs::read_to_string(&legacy) {
                if let Ok(old) = serde_json::from_str::<OldLegacyMission>(&content) {
                    let _ = self.migrate_legacy_mission(&old, project_path);
                }
            }
            let _ = fs::remove_file(&legacy);
        }

        for entry in fs::read_dir(&old_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // New-ish format: directory with mission.json inside
                let json_path = path.join("mission.json");
                if json_path.exists() {
                    if let Ok(content) = fs::read_to_string(&json_path) {
                        if let Ok(old) = serde_json::from_str::<OldJsonMission>(&content) {
                            let _ = self.migrate_json_mission(&old, project_path);
                        }
                    }
                }
            } else if path.extension().map(|e| e == "json").unwrap_or(false) {
                // Old flat format: {timestamp}.json
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(old) = serde_json::from_str::<OldLegacyMission>(&content) {
                        let _ = self.migrate_legacy_mission(&old, project_path);
                    }
                }
                let _ = fs::remove_file(&path);
            }
        }

        Ok(())
    }

    fn migrate_json_mission(&self, old: &OldJsonMission, project_path: &str) -> Result<Mission> {
        self.ensure_dir()?;
        let name = old.name.clone().unwrap_or_else(|| old.id.clone());
        let id = name_to_filename(&name);
        if id.is_empty() || self.mission_dir(&id).exists() {
            // Already migrated or conflict
            return Ok(Mission {
                id,
                name: Some(name),
                schedule: old.schedule.clone(),
                agent_id: MISSION_AGENT_ID.to_string(),
                prompt: old.prompt.clone(),
                model: old.model.clone(),
                project: Some(project_path.to_string()),
                enabled: old.enabled,
                created_at: old.created_at,
            });
        }

        let mission = Mission {
            id: id.clone(),
            name: Some(name),
            schedule: old.schedule.clone(),
            agent_id: MISSION_AGENT_ID.to_string(),
            prompt: old.prompt.clone(),
            model: old.model.clone(),
            project: Some(project_path.to_string()),
            enabled: old.enabled,
            created_at: old.created_at,
        };

        fs::create_dir_all(self.mission_dir(&id))?;
        let content = mission_to_md(&mission);
        fs::write(self.mission_path(&id), content)?;

        // Migrate runs too
        let old_runs_path = crate::paths::projects_dir()
            .join(super::path_encoding::encode_project_path(project_path))
            .join("missions")
            .join(&old.id)
            .join("runs.jsonl");
        if old_runs_path.exists() {
            if let Ok(runs_content) = fs::read_to_string(&old_runs_path) {
                let _ = fs::write(self.runs_path(&id), runs_content);
            }
        }

        Ok(mission)
    }

    fn migrate_legacy_mission(
        &self,
        old: &OldLegacyMission,
        project_path: &str,
    ) -> Result<Mission> {
        self.ensure_dir()?;
        let id = format!("migrated-{}", old.created_at);
        if self.mission_dir(&id).exists() {
            bail!("Already migrated");
        }

        let mission = Mission {
            id: id.clone(),
            name: None,
            schedule: "0 * * * *".to_string(),
            agent_id: MISSION_AGENT_ID.to_string(),
            prompt: old.text.clone(),
            model: None,
            project: Some(project_path.to_string()),
            enabled: false, // disabled — user should configure and re-enable
            created_at: old.created_at,
        };

        fs::create_dir_all(self.mission_dir(&id))?;
        let content = mission_to_md(&mission);
        fs::write(self.mission_path(&id), content)?;

        Ok(mission)
    }

    /// Migrate from old flat-file format (`<id>.md` + `<id>.runs.jsonl`) to
    /// directory-based format (`<id>/mission.md` + `<id>/runs.jsonl`).
    pub fn migrate_flat_to_dirs(&self) -> Result<()> {
        if !self.dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(ext) = path.extension() else {
                continue;
            };
            if ext != "md" {
                continue;
            }
            let filename = path.file_name().unwrap().to_string_lossy().to_string();
            let id = filename.strip_suffix(".md").unwrap_or(&filename).to_string();

            // Read the old flat file
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Create directory and write mission.md inside
            let mission_dir = self.dir.join(&id);
            if mission_dir.exists() {
                continue; // already migrated
            }
            fs::create_dir_all(&mission_dir)?;
            fs::write(mission_dir.join("mission.md"), &content)?;

            // Move runs file if it exists
            let old_runs = self.dir.join(format!("{}.runs.jsonl", id));
            if old_runs.exists() {
                let runs_content = fs::read_to_string(&old_runs)?;
                fs::write(mission_dir.join("runs.jsonl"), runs_content)?;
                fs::remove_file(&old_runs)?;
            }

            // Remove old flat file
            fs::remove_file(&path)?;
            tracing::info!("Migrated mission '{}' from flat file to directory", id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (MissionStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = MissionStore::with_dir(dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn test_validate_cron() {
        assert!(validate_cron("*/30 * * * *").is_ok());
        assert!(validate_cron("0 9 * * 1-5").is_ok());
        assert!(validate_cron("0 0 * * 0").is_ok());
        assert!(validate_cron("0 0 * * SUN").is_ok());
        assert!(validate_cron("0 */2 * * *").is_ok());
        assert!(validate_cron("0 9 * * 0-5").is_ok());
        assert!(validate_cron("0 9 * * 0,3,5").is_ok());
        assert!(validate_cron("invalid").is_err());
        assert!(validate_cron("").is_err());
        assert!(validate_cron("* * *").is_err());
    }

    #[test]
    fn test_create_and_list_missions() {
        let (store, _dir) = temp_store();

        let m1 = store
            .create_mission(Some("Check Status".into()), "*/30 * * * *", "Check status", None, None)
            .unwrap();
        assert_eq!(m1.id, "check-status");
        assert!(m1.enabled);
        assert_eq!(m1.agent_id, MISSION_AGENT_ID);

        let m2 = store
            .create_mission(Some("Review Code".into()), "0 9 * * 1-5", "Review code", Some("gpt-4".into()), Some("/tmp/proj".into()))
            .unwrap();
        assert_eq!(m2.id, "review-code");
        assert_eq!(m2.project, Some("/tmp/proj".to_string()));

        let all = store.list_all_missions().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_md_roundtrip() {
        let (store, _dir) = temp_store();

        let created = store
            .create_mission(Some("Daily Cleanup".into()), "0 9 * * *", "Clean up old files\n\nRemove build artifacts.", Some("gpt-4".into()), Some("/tmp/proj".into()))
            .unwrap();

        let loaded = store.get_mission("daily-cleanup").unwrap().unwrap();
        assert_eq!(loaded.schedule, "0 9 * * *");
        assert_eq!(loaded.prompt, "Clean up old files\n\nRemove build artifacts.");
        assert_eq!(loaded.model, Some("gpt-4".to_string()));
        assert_eq!(loaded.project, Some("/tmp/proj".to_string()));
        assert_eq!(loaded.enabled, true);
        assert_eq!(loaded.created_at, created.created_at);
    }

    #[test]
    fn test_update_delete() {
        let (store, _dir) = temp_store();

        let m = store
            .create_mission(Some("Test".into()), "0 * * * *", "Hello", None, None)
            .unwrap();

        let updated = store
            .update_mission(&m.id, None, Some("*/15 * * * *"), Some("Updated prompt"), None, None, Some(false))
            .unwrap();
        assert_eq!(updated.schedule, "*/15 * * * *");
        assert_eq!(updated.prompt, "Updated prompt");
        assert!(!updated.enabled);

        assert_eq!(store.list_enabled_missions().unwrap().len(), 0);

        store.delete_mission(&m.id).unwrap();
        assert!(store.get_mission(&m.id).unwrap().is_none());
    }

    #[test]
    fn test_run_history() {
        let (store, _dir) = temp_store();

        let m = store
            .create_mission(Some("Test".into()), "0 * * * *", "Test", None, None)
            .unwrap();

        let entry1 = MissionRunEntry {
            run_id: "run-1".into(),
            session_id: Some("sess-1".into()),
            triggered_at: 1000,
            status: "completed".into(),
            skipped: false,
        };
        let entry2 = MissionRunEntry {
            run_id: "run-2".into(),
            session_id: None,
            triggered_at: 2000,
            status: "skipped".into(),
            skipped: true,
        };

        store.append_mission_run(&m.id, &entry1).unwrap();
        store.append_mission_run(&m.id, &entry2).unwrap();

        let runs = store.list_mission_runs(&m.id).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, "run-1");
        assert!(runs[1].skipped);
    }

    #[test]
    fn test_name_to_filename() {
        assert_eq!(name_to_filename("Daily Code Review"), "daily-code-review");
        assert_eq!(name_to_filename("clean disk"), "clean-disk");
        assert_eq!(name_to_filename("  hello  world  "), "hello-world");
        assert_eq!(name_to_filename("Test_123"), "test-123");
    }

    #[test]
    fn test_duplicate_name_gets_suffix() {
        let (store, _dir) = temp_store();

        let m1 = store.create_mission(Some("Test".into()), "0 * * * *", "First", None, None).unwrap();
        assert_eq!(m1.id, "test");

        let m2 = store.create_mission(Some("Test".into()), "0 * * * *", "Second", None, None).unwrap();
        assert_eq!(m2.id, "test-2");
    }

    #[test]
    fn test_update_invalid_cron_rejected() {
        let (store, _dir) = temp_store();

        let m = store
            .create_mission(Some("Test".into()), "0 * * * *", "Test", None, None)
            .unwrap();

        let result = store.update_mission(&m.id, None, Some("bad cron"), None, None, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_migrate_flat_to_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // Create old flat-format files
        let md_content = "---\nschedule: '0 9 * * *'\nenabled: true\ncreated_at: 1000\n---\n\nDo stuff\n";
        fs::write(root.join("my-mission.md"), md_content).unwrap();

        let run_entry = r#"{"run_id":"r1","triggered_at":2000,"status":"completed","skipped":false}"#;
        fs::write(root.join("my-mission.runs.jsonl"), format!("{}\n", run_entry)).unwrap();

        let store = MissionStore::with_dir(root.clone());
        store.migrate_flat_to_dirs().unwrap();

        // Old files should be gone
        assert!(!root.join("my-mission.md").exists());
        assert!(!root.join("my-mission.runs.jsonl").exists());

        // New directory structure should exist
        assert!(root.join("my-mission").join("mission.md").exists());
        assert!(root.join("my-mission").join("runs.jsonl").exists());

        // Data should be intact
        let m = store.get_mission("my-mission").unwrap().unwrap();
        assert_eq!(m.schedule, "0 9 * * *");
        assert_eq!(m.prompt, "Do stuff");
        assert!(m.enabled);

        let runs = store.list_mission_runs("my-mission").unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "r1");
    }

    #[test]
    fn test_directory_structure() {
        let (store, dir) = temp_store();
        let root = dir.path().to_path_buf();

        let m = store
            .create_mission(Some("Test Dir".into()), "0 * * * *", "Hello", None, None)
            .unwrap();

        // Verify directory-based layout
        assert!(root.join("test-dir").is_dir());
        assert!(root.join("test-dir").join("mission.md").exists());

        // Runs go inside the same directory
        let entry = MissionRunEntry {
            run_id: "r1".into(),
            session_id: None,
            triggered_at: 1000,
            status: "completed".into(),
            skipped: false,
        };
        store.append_mission_run(&m.id, &entry).unwrap();
        assert!(root.join("test-dir").join("runs.jsonl").exists());

        // Delete removes entire directory
        store.delete_mission(&m.id).unwrap();
        assert!(!root.join("test-dir").exists());
    }
}
