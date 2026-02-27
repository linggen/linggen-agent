use crate::engine::render::normalize_tool_path_arg;
use crate::engine::tools::{AskUserOption, AskUserQuestion};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

// ---------------------------------------------------------------------------
// Permission action returned after prompting the user
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionAction {
    AllowOnce,
    AllowSession,
    AllowProject,
    Deny,
}

// ---------------------------------------------------------------------------
// Persisted format for project-level permissions
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Default)]
struct PersistedPermissions {
    #[serde(default)]
    tool_allows: HashSet<String>,
}

// ---------------------------------------------------------------------------
// PermissionStore
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PermissionStore {
    session_allows: HashSet<String>,
    project_allows: HashSet<String>,
    project_file: Option<PathBuf>,
}

impl PermissionStore {
    /// Load project-scoped permissions from disk.
    /// `project_dir` is `{workspace}/.linggen/` (same pattern as Claude Code's `.claude/`).
    pub fn load(project_dir: &Path) -> Self {
        let file = project_dir.join("permissions.json");
        let project_allows = if file.exists() {
            match fs::read_to_string(&file) {
                Ok(content) => match serde_json::from_str::<PersistedPermissions>(&content) {
                    Ok(p) => p.tool_allows,
                    Err(e) => {
                        warn!("Failed to parse permissions.json: {}", e);
                        HashSet::new()
                    }
                },
                Err(e) => {
                    warn!("Failed to read permissions.json: {}", e);
                    HashSet::new()
                }
            }
        } else {
            HashSet::new()
        };
        Self {
            session_allows: HashSet::new(),
            project_allows,
            project_file: Some(file),
        }
    }

    /// Create an empty store (no persistence).
    #[allow(dead_code)]
    pub fn empty() -> Self {
        Self {
            session_allows: HashSet::new(),
            project_allows: HashSet::new(),
            project_file: None,
        }
    }

    /// Check whether the tool is allowed (session OR project scope).
    pub fn check(&self, tool: &str) -> bool {
        self.session_allows.contains(tool) || self.project_allows.contains(tool)
    }

    /// Allow a tool for the remainder of this session/task.
    pub fn allow_for_session(&mut self, tool: &str) {
        self.session_allows.insert(tool.to_string());
    }

    /// Allow a tool for this project (persisted to disk).
    pub fn allow_for_project(&mut self, tool: &str) {
        self.project_allows.insert(tool.to_string());
        self.persist();
    }

    /// Clear session-scoped permissions (called on session reset).
    #[allow(dead_code)]
    pub fn clear_session(&mut self) {
        self.session_allows.clear();
    }

