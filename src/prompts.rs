//! Prompt template store.
//!
//! Loads prompt templates from `~/.linggen/prompts/` at runtime, falling back
//! to compiled-in defaults from `prompts/` in the source tree.  Templates use
//! `{variable}` placeholders that are substituted via [`PromptStore::render`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Well-known prompt keys
// ---------------------------------------------------------------------------

/// Response format schema injected into every system prompt.
pub const RESPONSE_FORMAT: &str = "response-format";
/// Instructions appended when plan mode is active.
pub const PLAN_MODE: &str = "plan-mode";
/// Template for executing an approved plan.  Vars: `{summary}`, `{items}`.
pub const PLAN_EXECUTE: &str = "plan-execute";
/// First user message that bootstraps the agent loop.
/// Vars: `{ws_root}`, `{platform}`, `{role}`, `{task}`.
pub const TASK_BOOTSTRAP: &str = "task-bootstrap";
/// Nudge when model returns invalid JSON.  Vars: `{raw}`.
pub const NUDGE_INVALID_JSON: &str = "nudge-invalid-json";
/// Nudge when model repeats the same response.
pub const NUDGE_REPETITION: &str = "nudge-repetition";
/// Nudge when model emits update_plan without tool calls.
pub const NUDGE_PLAN_ONLY: &str = "nudge-plan-only";
/// Nudge when model calls the same tool repeatedly.  Vars: `{tool}`.
pub const NUDGE_REDUNDANT_TOOL: &str = "nudge-redundant-tool";

// ---------------------------------------------------------------------------
// Embedded defaults (compile-time)
// ---------------------------------------------------------------------------

const DEFAULTS: &[(&str, &str)] = &[
    (RESPONSE_FORMAT, include_str!("../prompts/response-format.md")),
    (PLAN_MODE, include_str!("../prompts/plan-mode.md")),
    (PLAN_EXECUTE, include_str!("../prompts/plan-execute.md")),
    (TASK_BOOTSTRAP, include_str!("../prompts/task-bootstrap.md")),
    (NUDGE_INVALID_JSON, include_str!("../prompts/nudge-invalid-json.md")),
    (NUDGE_REPETITION, include_str!("../prompts/nudge-repetition.md")),
    (NUDGE_PLAN_ONLY, include_str!("../prompts/nudge-plan-only.md")),
    (NUDGE_REDUNDANT_TOOL, include_str!("../prompts/nudge-redundant-tool.md")),
];

// ---------------------------------------------------------------------------
// PromptStore
// ---------------------------------------------------------------------------

/// Runtime prompt template store.
///
/// On construction it loads embedded defaults, then overlays any `.md` files
/// found in `override_dir` (typically `~/.linggen/prompts/`).  This lets users
/// customise prompts without recompiling.
pub struct PromptStore {
    prompts: HashMap<String, String>,
}

impl PromptStore {
    /// Create a store seeded with compiled-in defaults, optionally overlaid
    /// with files from `override_dir`.
    pub fn load(override_dir: Option<&Path>) -> Self {
        let mut prompts: HashMap<String, String> = DEFAULTS
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        if let Some(dir) = override_dir {
            Self::overlay_from_dir(&mut prompts, dir);
        }

        Self { prompts }
    }

    /// Get a raw template by key.  Returns `None` for unknown keys.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.prompts.get(key).map(|s| s.as_str())
    }

    /// Render a template, replacing every `{name}` with the corresponding
    /// value from `vars`.  Unknown keys in the template are left as-is.
    pub fn render(&self, key: &str, vars: &[(&str, &str)]) -> Option<String> {
        self.get(key).map(|tpl| Self::substitute(tpl, vars))
    }

    /// Substitute `{name}` placeholders in `tpl`.
    pub fn substitute(tpl: &str, vars: &[(&str, &str)]) -> String {
        let mut out = tpl.to_string();
        for (name, value) in vars {
            out = out.replace(&format!("{{{}}}", name), value);
        }
        out
    }

    /// Default prompts dir: `~/.linggen/prompts/`.
    pub fn default_override_dir() -> PathBuf {
        crate::paths::linggen_home().join("prompts")
    }

    // -- private -------------------------------------------------------------

    /// Read every `.md` file in `dir` and insert/overwrite matching keys.
    fn overlay_from_dir(prompts: &mut HashMap<String, String>, dir: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return, // dir doesn't exist — that's fine
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Ok(content) = std::fs::read_to_string(&path) {
                prompts.insert(stem.to_string(), content);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_load() {
        let store = PromptStore::load(None);
        assert!(store.get(RESPONSE_FORMAT).is_some());
        assert!(store.get(PLAN_MODE).is_some());
        assert!(store.get(TASK_BOOTSTRAP).is_some());
        assert!(store.get(NUDGE_INVALID_JSON).is_some());
    }

    #[test]
    fn render_substitutes_vars() {
        let store = PromptStore::load(None);
        let rendered = store
            .render(NUDGE_REDUNDANT_TOOL, &[("tool", "Read")])
            .unwrap();
        assert!(rendered.contains("'Read'"));
        assert!(!rendered.contains("{tool}"));
    }

    #[test]
    fn render_preserves_unknown_vars() {
        let out = PromptStore::substitute("hello {name}, {unknown} world", &[("name", "alice")]);
        assert_eq!(out, "hello alice, {unknown} world");
    }

    #[test]
    fn overlay_from_tempdir() {
        let tmp = std::env::temp_dir().join("linggen_prompt_test");
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("nudge-plan-only.md"), "custom nudge").unwrap();

        let store = PromptStore::load(Some(&tmp));
        assert_eq!(store.get(NUDGE_PLAN_ONLY).unwrap(), "custom nudge");

        // cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
