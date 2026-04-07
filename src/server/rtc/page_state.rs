//! Server-pushed page state — aggregates all UI-relevant data into one message.
//!
//! Instead of the frontend making 10+ individual HTTP requests on every state
//! change, the server pushes a single `PageState` message over the WebRTC
//! control data channel at 0.5 Hz (every 2 seconds), only when state changed.

use crate::server::ServerState;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

/// The frontend's current view context — what session/project is active.
/// Sent by the frontend via `set_view_context` control channel message.
#[derive(Debug, Clone, Default)]
pub struct ViewContext {
    pub session_id: Option<String>,
    pub project_root: Option<String>,
    pub is_compact: bool,
}

/// Aggregated page state pushed to the frontend.
/// All fields are optional — the server omits fields that haven't changed
/// or aren't applicable to the current view context.
#[derive(Debug, Clone, Serialize)]
pub struct PageState {
    // -- Global (always included unless compact mode) --

    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_sessions: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_models: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub missions: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_ask_user: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_counts_by_project: Option<std::collections::HashMap<String, usize>>,

    // -- Scoped (based on ViewContext) --

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_runs: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_permission: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<serde_json::Value>>,
}

/// Dirty-flag bits for tracking which data categories changed.
pub const DIRTY_GLOBAL: u64 = 0b0000_0001;
pub const DIRTY_SCOPED: u64 = 0b0000_0010;
pub const DIRTY_ALL: u64 = DIRTY_GLOBAL | DIRTY_SCOPED;

/// Build the page state from server state + view context.
/// Each sub-query is individually fallible — failures produce empty/None fields.
pub async fn build_page_state(
    state: &Arc<ServerState>,
    ctx: &ViewContext,
    dirty: u64,
) -> PageState {
    let include_global = (dirty & DIRTY_GLOBAL) != 0;
    let include_scoped = (dirty & DIRTY_SCOPED) != 0;

    let mut ps = PageState {
        projects: None,
        all_sessions: None,
        models: None,
        default_models: None,
        skills: None,
        missions: None,
        pending_ask_user: None,
        session_counts_by_project: None,
        agents: None,
        agent_runs: None,
        sessions: None,
        session_permission: None,
        files: None,
    };

    // -- Global data (skip in compact/skill mode) --
    if include_global && !ctx.is_compact {
        // Projects
        if let Ok(projects) = state.manager.store.list_projects() {
            // Compute session counts per project from the unified session list
            let all_sessions = state.manager.global_sessions.list_sessions().unwrap_or_default();
            let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for s in &all_sessions {
                if let Some(ref p) = s.project {
                    *counts.entry(p.clone()).or_default() += 1;
                } else if let Some(ref cwd) = s.cwd {
                    *counts.entry(cwd.clone()).or_default() += 1;
                }
            }
            ps.session_counts_by_project = Some(counts);
            ps.projects = Some(
                projects
                    .into_iter()
                    .filter_map(|p| serde_json::to_value(p).ok())
                    .collect(),
            );
            ps.all_sessions = Some(
                all_sessions
                    .into_iter()
                    .filter_map(|s| serde_json::to_value(s).ok())
                    .collect(),
            );
        }

        // Missions
        if let Ok(missions) = state.manager.missions.list_all_missions() {
            ps.missions = Some(
                missions
                    .into_iter()
                    .filter_map(|m| serde_json::to_value(m).ok())
                    .collect(),
            );
        }
    }

    // Models + config + skills + pending ask-user (always include — needed in compact mode too)
    if include_global {
        let models_guard = state.manager.models.read().await;
        let models: Vec<_> = models_guard.list_models().into_iter().cloned().collect();
        drop(models_guard);
        ps.models = Some(
            models
                .into_iter()
                .filter_map(|m| serde_json::to_value(m).ok())
                .collect(),
        );

        // Default models from config
        if let Ok((config, _)) = crate::config::Config::load_with_path() {
            ps.default_models = Some(config.routing.default_models.clone());
        }

        // Skills
        let skills = state.skill_manager.list_skills().await;
        ps.skills = Some(
            skills
                .into_iter()
                .filter_map(|s| serde_json::to_value(s).ok())
                .collect(),
        );

        // Pending ask-user
        let pending = state.pending_ask_user.lock().await;
        let items: Vec<serde_json::Value> = pending
            .iter()
            .map(|(qid, entry)| {
                serde_json::json!({
                    "question_id": qid,
                    "agent_id": entry.agent_id,
                    "questions": entry.questions,
                    "session_id": entry.session_id,
                })
            })
            .collect();
        ps.pending_ask_user = Some(items);
    }

    // -- Scoped data (based on active project/session) --
    if include_scoped {
        if let Some(ref root) = ctx.project_root {
            let root_buf = PathBuf::from(root);

            // Agents for project
            if let Ok(agents) = state.manager.list_agents(&root_buf).await {
                ps.agents = Some(
                    agents
                        .into_iter()
                        .filter_map(|a| serde_json::to_value(a).ok())
                        .collect(),
                );
            }

            // Sessions for project (filter from unified list, limit 50)
            if let Ok(all) = state.manager.global_sessions.list_sessions() {
                let project_sessions: Vec<_> = all
                    .into_iter()
                    .filter(|s| {
                        s.project.as_deref() == Some(root.as_str())
                            || s.cwd.as_deref() == Some(root.as_str())
                    })
                    .take(50)
                    .filter_map(|s| serde_json::to_value(s).ok())
                    .collect();
                ps.sessions = Some(project_sessions);
            }
        }

        if let Some(ref session_id) = ctx.session_id {
            // Agent runs for session
            let root_buf = PathBuf::from(ctx.project_root.as_deref().unwrap_or(""));
            if let Ok(runs) = state
                .manager
                .list_agent_runs(&root_buf, Some(session_id.as_str()))
                .await
            {
                ps.agent_runs = Some(
                    runs.into_iter()
                        .filter_map(|r| serde_json::to_value(r).ok())
                        .collect(),
                );
            }

            // Session permission
            let session_dir =
                crate::paths::global_sessions_dir().join(session_id);
            let perms =
                crate::engine::permission::SessionPermissions::load(&session_dir);
            let mut perm_val = serde_json::to_value(&perms).unwrap_or_default();

            // Compute effective mode for the project root (cwd)
            if let Some(ref root) = ctx.project_root {
                if let Some(mode) = crate::engine::permission::effective_mode_for_path(
                    &perms.path_modes,
                    std::path::Path::new(root),
                ) {
                    perm_val.as_object_mut().map(|m| {
                        m.insert(
                            "effective_mode".to_string(),
                            serde_json::Value::String(mode.to_string()),
                        )
                    });
                }
                let zone = crate::engine::permission::path_zone(std::path::Path::new(root));
                let zone_str = match zone {
                    crate::engine::permission::PathZone::Home => "home",
                    crate::engine::permission::PathZone::Temp => "temp",
                    crate::engine::permission::PathZone::System => "system",
                };
                perm_val.as_object_mut().map(|m| {
                    m.insert(
                        "zone".to_string(),
                        serde_json::Value::String(zone_str.to_string()),
                    )
                });
            }
            ps.session_permission = Some(perm_val);
        }
    }

    ps
}
