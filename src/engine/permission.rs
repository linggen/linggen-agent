use crate::engine::render::normalize_tool_path_arg;
use crate::engine::tools::{AskUserOption, AskUserQuestion};
use globset::Glob;
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
    ///
    /// For Bash commands, pass the command string to enable pattern-based matching.
    /// A blanket `"Bash"` entry still grants access to all commands (backward compat).
    /// Pattern entries like `"Bash:npm run *"` only match commands that fit the glob.
    pub fn check(&self, tool: &str, command: Option<&str>) -> bool {
        // 1. Blanket tool-level allow (backward compat)
        if self.session_allows.contains(tool) || self.project_allows.contains(tool) {
            return true;
        }
        // 2. Pattern-based matching for Bash commands
        if let Some(cmd) = command {
            let prefix = format!("{}:", tool);
            for entry in self.session_allows.iter().chain(self.project_allows.iter()) {
                if let Some(pattern) = entry.strip_prefix(&prefix) {
                    if command_matches_pattern(cmd, pattern) {
                        return true;
                    }
                }
            }
        }
        false
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
// Bash command pattern helpers
// ---------------------------------------------------------------------------

/// Detects shell operators that chain multiple commands.
/// Compound commands always require explicit approval (no pattern derivation).
pub fn is_compound_command(cmd: &str) -> bool {
    // Check for common shell chaining/substitution operators.
    // We scan character-by-character to avoid false positives inside quotes,
    // but for simplicity we use a heuristic approach on the raw string.
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    for i in 0..len {
        match bytes[i] {
            b';' => return true,
            b'|' => return true, // covers `|` and `||`
            b'&' if i + 1 < len && bytes[i + 1] == b'&' => return true,
            b'`' => return true,
            b'$' if i + 1 < len && bytes[i + 1] == b'(' => return true,
            _ => {}
        }
    }
    false
}

/// Extracts a glob pattern from a simple (non-compound) command.
///
/// - Returns `None` for compound commands (always ask).
/// - `"pwd"` → `"pwd"` (single word = exact match)
/// - `"git status"` → `"git *"` (two words = first token + wildcard)
/// - `"npm run build"` → `"npm run *"` (3+ words = first two tokens + wildcard)
/// - `"ls -la"` → `"ls *"` (second token starts with `-` = first token + wildcard)
pub fn derive_command_pattern(cmd: &str) -> Option<String> {
    if is_compound_command(cmd) {
        return None;
    }
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    match tokens.len() {
        0 => None,
        1 => Some(tokens[0].to_string()),
        2 => {
            // If second token is a flag, use just "program *"
            // Otherwise "program subcommand" → "program *"
            Some(format!("{} *", tokens[0]))
        }
        _ => {
            // 3+ tokens: if second token starts with `-`, use "program *"
            if tokens[1].starts_with('-') {
                Some(format!("{} *", tokens[0]))
            } else {
                Some(format!("{} {} *", tokens[0], tokens[1]))
            }
        }
    }
}

/// Matches a command against a stored glob pattern.
/// Falls back to exact string comparison if glob compilation fails.
pub fn command_matches_pattern(cmd: &str, pattern: &str) -> bool {
    match Glob::new(pattern) {
        Ok(glob) => glob.compile_matcher().is_match(cmd),
        Err(_) => cmd == pattern,
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

/// Build the AskUser question for a Bash permission prompt with command-level granularity.
///
/// If a pattern was derived (simple command), offers pattern-scoped options.
/// If no pattern (compound command), only offers blanket allow or cancel.
pub fn build_bash_permission_question(command: &str, pattern: Option<&str>) -> AskUserQuestion {
    let mut options = vec![AskUserOption {
        label: "Allow once".to_string(),
        description: Some("Proceed this one time only".to_string()),
        preview: None,
    }];

    if let Some(pat) = pattern {
        options.push(AskUserOption {
            label: format!("Allow Bash({}) for this task", pat),
            description: Some("Session-scoped; resets on new session".to_string()),
            preview: None,
        });
        options.push(AskUserOption {
            label: format!("Allow Bash({}) for this project", pat),
            description: Some("Persisted; won't ask again for this project".to_string()),
            preview: None,
        });
    }

    options.push(AskUserOption {
        label: "Allow all Bash for this task".to_string(),
        description: Some("Session-scoped; allows any Bash command".to_string()),
        preview: None,
    });

    options.push(AskUserOption {
        label: "Cancel".to_string(),
        description: Some("Deny this command".to_string()),
        preview: None,
    });

    AskUserQuestion {
        question: format!("Bash {}", command),
        header: "Permission".to_string(),
        options,
        multi_select: false,
    }
}

/// Parse the selected option from a Bash permission prompt.
/// Returns `(action, permission_key)` where `permission_key` is the string to store
/// (e.g., `"Bash:npm run *"` for pattern-scoped, `"Bash"` for blanket).
pub fn parse_bash_permission_answer(
    selected: &str,
    _tool: &str,
    pattern: Option<&str>,
) -> (PermissionAction, Option<String>) {
    if selected == "Allow once" {
        return (PermissionAction::AllowOnce, None);
    }
    if selected == "Allow all Bash for this task" {
        return (PermissionAction::AllowSession, Some("Bash".to_string()));
    }
    if let Some(pat) = pattern {
        let key = format!("Bash:{}", pat);
        if selected == format!("Allow Bash({}) for this task", pat) {
            return (PermissionAction::AllowSession, Some(key));
        }
        if selected == format!("Allow Bash({}) for this project", pat) {
            return (PermissionAction::AllowProject, Some(key));
        }
    }
    // "Cancel" or anything unexpected
    (PermissionAction::Deny, None)
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
        assert!(!store.check("Write", None));
        store.allow_for_session("Write");
        assert!(store.check("Write", None));
        assert!(!store.check("Bash", None));
        store.clear_session();
        assert!(!store.check("Write", None));
    }

    #[test]
    fn test_permission_store_load_persist() {
        let tmp = std::env::temp_dir().join("linggen_perm_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut store = PermissionStore::load(&tmp);
        assert!(!store.check("Edit", None));
        store.allow_for_project("Edit");
        assert!(store.check("Edit", None));

        // Reload from disk
        let store2 = PermissionStore::load(&tmp);
        assert!(store2.check("Edit", None));

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

    // --- Bash command pattern tests ---

    #[test]
    fn test_is_compound_command() {
        // Compound commands
        assert!(is_compound_command("ls; rm -rf /"));
        assert!(is_compound_command("echo foo && echo bar"));
        assert!(is_compound_command("cat file || true"));
        assert!(is_compound_command("echo `whoami`"));
        assert!(is_compound_command("echo $(whoami)"));
        assert!(is_compound_command("ls | grep foo"));

        // Simple commands
        assert!(!is_compound_command("npm run build"));
        assert!(!is_compound_command("cargo test"));
        assert!(!is_compound_command("git status"));
        assert!(!is_compound_command("ls -la"));
        assert!(!is_compound_command("pwd"));
    }

    #[test]
    fn test_derive_command_pattern() {
        // Single word → exact match
        assert_eq!(derive_command_pattern("pwd"), Some("pwd".to_string()));

        // Two words → "program *"
        assert_eq!(
            derive_command_pattern("git status"),
            Some("git *".to_string())
        );
        assert_eq!(
            derive_command_pattern("ls -la"),
            Some("ls *".to_string())
        );

        // 3+ words → "program subcommand *"
        assert_eq!(
            derive_command_pattern("npm run build"),
            Some("npm run *".to_string())
        );
        assert_eq!(
            derive_command_pattern("cargo test --release"),
            Some("cargo test *".to_string())
        );

        // 3+ words with flag as second token → "program *"
        assert_eq!(
            derive_command_pattern("ls -la /tmp"),
            Some("ls *".to_string())
        );

        // Compound commands → None
        assert_eq!(derive_command_pattern("ls && cat foo"), None);
        assert_eq!(derive_command_pattern("echo $(pwd)"), None);

        // Empty → None
        assert_eq!(derive_command_pattern(""), None);
    }

    #[test]
    fn test_command_matches_pattern() {
        // Glob matching
        assert!(command_matches_pattern("npm run build", "npm run *"));
        assert!(command_matches_pattern("npm run test", "npm run *"));
        assert!(!command_matches_pattern("cargo build", "npm run *"));

        // "git *" matches any git command
        assert!(command_matches_pattern("git status", "git *"));
        assert!(command_matches_pattern("git push origin main", "git *"));
        assert!(!command_matches_pattern("npm install", "git *"));

        // Exact match
        assert!(command_matches_pattern("pwd", "pwd"));
        assert!(!command_matches_pattern("ls", "pwd"));
    }

    #[test]
    fn test_check_with_command_pattern() {
        let mut store = PermissionStore::empty();

        // No permissions → denied
        assert!(!store.check("Bash", Some("npm run build")));

        // Add pattern-scoped permission
        store.allow_for_session("Bash:npm run *");
        assert!(store.check("Bash", Some("npm run build")));
        assert!(store.check("Bash", Some("npm run test")));
        assert!(!store.check("Bash", Some("cargo build")));

        // Multiple patterns
        store.allow_for_session("Bash:cargo *");
        assert!(store.check("Bash", Some("cargo build")));
        assert!(store.check("Bash", Some("cargo test --release")));
    }

    #[test]
    fn test_backward_compat_blanket_allow() {
        let mut store = PermissionStore::empty();

        // Old-style blanket "Bash" entry allows everything
        store.allow_for_session("Bash");
        assert!(store.check("Bash", Some("npm run build")));
        assert!(store.check("Bash", Some("rm -rf /")));
        assert!(store.check("Bash", None));
    }

    #[test]
    fn test_non_bash_tools_unaffected() {
        let mut store = PermissionStore::empty();

        // Write/Edit still use simple matching (no command parameter)
        store.allow_for_session("Write");
        assert!(store.check("Write", None));
        assert!(!store.check("Edit", None));
    }

    #[test]
    fn test_build_bash_permission_question_with_pattern() {
        let q = build_bash_permission_question("npm run build", Some("npm run *"));
        assert_eq!(q.question, "Bash npm run build");
        // With pattern: Allow once, Allow Bash(pattern) task, Allow Bash(pattern) project, Allow all Bash task, Cancel
        assert_eq!(q.options.len(), 5);
        assert_eq!(q.options[0].label, "Allow once");
        assert_eq!(q.options[1].label, "Allow Bash(npm run *) for this task");
        assert_eq!(
            q.options[2].label,
            "Allow Bash(npm run *) for this project"
        );
        assert_eq!(q.options[3].label, "Allow all Bash for this task");
        assert_eq!(q.options[4].label, "Cancel");
    }

    #[test]
    fn test_build_bash_permission_question_no_pattern() {
        let q = build_bash_permission_question("ls && cat foo", None);
        assert_eq!(q.question, "Bash ls && cat foo");
        // Without pattern: Allow once, Allow all Bash task, Cancel
        assert_eq!(q.options.len(), 3);
        assert_eq!(q.options[0].label, "Allow once");
        assert_eq!(q.options[1].label, "Allow all Bash for this task");
        assert_eq!(q.options[2].label, "Cancel");
    }

    #[test]
    fn test_parse_bash_permission_answer_all_paths() {
        let pat = Some("npm run *");

        // Allow once
        let (action, key) = parse_bash_permission_answer("Allow once", "Bash", pat);
        assert_eq!(action, PermissionAction::AllowOnce);
        assert!(key.is_none());

        // Pattern-scoped session
        let (action, key) =
            parse_bash_permission_answer("Allow Bash(npm run *) for this task", "Bash", pat);
        assert_eq!(action, PermissionAction::AllowSession);
        assert_eq!(key.as_deref(), Some("Bash:npm run *"));

        // Pattern-scoped project
        let (action, key) =
            parse_bash_permission_answer("Allow Bash(npm run *) for this project", "Bash", pat);
        assert_eq!(action, PermissionAction::AllowProject);
        assert_eq!(key.as_deref(), Some("Bash:npm run *"));

        // Blanket session
        let (action, key) =
            parse_bash_permission_answer("Allow all Bash for this task", "Bash", pat);
        assert_eq!(action, PermissionAction::AllowSession);
        assert_eq!(key.as_deref(), Some("Bash"));

        // Cancel
        let (action, key) = parse_bash_permission_answer("Cancel", "Bash", pat);
        assert_eq!(action, PermissionAction::Deny);
        assert!(key.is_none());

        // No pattern: only blanket or cancel
        let (action, key) =
            parse_bash_permission_answer("Allow all Bash for this task", "Bash", None);
        assert_eq!(action, PermissionAction::AllowSession);
        assert_eq!(key.as_deref(), Some("Bash"));
    }

    #[test]
    fn test_check_with_project_pattern() {
        let tmp = std::env::temp_dir().join("linggen_perm_pattern_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut store = PermissionStore::load(&tmp);
        store.allow_for_project("Bash:git *");

        // Pattern matches
        assert!(store.check("Bash", Some("git status")));
        assert!(store.check("Bash", Some("git push origin main")));
        assert!(!store.check("Bash", Some("npm install")));

        // Reload from disk and verify persistence
        let store2 = PermissionStore::load(&tmp);
        assert!(store2.check("Bash", Some("git status")));
        assert!(!store2.check("Bash", Some("npm install")));

        let _ = fs::remove_dir_all(&tmp);
    }
}