    fn persist(&self) {
        let Some(file) = &self.project_file else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let data = PersistedPermissions {
            tool_allows: self.project_allows.clone(),
        };
        match serde_json::to_string_pretty(&data) {
            Ok(json) => {
                if let Err(e) = fs::write(file, json) {
                    warn!("Failed to write permissions.json: {}", e);
                }
            }
            Err(e) => warn!("Failed to serialize permissions: {}", e),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true for tools that modify the filesystem or execute commands.
pub fn is_destructive_tool(tool: &str) -> bool {
    matches!(tool, "Write" | "Edit" | "Bash" | "Patch")
}

/// Returns true for tools that make network requests (WebFetch, WebSearch).
pub fn is_web_tool(tool: &str) -> bool {
    matches!(tool, "WebFetch" | "WebSearch")
}

/// Build a human-readable summary of what the tool is about to do.
pub fn permission_target_summary(tool: &str, args: &serde_json::Value, ws_root: &Path) -> String {
    match tool {
        "Write" | "Edit" => normalize_tool_path_arg(ws_root, args)
            .unwrap_or_else(|| "<unknown file>".to_string()),
        "Bash" => args
            .get("cmd")
            .or_else(|| args.get("command"))
            .and_then(|v| v.as_str())
            .map(|cmd| {
                if cmd.len() > 80 {
                    format!("{}...", &cmd[..77])
                } else {
                    cmd.to_string()
                }
            })
            .unwrap_or_else(|| "<unknown command>".to_string()),
        "Patch" => {
            // Try to extract the first file from a unified diff header.
            args.get("diff")
                .or_else(|| args.get("patch"))
                .and_then(|v| v.as_str())
                .and_then(|diff| {
                    diff.lines().find_map(|line| {
                        line.strip_prefix("+++ b/")
                            .or_else(|| line.strip_prefix("+++ "))
                            .map(|s| s.to_string())
                    })
                })
                .unwrap_or_else(|| "<patch>".to_string())
        }
        "WebFetch" => args
            .get("url")
            .and_then(|v| v.as_str())
            .map(|url| {
                if url.len() > 120 {
                    format!("{}...", &url[..117])
                } else {
                    url.to_string()
                }
            })
            .unwrap_or_else(|| "<unknown URL>".to_string()),
        "WebSearch" => args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| {
                if q.len() > 120 {
                    format!("{}...", &q[..117])
                } else {
                    q.to_string()
                }
            })
            .unwrap_or_else(|| "<unknown query>".to_string()),
        _ => tool.to_string(),
    }
}

/// Build the AskUser question for a permission prompt.
pub fn build_permission_question(tool: &str, target_summary: &str) -> AskUserQuestion {
    AskUserQuestion {
        question: format!("{} {}", tool, target_summary),
        header: "Permission".to_string(),
        options: vec![
            AskUserOption {
                label: "Allow once".to_string(),
                description: Some("Proceed this one time only".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Allow all {} for this task", tool),
                description: Some("Session-scoped; resets on new session".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Allow all {} for this project", tool),
                description: Some("Persisted; won't ask again for this project".to_string()),
                preview: None,
            },
            AskUserOption {
                label: "Cancel".to_string(),
                description: Some("Deny this tool call".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    }
}

/// Build the AskUser question for a web tool permission prompt.
/// Uses a simpler 3-option layout (no project-level persistence for web tools).
pub fn build_web_permission_question(tool: &str, target_summary: &str) -> AskUserQuestion {
    AskUserQuestion {
        question: format!("{} {}", tool, target_summary),
        header: "Permission".to_string(),
        options: vec![
            AskUserOption {
                label: "Allow this request".to_string(),
                description: Some("Proceed this one time only".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Allow all {} for this task", tool),
                description: Some("Session-scoped; resets on new session".to_string()),
                preview: None,
            },
            AskUserOption {
                label: "Cancel".to_string(),
                description: Some("Deny this web request".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    }
}

/// Parse the selected option from a web permission prompt.
pub fn parse_web_permission_answer(selected: &str, tool: &str) -> PermissionAction {
    if selected == "Allow this request" {
        PermissionAction::AllowOnce
    } else if selected == format!("Allow all {} for this task", tool) {
        PermissionAction::AllowSession
    } else {
        PermissionAction::Deny
    }
}

/// Parse the selected option label back into a PermissionAction.
pub fn parse_permission_answer(selected: &str, tool: &str) -> PermissionAction {
    if selected == "Allow once" {
        PermissionAction::AllowOnce
    } else if selected == format!("Allow all {} for this task", tool) {
        PermissionAction::AllowSession
    } else if selected == format!("Allow all {} for this project", tool) {
        PermissionAction::AllowProject
    } else {
        // "Cancel" or anything unexpected
        PermissionAction::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_destructive() {
        assert!(is_destructive_tool("Write"));
        assert!(is_destructive_tool("Edit"));
        assert!(is_destructive_tool("Bash"));
        assert!(is_destructive_tool("Patch"));
        assert!(!is_destructive_tool("Read"));
        assert!(!is_destructive_tool("Glob"));
        assert!(!is_destructive_tool("Grep"));
    }

    #[test]
    fn test_permission_store_session() {
        let mut store = PermissionStore::empty();
        assert!(!store.check("Write"));
        store.allow_for_session("Write");
        assert!(store.check("Write"));
        assert!(!store.check("Bash"));
        store.clear_session();
        assert!(!store.check("Write"));
    }

    #[test]
    fn test_permission_store_load_persist() {
        let tmp = std::env::temp_dir().join("linggen_perm_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut store = PermissionStore::load(&tmp);
        assert!(!store.check("Edit"));
        store.allow_for_project("Edit");
        assert!(store.check("Edit"));

        // Reload from disk
        let store2 = PermissionStore::load(&tmp);
        assert!(store2.check("Edit"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_parse_permission_answer() {
        assert_eq!(parse_permission_answer("Allow once", "Write"), PermissionAction::AllowOnce);
        assert_eq!(
            parse_permission_answer("Allow all Write for this task", "Write"),
            PermissionAction::AllowSession
        );
        assert_eq!(
            parse_permission_answer("Allow all Bash for this project", "Bash"),
            PermissionAction::AllowProject
        );
        assert_eq!(parse_permission_answer("Cancel", "Write"), PermissionAction::Deny);
        assert_eq!(parse_permission_answer("anything else", "Write"), PermissionAction::Deny);
    }

    #[test]
    fn test_build_permission_question() {
        let q = build_permission_question("Write", "src/main.rs");
        assert_eq!(q.question, "Write src/main.rs");
        assert_eq!(q.header, "Permission");
        assert_eq!(q.options.len(), 4);
        assert_eq!(q.options[0].label, "Allow once");
        assert!(q.options[1].label.contains("Write"));
    }

    #[test]
    fn test_permission_target_summary_bash() {
        let args = serde_json::json!({ "cmd": "cargo build" });
        let summary = permission_target_summary("Bash", &args, Path::new("/tmp"));
        assert_eq!(summary, "cargo build");
    }

    #[test]
    fn test_permission_target_summary_write() {
        let args = serde_json::json!({ "path": "src/main.rs", "content": "fn main() {}" });
        let summary = permission_target_summary("Write", &args, Path::new("/tmp"));
        assert_eq!(summary, "src/main.rs");
    }

    #[test]
    fn test_is_web_tool() {
        assert!(is_web_tool("WebFetch"));
        assert!(is_web_tool("WebSearch"));
        assert!(!is_web_tool("Read"));
        assert!(!is_web_tool("Bash"));
        assert!(!is_web_tool("Write"));
    }

    #[test]
    fn test_permission_target_summary_webfetch() {
        let args = serde_json::json!({ "url": "https://example.com/docs" });
        let summary = permission_target_summary("WebFetch", &args, Path::new("/tmp"));
        assert_eq!(summary, "https://example.com/docs");
    }

    #[test]
    fn test_permission_target_summary_websearch() {
        let args = serde_json::json!({ "query": "rust async patterns" });
        let summary = permission_target_summary("WebSearch", &args, Path::new("/tmp"));
        assert_eq!(summary, "rust async patterns");
    }

    #[test]
    fn test_build_web_permission_question() {
        let q = build_web_permission_question("WebFetch", "https://example.com");
        assert_eq!(q.question, "WebFetch https://example.com");
        assert_eq!(q.options.len(), 3);
        assert_eq!(q.options[0].label, "Allow this request");
        assert!(q.options[1].label.contains("WebFetch"));
        assert_eq!(q.options[2].label, "Cancel");
    }

    #[test]
    fn test_parse_web_permission_answer() {
        assert_eq!(
            parse_web_permission_answer("Allow this request", "WebFetch"),
            PermissionAction::AllowOnce
        );
        assert_eq!(
            parse_web_permission_answer("Allow all WebFetch for this task", "WebFetch"),
            PermissionAction::AllowSession
        );
        assert_eq!(
            parse_web_permission_answer("Cancel", "WebFetch"),
            PermissionAction::Deny
        );
    }
}
