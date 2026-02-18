use crate::agent_manager::AgentManager;
use crate::config::AgentPolicyCapability;
use crate::engine::PromptMode;
use crate::engine::sanitize_tool_args_for_display;
use crate::server::chat_helpers::{
    emit_outcome_event, emit_queue_updated, extract_tool_path_arg, persist_and_emit_message,
    persist_message_only, queue_key, queue_preview, tool_status_line, ToolStatusPhase,
};
use crate::server::{AgentStatusKind, QueuedChatItem, ServerEvent, ServerState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;

#[derive(Deserialize)]
pub(crate) struct ChatRequest {
    project_root: String,
    agent_id: String,
    message: String,
    session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ClearChatRequest {
    project_root: String,
    session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SettingsQuery {
    project_root: String,
}

#[derive(Deserialize)]
pub(crate) struct UpdateSettingsRequest {
    project_root: String,
    mode: String,
}

#[derive(Serialize)]
struct SettingsResponse {
    mode: String,
}

fn prompt_mode_from_project_mode(mode: crate::db::ProjectMode) -> PromptMode {
    match mode {
        crate::db::ProjectMode::Chat => PromptMode::Chat,
        crate::db::ProjectMode::Auto => PromptMode::Structured,
    }
}

fn prompt_mode_from_string(mode: &str) -> PromptMode {
    if mode.eq_ignore_ascii_case("chat") {
        PromptMode::Chat
    } else {
        PromptMode::Structured
    }
}

fn project_mode_from_string(mode: &str) -> crate::db::ProjectMode {
    if mode.eq_ignore_ascii_case("chat") {
        crate::db::ProjectMode::Chat
    } else {
        crate::db::ProjectMode::Auto
    }
}

fn mode_label(mode: PromptMode) -> &'static str {
    if mode == PromptMode::Chat {
        "chat"
    } else {
        "auto"
    }
}

fn parse_explicit_target_prefix(message: &str) -> Option<(&str, &str)> {
    let rest = message.strip_prefix('@')?;
    let space_idx = rest.find(' ')?;
    let candidate = rest[..space_idx].trim();
    let body = rest[space_idx + 1..].trim_start();
    if candidate.is_empty() {
        return None;
    }
    if !candidate
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some((candidate, body))
}

const CHAT_DUP_TOOL_STREAK_LIMIT: usize = 3;
const CHAT_NO_NEW_READ_STEP_LIMIT: usize = 12;
const CHANGE_REPORT_MAX_FILES: usize = 20;
const CHANGE_REPORT_MAX_DIFF_CHARS: usize = 12_000;

#[derive(Debug, Clone)]
struct GitChangeSnapshot {
    repo_root: PathBuf,
    dirty_paths: HashSet<String>,
}

fn normalize_path_separators(path: &str) -> String {
    path.trim().replace('\\', "/")
}

fn parse_git_status_line(line: &str) -> Option<(String, String)> {
    if line.len() < 3 {
        return None;
    }
    let status = line.get(0..2)?.trim().to_string();
    let raw_path = line.get(3..)?.trim();
    if raw_path.is_empty() {
        return None;
    }
    let candidate = if let Some((_from, to)) = raw_path.rsplit_once(" -> ") {
        to.trim()
    } else {
        raw_path
    };
    let unquoted = candidate.trim_matches('"');
    if unquoted.is_empty() {
        return None;
    }
    Some((normalize_path_separators(unquoted), status))
}

fn git_status_for_repo(repo_root: &Path) -> Option<(HashSet<String>, HashMap<String, String>)> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut paths = HashSet::new();
    let mut status_by_path = HashMap::new();
    for line in stdout.lines() {
        if let Some((path, status)) = parse_git_status_line(line) {
            paths.insert(path.clone());
            status_by_path.entry(path).or_insert(status);
        }
    }
    Some((paths, status_by_path))
}

fn capture_git_snapshot(project_root: &Path) -> Option<GitChangeSnapshot> {
    let repo_output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !repo_output.status.success() {
        return None;
    }
    let repo_root = PathBuf::from(String::from_utf8_lossy(&repo_output.stdout).trim());
    let (dirty_paths, _status_by_path) = git_status_for_repo(&repo_root)?;
    Some(GitChangeSnapshot {
        repo_root,
        dirty_paths,
    })
}

fn to_repo_relative_path(project_root: &Path, repo_root: &Path, raw_path: &str) -> Option<String> {
    let candidate = PathBuf::from(raw_path);
    let abs = if candidate.is_absolute() {
        candidate
    } else {
        project_root.join(candidate)
    };
    let resolved = abs.canonicalize().unwrap_or(abs);
    let rel = resolved.strip_prefix(repo_root).ok()?;
    Some(normalize_path_separators(&rel.to_string_lossy()))
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let head: String = input.chars().take(max_chars).collect();
    format!("{head}\n... (truncated)")
}

fn git_diff_for_path(repo_root: &Path, rel_path: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("diff")
        .arg("--")
        .arg(rel_path)
        .output()
        .ok()?;

    if output.status.success() {
        let diff = String::from_utf8_lossy(&output.stdout).to_string();
        if !diff.trim().is_empty() {
            return Some(diff);
        }
    }

    let abs_path = repo_root.join(rel_path);
    if !abs_path.exists() {
        return None;
    }

    let untracked_output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["diff", "--no-index", "--", "/dev/null"])
        .arg(abs_path)
        .output()
        .ok()?;
    let diff = String::from_utf8_lossy(&untracked_output.stdout).to_string();
    if diff.trim().is_empty() {
        None
    } else {
        Some(diff)
    }
}

fn summarize_status(status: Option<&str>, tool_hint: Option<&str>) -> String {
    if let Some(tool) = tool_hint {
        let tool_label = if tool.eq_ignore_ascii_case("Edit") {
            "Edited"
        } else if tool.eq_ignore_ascii_case("Write") {
            "Written"
        } else {
            "Updated"
        };
        return format!("{} via {}", tool_label, tool);
    }
    match status.unwrap_or("").trim() {
        "??" => "Added (untracked)".to_string(),
        code if code.contains('R') => "Renamed".to_string(),
        code if code.contains('A') => "Added".to_string(),
        code if code.contains('D') => "Deleted".to_string(),
        code if code.contains('M') => "Modified".to_string(),
        _ => "Updated".to_string(),
    }
}

async fn emit_change_report_if_any(
    manager: &Arc<crate::agent_manager::AgentManager>,
    events_tx: &tokio::sync::broadcast::Sender<ServerEvent>,
    root: &PathBuf,
    agent_id: &str,
    session_id: Option<&str>,
    baseline: Option<&GitChangeSnapshot>,
    explicit_mutations: &HashMap<String, String>,
) {
    let Some(snapshot) = baseline else {
        return;
    };
    let Some((current_dirty, current_status_by_path)) = git_status_for_repo(&snapshot.repo_root)
    else {
        return;
    };

    let mut changed_paths: HashSet<String> = current_dirty
        .difference(&snapshot.dirty_paths)
        .cloned()
        .collect();
    let mut tool_hint_by_path: HashMap<String, String> = HashMap::new();
    for (raw_path, tool_name) in explicit_mutations {
        if let Some(repo_rel) = to_repo_relative_path(root, &snapshot.repo_root, raw_path) {
            changed_paths.insert(repo_rel.clone());
            tool_hint_by_path.insert(repo_rel, tool_name.clone());
        }
    }
    if changed_paths.is_empty() {
        return;
    }

    let mut changed_paths_sorted = changed_paths.into_iter().collect::<Vec<_>>();
    changed_paths_sorted.sort();

    let omitted_count = changed_paths_sorted
        .len()
        .saturating_sub(CHANGE_REPORT_MAX_FILES);
    let report_paths = changed_paths_sorted
        .into_iter()
        .take(CHANGE_REPORT_MAX_FILES)
        .collect::<Vec<_>>();

    let files = report_paths
        .iter()
        .map(|path| {
            let summary = summarize_status(
                current_status_by_path.get(path).map(String::as_str),
                tool_hint_by_path.get(path).map(String::as_str),
            );
            let diff = git_diff_for_path(&snapshot.repo_root, path)
                .map(|v| truncate_chars(&v, CHANGE_REPORT_MAX_DIFF_CHARS))
                .unwrap_or_else(|| "(diff unavailable)".to_string());
            serde_json::json!({
                "path": path,
                "summary": summary,
                "diff": diff,
            })
        })
        .collect::<Vec<_>>();

    // Emit a typed ChangeReport SSE event (not a generic Message).
    let _ = events_tx.send(ServerEvent::ChangeReport {
        agent_id: agent_id.to_string(),
        files: files.clone(),
        truncated_count: omitted_count,
    });

    // Persist the change report to DB/state_fs as a serialised message.
    let payload = serde_json::json!({
        "type": "change_report",
        "files": files,
        "truncated_count": omitted_count,
        "review_hint": "Review these diffs in the UI and rollback any file you don't want.",
    })
    .to_string();
    persist_message_only(
        manager, root, agent_id, agent_id, "user", &payload, session_id, false,
    )
    .await;
}

async fn force_plaintext_summary(
    engine: &mut crate::engine::AgentEngine,
    events_tx: &broadcast::Sender<ServerEvent>,
    agent_id: &str,
    session_id: Option<&str>,
    original_user_request: &str,
    read_paths: &[String],
    reason: &str,
) -> String {
    let read_files = if read_paths.is_empty() {
        "(none)".to_string()
    } else {
        read_paths.join(", ")
    };
    let prompt = format!(
        "Stop using tools now.\n\
Reason: {reason}\n\
Original user request: {original_user_request}\n\
Files already read: {read_files}\n\n\
Provide a concise final plain-text response to the user based on gathered information. \
Do not output JSON and do not request more tool calls."
    );

    let mut summary = String::new();
    match engine
        .chat_stream(&prompt, session_id, crate::engine::PromptMode::Chat)
        .await
    {
        Ok(mut stream) => {
            while let Some(token_result) = stream.next().await {
                if let Ok(token) = token_result {
                    summary.push_str(&token);
                    let _ = events_tx.send(ServerEvent::Token {
                        agent_id: agent_id.to_string(),
                        token,
                        done: false,
                        thinking: false,
                    });
                }
            }
            let _ = events_tx.send(ServerEvent::Token {
                agent_id: agent_id.to_string(),
                token: String::new(),
                done: true,
                thinking: false,
            });
        }
        Err(err) => {
            tracing::warn!("Forced summary stream failed: {}", err);
        }
    }

    let trimmed = summary.trim().to_string();
    if trimmed.is_empty() || extract_chat_tool_call(&trimmed).is_some() {
        if read_paths.is_empty() {
            format!(
                "I stopped because the tool loop was repeating without progress ({reason}). \
Please narrow scope (for example, exact file names) and I can continue."
            )
        } else {
            format!(
                "I stopped because the tool loop was repeating without progress ({reason}). \
I already read: {}. Please tell me which file(s) to focus on next, or ask for a summary of specific files.",
                read_paths.join(", ")
            )
        }
    } else {
        trimmed
    }
}

fn extract_chat_tool_call(text: &str) -> Option<(String, serde_json::Value)> {
    if let Ok(action) = crate::engine::parse_first_action(text) {
        if let crate::engine::ModelAction::Tool { tool, args } = action {
            return Some((tool, args));
        }
    }

    None
}

fn extract_all_chat_tool_calls(text: &str) -> Vec<(String, serde_json::Value)> {
    match crate::engine::parse_all_actions(text) {
        Ok(actions) => actions
            .into_iter()
            .filter_map(|a| match a {
                crate::engine::ModelAction::Tool { tool, args } => Some((tool, args)),
                _ => None,
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn chat_mode_structured_output_error(
    text: &str,
    allow_patch: bool,
    allow_finalize: bool,
) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(action) = crate::engine::parse_first_action(trimmed) {
        return match action {
            crate::engine::ModelAction::Tool { .. } => None,
            crate::engine::ModelAction::Patch { .. } => {
                if !allow_patch {
                    Some(
                        "I couldn't continue because this agent's policy does not allow `patch`. Update frontmatter `policy` to include `Patch` if needed."
                            .to_string(),
                    )
                } else {
                    Some(
                        "I couldn't continue because chat mode expects plain text or a single tool call. Please try again."
                            .to_string(),
                    )
                }
            }
            crate::engine::ModelAction::FinalizeTask { .. } => {
                if !allow_finalize {
                    Some(
                        "I couldn't continue because this agent's policy does not allow `finalize_task`. Update frontmatter `policy` to include `Finalize` if needed."
                            .to_string(),
                    )
                } else {
                    Some(
                        "I couldn't continue because chat mode expects plain text or a single tool call. Please try again."
                            .to_string(),
                    )
                }
            }
            crate::engine::ModelAction::Done { message } => {
                Some(message.unwrap_or_else(|| "Task completed.".to_string()))
            }
        };
    }

    if trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if value.get("type").and_then(|v| v.as_str()).is_some() {
                return Some(
                    "I couldn't continue because chat mode expects plain text or a single tool call."
                        .to_string(),
                );
            }
        }
    }

    None
}

async fn run_loop_with_tracking(
    manager: &Arc<crate::agent_manager::AgentManager>,
    root: &PathBuf,
    engine: &mut crate::engine::AgentEngine,
    agent_id: &str,
    session_id: Option<&str>,
    detail: &str,
) -> Result<crate::engine::AgentOutcome, anyhow::Error> {
    let run_id = manager
        .begin_agent_run(root, session_id, agent_id, None, Some(detail.to_string()))
        .await
        .ok();

    engine.set_run_id(run_id.clone());
    let result = engine.run_agent_loop(session_id).await;
    engine.set_run_id(None);

    if let Some(run_id) = run_id {
        match &result {
            Ok(_) => {
                let _ = manager
                    .finish_agent_run(&run_id, crate::db::AgentRunStatus::Completed, None)
                    .await;
            }
            Err(err) => {
                let msg = err.to_string();
                let status = if msg.to_lowercase().contains("cancel") {
                    crate::db::AgentRunStatus::Cancelled
                } else {
                    crate::db::AgentRunStatus::Failed
                };
                let _ = manager.finish_agent_run(&run_id, status, Some(msg)).await;
            }
        }
    }

    result
}

pub(crate) async fn get_settings_api(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<SettingsQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&q.project_root)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&q.project_root));
    match state
        .manager
        .db
        .get_project_settings(&root.to_string_lossy())
    {
        Ok(settings) => Json(SettingsResponse {
            mode: settings.mode.to_string(),
        })
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub(crate) async fn update_settings_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<UpdateSettingsRequest>,
) -> impl IntoResponse {
    let mode = project_mode_from_string(&req.mode);
    let root = PathBuf::from(&req.project_root)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&req.project_root));
    let root_str = root.to_string_lossy().to_string();
    let _ = state.manager.get_or_create_project(root.clone()).await;
    if let Err(e) = state.manager.db.set_project_mode(&root_str, mode) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let _ = state
        .manager
        .set_project_prompt_mode(&root, prompt_mode_from_project_mode(mode))
        .await;
    let mode_str = mode.to_string();
    let _ = state.events_tx.send(ServerEvent::SettingsUpdated {
        project_root: root_str,
        mode: mode_str.clone(),
    });
    Json(SettingsResponse { mode: mode_str }).into_response()
}

pub(crate) async fn clear_chat_history_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ClearChatRequest>,
) -> impl IntoResponse {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    match state
        .manager
        .db
        .clear_chat_history(&req.project_root, &session_id)
    {
        Ok(removed) => {
            // Also clear in-memory chat history for all agents in this project/session
            if let Ok(root) = PathBuf::from(&req.project_root).canonicalize() {
                if let Ok(ctx) = state.manager.get_or_create_project(root).await {
                    let agents = ctx.agents.lock().await;
                    for agent_mutex in agents.values() {
                        let mut agent = agent_mutex.lock().await;
                        agent.chat_history.clear();
                        agent.observations.clear();
                    }
                }
            }

            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(serde_json::json!({ "removed": removed })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Shared context for the async chat dispatch functions.
struct ChatRunCtx {
    state: Arc<ServerState>,
    manager: Arc<AgentManager>,
    events_tx: broadcast::Sender<ServerEvent>,
    root: PathBuf,
    agent_id: String,
    session_id: Option<String>,
    clean_msg: String,
    git_baseline: Option<GitChangeSnapshot>,
}

/// Dispatch a skill (slash command) invocation.
async fn run_skill_dispatch(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    let parts: Vec<&str> = ctx.clean_msg.trim().splitn(2, ' ').collect();
    let cmd = parts[0].trim_start_matches('/');
    let user_args = parts
        .get(1)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Resolve the skill.
    if let Some(manager) = engine.tools.get_manager() {
        if let Some(skill) = manager.skill_manager.get_skill(cmd).await {
            if !skill.user_invocable {
                let err_msg = format!(
                    "Skill '{}' is not user-invocable and cannot be activated with /{cmd}.",
                    skill.name
                );
                persist_and_emit_message(
                    &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
                )
                .await;
                return;
            }
            // If no arguments given and the skill has a usage hint, respond immediately
            // with a subcommand list instead of spinning up the agent loop.
            if user_args.is_none() {
                if let Some(hint) = &skill.argument_hint {
                    let subcommands: Vec<String> = hint
                        .split('|')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(|s| format!("  `/{} {}`", skill.name, s))
                        .collect();
                    let usage_msg = format!(
                        "{}\n\n**Commands:**\n{}",
                        skill.description,
                        subcommands.join("\n"),
                    );
                    persist_and_emit_message(
                        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                        &ctx.agent_id, "user", &usage_msg, ctx.session_id.as_deref(), false,
                    )
                    .await;
                    return;
                }
            }
            engine.active_skill = Some(skill);
        }
    }

    let task_for_loop = user_args
        .unwrap_or_else(|| "Initialize this workspace and summarize status.".to_string());

    engine.observations.clear();
    engine.task = Some(task_for_loop);

    let skill_msg = format!("Running skill: {}", cmd);
    persist_and_emit_message(
        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
        &ctx.agent_id, "user", &skill_msg, ctx.session_id.as_deref(), false,
    )
    .await;

    let outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:skill",
    )
    .await;

    if let Err(e) = outcome {
        tracing::warn!("Skill loop failed: {}", e);
        let err_msg = format!("Error: {}", e);
        persist_and_emit_message(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
        )
        .await;
    } else {
        if let Ok(outcome) = &outcome {
            emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id);
        }
        let no_explicit_mutations: HashMap<String, String> = HashMap::new();
        emit_change_report_if_any(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            ctx.session_id.as_deref(), ctx.git_baseline.as_ref(), &no_explicit_mutations,
        )
        .await;
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
}

/// Dispatch a user-defined trigger activation.
/// Similar to `run_skill_dispatch` but takes a pre-resolved skill name and remaining input.
async fn run_trigger_dispatch(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
    skill_name: &str,
    remaining: &str,
) {
    let user_args = if remaining.is_empty() {
        None
    } else {
        Some(remaining.to_string())
    };

    let mut skill_default_task: Option<String> = None;
    if let Some(manager) = engine.tools.get_manager() {
        if let Some(skill) = manager.skill_manager.get_skill(skill_name).await {
            if !skill.user_invocable {
                let err_msg = format!(
                    "Skill '{}' is not user-invocable and cannot be activated via trigger.",
                    skill.name
                );
                persist_and_emit_message(
                    &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
                )
                .await;
                return;
            }
            if user_args.is_none() {
                skill_default_task =
                    Some(format!("Run the '{}' skill: {}", skill.name, skill.description));
            }
            engine.active_skill = Some(skill);
        }
    }

    let task_for_loop = user_args
        .or(skill_default_task)
        .unwrap_or_else(|| "Initialize this workspace and summarize status.".to_string());

    engine.observations.clear();
    engine.task = Some(task_for_loop);

    let skill_msg = format!("Running skill via trigger: {}", skill_name);
    persist_and_emit_message(
        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
        &ctx.agent_id, "user", &skill_msg, ctx.session_id.as_deref(), false,
    )
    .await;

    let outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:trigger",
    )
    .await;

    if let Err(e) = outcome {
        tracing::warn!("Trigger skill loop failed: {}", e);
        let err_msg = format!("Error: {}", e);
        persist_and_emit_message(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
        )
        .await;
    } else {
        if let Ok(outcome) = &outcome {
            emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id);
        }
        let no_explicit_mutations: HashMap<String, String> = HashMap::new();
        emit_change_report_if_any(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            ctx.session_id.as_deref(), ctx.git_baseline.as_ref(), &no_explicit_mutations,
        )
        .await;
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
}

/// Dispatch the structured (auto) mode agent loop.
async fn run_structured_loop(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    ctx.state
        .send_agent_status(ctx.agent_id.clone(), AgentStatusKind::Thinking, Some("Thinking".to_string()))
        .await;
    engine.observations.clear();
    let task_for_loop = ctx.clean_msg.trim().to_string();
    engine.task = Some(task_for_loop);

    // Wire up the thinking channel so streaming thinking tokens reach the UI.
    let (thinking_tx, mut thinking_rx) = tokio::sync::mpsc::unbounded_channel();
    engine.thinking_tx = Some(thinking_tx);

    let events_tx_clone = ctx.events_tx.clone();
    let agent_id_clone = ctx.agent_id.clone();
    tokio::spawn(async move {
        while let Some(event) = thinking_rx.recv().await {
            match event {
                crate::engine::ThinkingEvent::Token(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token {
                        agent_id: agent_id_clone.clone(),
                        token,
                        done: false,
                        thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::Done => {
                    let _ = events_tx_clone.send(ServerEvent::Token {
                        agent_id: agent_id_clone.clone(),
                        token: String::new(),
                        done: true,
                        thinking: true,
                    });
                }
            }
        }
    });

    let outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:structured-loop",
    )
    .await;

    // Drop the thinking sender so the forwarder task exits.
    engine.thinking_tx = None;

    if let Ok(outcome) = &outcome {
        emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id);
        let no_explicit_mutations: HashMap<String, String> = HashMap::new();
        emit_change_report_if_any(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            ctx.session_id.as_deref(), ctx.git_baseline.as_ref(), &no_explicit_mutations,
        )
        .await;
    } else if let Err(err) = outcome {
        let _ = ctx.events_tx.send(ServerEvent::Message {
            from: ctx.agent_id.clone(),
            to: "user".to_string(),
            content: format!("Error: {}", err),
        });
    }
    let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
}

/// Dispatch the chat-mode bounded tool loop.
async fn run_chat_tool_loop(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
    prompt_mode: PromptMode,
) {
    let mut full_response = String::new();

    match engine
        .chat_stream(&ctx.clean_msg, ctx.session_id.as_deref(), prompt_mode)
        .await
    {
        Ok(mut stream) => {
            ctx.state
                .send_agent_status(ctx.agent_id.clone(), AgentStatusKind::Thinking, Some("Thinking".to_string()))
                .await;
            while let Some(token_result) = stream.next().await {
                if let Ok(token) = token_result {
                    full_response.push_str(&token);
                    let _ = ctx.events_tx.send(ServerEvent::Token {
                        agent_id: ctx.agent_id.clone(),
                        token,
                        done: false,
                        thinking: false,
                    });
                }
            }
            let _ = ctx.events_tx.send(ServerEvent::Token {
                agent_id: ctx.agent_id.clone(),
                token: String::new(),
                done: true,
                thinking: false,
            });

            let (text_part, json_part) =
                crate::engine::model_message_log_parts(&full_response, 100, 100);
            let json_rendered = json_part
                .as_ref()
                .and_then(|v| serde_json::to_string(v).ok())
                .unwrap_or_else(|| "null".to_string());
            tracing::info!(
                "Chat model output split: text='{}' json={}",
                text_part.replace('\n', "\\n"),
                json_rendered
            );

            let chat_tool_max_iters = engine.cfg.max_iters;
            let allow_patch = engine.spec.as_ref()
                .map(|s| s.allows_policy(AgentPolicyCapability::Patch))
                .unwrap_or(false);
            let allow_finalize = engine.spec.as_ref()
                .map(|s| s.allows_policy(AgentPolicyCapability::Finalize))
                .unwrap_or(false);
            let mut pending_tools = extract_all_chat_tool_calls(&full_response);
            // If the model emitted tool calls, the streamed text was "thinking"
            // rather than a final answer. Signal this to the UI.
            if !pending_tools.is_empty() {
                let _ = ctx.events_tx.send(ServerEvent::Token {
                    agent_id: ctx.agent_id.clone(),
                    token: String::new(),
                    done: true,
                    thinking: true,
                });
            }
            let mut final_response = if !pending_tools.is_empty() {
                None
            } else if let Some(err_msg) = chat_mode_structured_output_error(
                &full_response, allow_patch, allow_finalize,
            ) {
                Some(err_msg)
            } else {
                Some(full_response.clone())
            };
            let mut tool_steps = 0usize;
            let mut last_tool_sig = String::new();
            let mut duplicate_tool_streak = 0usize;
            let mut read_paths_seen: HashSet<String> = HashSet::new();
            let mut read_paths_order: Vec<String> = Vec::new();
            let mut steps_since_new_read = 0usize;
            let mut explicit_mutations: HashMap<String, String> = HashMap::new();

            while final_response.is_none() {
                if pending_tools.is_empty() {
                    break;
                }
                // Take the current batch and execute sequentially.
                let batch = std::mem::take(&mut pending_tools);
                let mut batch_observations: Vec<String> = Vec::new();
                for (tool, args) in batch {
                    if tool_steps >= chat_tool_max_iters {
                        final_response = Some(format!(
                            "I stopped after {} tool steps to respect max_iters.",
                            chat_tool_max_iters
                        ));
                        break;
                    }
                    tool_steps += 1;
                    steps_since_new_read = steps_since_new_read.saturating_add(1);

                let call_sig = crate::engine::tool_call_signature(&tool, &args);
                if call_sig == last_tool_sig {
                    duplicate_tool_streak = duplicate_tool_streak.saturating_add(1);
                } else {
                    duplicate_tool_streak = 0;
                    last_tool_sig = call_sig;
                }
                if duplicate_tool_streak >= CHAT_DUP_TOOL_STREAK_LIMIT {
                    tracing::warn!(
                        "Chat tool loop breaker: duplicate call streak={} tool={}",
                        duplicate_tool_streak + 1,
                        tool
                    );
                    let reply = force_plaintext_summary(
                        engine, &ctx.events_tx, &ctx.agent_id,
                        ctx.session_id.as_deref(), &ctx.clean_msg,
                        &read_paths_order, "same tool call repeated",
                    )
                    .await;
                    final_response = Some(reply);
                    break;
                }

                let tool_start_status = tool_status_line(&tool, Some(&args), ToolStatusPhase::Start);
                let tool_done_status = tool_status_line(&tool, Some(&args), ToolStatusPhase::Done);
                let tool_failed_status = tool_status_line(&tool, Some(&args), ToolStatusPhase::Failed);

                ctx.state
                    .send_agent_status(ctx.agent_id.clone(), AgentStatusKind::CallingTool, Some(tool_start_status))
                    .await;
                let safe_args = sanitize_tool_args_for_display(&tool, &args);
                let tool_msg = serde_json::json!({
                    "type": "tool",
                    "tool": tool.clone(),
                    "args": safe_args
                })
                .to_string();
                persist_and_emit_message(
                    &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &tool_msg, ctx.session_id.as_deref(), false,
                )
                .await;

                let mutate_path = if matches!(tool.as_str(), "Write" | "Edit") {
                    extract_tool_path_arg(&args)
                } else {
                    None
                };
                let read_path = if matches!(tool.as_str(), "Read") {
                    extract_tool_path_arg(&args)
                } else {
                    None
                };
                let call = crate::engine::tools::ToolCall {
                    tool: tool.clone(),
                    args,
                };

                let result = match engine.tools.execute(call) {
                    Ok(result) => result,
                    Err(e) => {
                        tracing::warn!("Tool execution failed ({}): {}", tool, e);
                        ctx.state
                            .send_agent_status(
                                ctx.agent_id.clone(), AgentStatusKind::CallingTool,
                                Some(tool_failed_status.clone()),
                            )
                            .await;
                        let rendered = format!("tool_error: tool={} error={}", tool, e);
                        engine.upsert_observation("error", &tool, rendered.clone());
                        let _ = engine
                            .manager_db_add_observation(&tool, &rendered, ctx.session_id.as_deref())
                            .await;
                        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
                        final_response = Some(format!("Tool execution failed ({}): {}", tool, e));
                        break;
                    }
                };

                let rendered_model = crate::engine::render_tool_result(&result);
                let rendered_public = crate::engine::render_tool_result_public(&result);
                if matches!(tool.as_str(), "Write" | "Edit")
                    && (rendered_public.contains("File written:")
                        || rendered_public.contains("Edited file:"))
                {
                    if let Some(path) = mutate_path.as_ref() {
                        explicit_mutations.insert(path.clone(), tool.clone());
                    }
                }
                engine.upsert_observation("tool", &tool, rendered_model.clone());
                let _ = engine
                    .manager_db_add_observation(&tool, &rendered_public, ctx.session_id.as_deref())
                    .await;
                let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
                ctx.state
                    .send_agent_status(ctx.agent_id.clone(), AgentStatusKind::CallingTool, Some(tool_done_status))
                    .await;
                ctx.state
                    .send_agent_status(ctx.agent_id.clone(), AgentStatusKind::Thinking, Some("Thinking".to_string()))
                    .await;

                let mut observation_for_prompt = rendered_model.clone();
                let mut observation_for_display = rendered_public.clone();

                if matches!(tool.as_str(), "Read") {
                    if let Some(path) = read_path {
                        let norm = path.trim().replace('\\', "/");
                        if !norm.is_empty() {
                            if read_paths_seen.insert(norm.clone()) {
                                read_paths_order.push(norm);
                                steps_since_new_read = 0;
                            }
                        }
                    }
                    if !read_paths_order.is_empty()
                        && steps_since_new_read >= CHAT_NO_NEW_READ_STEP_LIMIT
                    {
                        tracing::warn!(
                            "Chat tool loop breaker: no new read files for {} steps",
                            steps_since_new_read
                        );
                        let reply = force_plaintext_summary(
                            engine, &ctx.events_tx, &ctx.agent_id,
                            ctx.session_id.as_deref(), &ctx.clean_msg,
                            &read_paths_order, "no new files were read",
                        )
                        .await;
                        final_response = Some(reply);
                        break;
                    }
                }

                if matches!(tool.as_str(), "Write" | "Edit") {
                    if let Some(path) = mutate_path {
                        let readback = engine.tools.execute(crate::engine::tools::ToolCall {
                            tool: "Read".to_string(),
                            args: serde_json::json!({ "path": path, "max_bytes": 8000 }),
                        });
                        if let Ok(read_result) = readback {
                            let read_model = crate::engine::render_tool_result(&read_result);
                            let read_public = crate::engine::render_tool_result_public(&read_result);
                            engine.upsert_observation("tool", "Read", read_model.clone());
                            let _ = engine
                                .manager_db_add_observation("Read", &read_public, ctx.session_id.as_deref())
                                .await;
                            observation_for_prompt = format!(
                                "{}\n\nPost-write readback:\n{}",
                                observation_for_prompt, read_model
                            );
                            observation_for_display = format!(
                                "{}\n\nPost-write readback:\n{}",
                                observation_for_display, read_public
                            );
                            let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
                        }
                    }
                }

                    // Truncate individual observations to 4KB to avoid blowing context.
                    const OBS_MAX_CHARS: usize = 4096;
                    let truncated_obs = if observation_for_prompt.len() > OBS_MAX_CHARS {
                        format!("{}... (truncated)", &observation_for_prompt[..OBS_MAX_CHARS])
                    } else {
                        observation_for_prompt
                    };
                    batch_observations.push(format!("[{}] {}", tool, truncated_obs));
                } // end for (tool, args) in batch

                if final_response.is_some() {
                    continue;
                }

                // Keep only the last 3 observations if the batch was larger.
                let obs_for_prompt: Vec<&String> = if batch_observations.len() > 3 {
                    batch_observations.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect()
                } else {
                    batch_observations.iter().collect()
                };
                let observations_block = obs_for_prompt.iter()
                    .enumerate()
                    .map(|(i, obs)| format!("{}. {}", i + 1, obs))
                    .collect::<Vec<_>>()
                    .join("\n\n");

                // Persist batch observations into chat_history for context compression.
                for obs in &batch_observations {
                    engine.chat_history.push(crate::ollama::ChatMessage {
                        role: "user".to_string(),
                        content: obs.clone(),
                    });
                }

                let followup_prompt = format!(
                    "Here are the results from your tool calls:\n\n{}\n\nOriginal user request:\n{}\n\nBased on these results, either continue with more tool calls or provide your final answer in plain text.",
                    observations_block, ctx.clean_msg
                );
                let mut followup_response = String::new();
                match engine
                    .chat_stream(&followup_prompt, ctx.session_id.as_deref(), crate::engine::PromptMode::Chat)
                    .await
                {
                    Ok(mut followup_stream) => {
                        while let Some(token_result) = followup_stream.next().await {
                            match token_result {
                                Ok(token) => {
                                    followup_response.push_str(&token);
                                    let _ = ctx.events_tx.send(ServerEvent::Token {
                                        agent_id: ctx.agent_id.clone(),
                                        token,
                                        done: false,
                                        thinking: false,
                                    });
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        "Follow-up stream token error (step {}): {}",
                                        tool_steps, err
                                    );
                                    final_response = Some(format!(
                                        "I hit a model stream error while continuing the task: {}",
                                        err
                                    ));
                                    break;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            "Follow-up model stream failed (step {}): {}",
                            tool_steps, err
                        );
                        final_response = Some(format!(
                            "I couldn't continue due to model error: {}",
                            err
                        ));
                    }
                }
                let _ = ctx.events_tx.send(ServerEvent::Token {
                    agent_id: ctx.agent_id.clone(),
                    token: String::new(),
                    done: true,
                    thinking: false,
                });
                if final_response.is_some() {
                    continue;
                }

                let (followup_text_part, followup_json_part) =
                    crate::engine::model_message_log_parts(&followup_response, 100, 100);
                let followup_json_rendered = followup_json_part
                    .as_ref()
                    .and_then(|v| serde_json::to_string(v).ok())
                    .unwrap_or_else(|| "null".to_string());
                tracing::info!(
                    "Chat follow-up output split: text='{}' json={}",
                    followup_text_part.replace('\n', "\\n"),
                    followup_json_rendered
                );

                pending_tools = extract_all_chat_tool_calls(&followup_response);
                if pending_tools.is_empty() {
                    if let Some(err_msg) = chat_mode_structured_output_error(
                        &followup_response, allow_patch, allow_finalize,
                    ) {
                        final_response = Some(err_msg);
                    } else {
                        final_response = Some(followup_response);
                    }
                }
            }

            let mut reply = final_response
                .unwrap_or_else(|| "I couldn't produce a final answer from the tool loop.".to_string());
            if reply.trim().is_empty() {
                reply = "I couldn't produce a non-empty response. Please try again.".to_string();
            }

            let _ = engine
                .finalize_chat(&ctx.clean_msg, &reply, ctx.session_id.as_deref(), prompt_mode)
                .await;
            let _ = ctx.events_tx.send(ServerEvent::Message {
                from: ctx.agent_id.clone(),
                to: "user".to_string(),
                content: reply,
            });
            emit_change_report_if_any(
                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                ctx.session_id.as_deref(), ctx.git_baseline.as_ref(), &explicit_mutations,
            )
            .await;
            let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
        }
        Err(e) => {
            let error_msg = format!("Error: {}", e);
            persist_and_emit_message(
                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                &ctx.agent_id, "user", &error_msg, ctx.session_id.as_deref(), false,
            )
            .await;
        }
    }
}

pub(crate) async fn chat_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let project_root_str = root.to_string_lossy().to_string();
    let session_id = req.session_id.clone();
    let effective_session_id = session_id.clone().unwrap_or_else(|| "default".to_string());
    let events_tx = state.events_tx.clone();

    // Optional explicit target prefix: "@agent_id <message>".
    // Only reroute when the candidate agent exists in this project.
    let (target_id, clean_msg) =
        if let Some((candidate, body)) = parse_explicit_target_prefix(&req.message) {
            let candidate_id = candidate.to_string();
            if state
                .manager
                .agent_exists(&root, &candidate_id)
                .await
            {
                (candidate_id, body.to_string())
            } else {
                (req.agent_id.clone(), req.message.clone())
            }
        } else {
            (req.agent_id.clone(), req.message.clone())
        };
    let trimmed_msg = clean_msg.trim();

    match state.manager.get_or_create_agent(&root, &target_id).await {
        Ok(agent) => {
            let was_busy = agent.try_lock().is_err();
            let queued_item = if was_busy {
                Some(QueuedChatItem {
                    id: format!(
                        "{}-{}",
                        crate::util::now_ts_ms(),
                        state.queue_seq.fetch_add(1, Ordering::Relaxed)
                    ),
                    agent_id: target_id.clone(),
                    session_id: effective_session_id.clone(),
                    preview: queue_preview(&clean_msg),
                    timestamp: crate::util::now_ts_secs(),
                })
            } else {
                None
            };
            if let Some(item) = &queued_item {
                let key = queue_key(&project_root_str, &effective_session_id, &target_id);
                {
                    let mut guard = state.queued_chats.lock().await;
                    guard.entry(key).or_default().push(item.clone());
                }
                emit_queue_updated(&state, &project_root_str, &effective_session_id, &target_id)
                    .await;
            }

            let events_tx_clone = events_tx.clone();
            let target_id_clone = target_id.clone();
            let clean_msg_clone = clean_msg.clone();
            let root_clone = root.clone();
            let manager = state.manager.clone();
            let state_clone = state.clone();
            let queued_item_id = queued_item.as_ref().map(|q| q.id.clone());
            let session_id_for_queue = effective_session_id.clone();
            let project_root_for_queue = project_root_str.clone();

            // Handle mode switch commands before emitting a user message.
            if let Some(mode_value) = trimmed_msg.strip_prefix("/mode ") {
                // Emit and persist the user's /mode command so it appears in chat history.
                persist_and_emit_message(
                    &state.manager,
                    &events_tx,
                    &root,
                    &target_id,
                    "user",
                    &target_id,
                    &clean_msg,
                    session_id.as_deref(),
                    false,
                )
                .await;

                let mode_value = mode_value.trim().to_lowercase();
                let mut engine = agent.lock().await;
                let mode = prompt_mode_from_string(&mode_value);
                engine.set_prompt_mode(mode);
                let mode_label = mode_label(mode);
                let _ = state
                    .manager
                    .db
                    .set_project_mode(&root.to_string_lossy(), project_mode_from_string(mode_label));
                let _ = state.manager.set_project_prompt_mode(&root, mode).await;
                let _ = events_tx_clone.send(ServerEvent::SettingsUpdated {
                    project_root: root.to_string_lossy().to_string(),
                    mode: mode_label.to_string(),
                });
                let _ = events_tx_clone.send(ServerEvent::Message {
                    from: target_id_clone.clone(),
                    to: "user".to_string(),
                    content: format!("Mode set to {}", mode_label),
                });
                return Json(serde_json::json!({ "status": "mode_set" })).into_response();
            }

            if !was_busy {
                // Emit and persist user message immediately if the target agent is not busy.
                persist_and_emit_message(
                    &state.manager,
                    &events_tx,
                    &root,
                    &target_id,
                    "user",
                    &target_id,
                    &clean_msg,
                    session_id.as_deref(),
                    false,
                )
                .await;
            }

            tokio::spawn(async move {
                let mut engine = agent.lock().await;
                if let Some(queued_id) = queued_item_id.as_deref() {
                    let key = queue_key(
                        &project_root_for_queue,
                        &session_id_for_queue,
                        &target_id_clone,
                    );
                    {
                        let mut guard = state_clone.queued_chats.lock().await;
                        if let Some(items) = guard.get_mut(&key) {
                            items.retain(|item| item.id != queued_id);
                            if items.is_empty() {
                                guard.remove(&key);
                            }
                        }
                    }
                    emit_queue_updated(
                        &state_clone,
                        &project_root_for_queue,
                        &session_id_for_queue,
                        &target_id_clone,
                    )
                    .await;

                    persist_and_emit_message(
                        &manager, &events_tx_clone, &root_clone, &target_id_clone,
                        "user", &target_id_clone, &clean_msg_clone, session_id.as_deref(), false,
                    )
                    .await;
                }

                state_clone
                    .send_agent_status(
                        target_id_clone.clone(), AgentStatusKind::ModelLoading,
                        Some("Model loading".to_string()),
                    )
                    .await;
                if let Ok(settings) = manager.get_project_settings(&root_clone).await {
                    engine.set_prompt_mode(prompt_mode_from_project_mode(settings.mode));
                }
                let git_baseline = capture_git_snapshot(&root_clone);

                let ctx = ChatRunCtx {
                    state: state_clone.clone(),
                    manager: manager.clone(),
                    events_tx: events_tx_clone.clone(),
                    root: root_clone,
                    agent_id: target_id_clone.clone(),
                    session_id: session_id.clone(),
                    clean_msg: clean_msg_clone.clone(),
                    git_baseline,
                };

                if clean_msg_clone.trim_start().starts_with('/') {
                    // 1. Slash-command skill dispatch
                    run_skill_dispatch(&ctx, &mut engine).await;
                } else if let Some((skill_name, remaining)) =
                    manager.skill_manager.match_trigger(&clean_msg_clone).await
                {
                    // 2. User-defined trigger prefix match
                    run_trigger_dispatch(&ctx, &mut engine, &skill_name, &remaining).await;
                } else {
                    // 3. Normal chat / structured loop
                    let prompt_mode = engine.get_prompt_mode();
                    if prompt_mode == crate::engine::PromptMode::Structured {
                        run_structured_loop(&ctx, &mut engine).await;
                    } else {
                        run_chat_tool_loop(&ctx, &mut engine, prompt_mode).await;
                    }
                }

                state_clone
                    .send_agent_status(
                        target_id_clone.clone(), AgentStatusKind::Idle,
                        Some("Idle".to_string()),
                    )
                    .await;
            });

            let status = if was_busy { "queued" } else { "started" };
            Json(serde_json::json!({ "status": status })).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        chat_mode_structured_output_error, extract_chat_tool_call, parse_explicit_target_prefix,
    };

    #[test]
    fn extract_chat_tool_call_parses_supported_tool_json() {
        let input =
            r#"{"type":"tool","tool":"Grep","args":{"query":"logging.rs","globs":["src/**"]}}"#;
        let parsed = extract_chat_tool_call(input);
        assert!(parsed.is_some());
        let (tool, args) = parsed.unwrap();
        assert_eq!(tool, "Grep");
        assert_eq!(args["query"], "logging.rs");
        assert_eq!(args["globs"][0], "src/**");
    }

    #[test]
    fn extract_chat_tool_call_parses_edit_tool_json() {
        let input = r#"{"type":"tool","tool":"Edit","args":{"path":"src/logging.rs","old_string":"a","new_string":"b","replace_all":false}}"#;
        let parsed = extract_chat_tool_call(input);
        assert!(parsed.is_some());
        let (tool, args) = parsed.unwrap();
        assert_eq!(tool, "Edit");
        assert_eq!(args["path"], "src/logging.rs");
        assert_eq!(args["old_string"], "a");
        assert_eq!(args["new_string"], "b");
        assert_eq!(args["replace_all"], false);
    }

    #[test]
    fn chat_mode_structured_output_error_blocks_finalize_task() {
        let input = r#"{"type":"finalize_task","packet":{"title":"x","user_stories":[],"acceptance_criteria":[],"mermaid_wireframe":null}}"#;
        let err = chat_mode_structured_output_error(input, false, false);
        assert!(err.is_some());
    }

    #[test]
    fn chat_mode_structured_output_error_blocks_unknown_structured_json() {
        let input = r#"{"type":"unsupported_action","foo":"bar"}"#;
        let err = chat_mode_structured_output_error(input, false, false);
        assert!(err.is_some());
    }

    #[test]
    fn chat_mode_structured_output_error_allows_plain_text() {
        let err = chat_mode_structured_output_error(
            "I reviewed logging.rs and found two issues.",
            false,
            false,
        );
        assert!(err.is_none());
    }

    #[test]
    fn chat_mode_structured_output_error_finalize_allowed_still_requires_chat_shape() {
        let input = r#"{"type":"finalize_task","packet":{"title":"x","user_stories":[],"acceptance_criteria":[],"mermaid_wireframe":null}}"#;
        let err = chat_mode_structured_output_error(input, false, true);
        assert!(err.is_some());
        assert!(err
            .unwrap()
            .contains("chat mode expects plain text or a single tool call"));
    }

    #[test]
    fn parse_explicit_target_prefix_accepts_valid_mention() {
        let parsed = parse_explicit_target_prefix("@coder please review src/main.rs");
        assert_eq!(parsed, Some(("coder", "please review src/main.rs")));
    }

    #[test]
    fn parse_explicit_target_prefix_rejects_missing_body() {
        let parsed = parse_explicit_target_prefix("@coder");
        assert_eq!(parsed, None);
    }

    #[test]
    fn parse_explicit_target_prefix_rejects_invalid_agent_token() {
        let parsed = parse_explicit_target_prefix("@coder! please review");
        assert_eq!(parsed, None);
    }

    #[test]
    fn extract_chat_tool_call_does_not_infer_read_from_plain_text_with_filename() {
        let input = "All fixes have been applied to src/logging.rs and cargo check passed.";
        assert_eq!(extract_chat_tool_call(input), None);
    }

    #[test]
    fn extract_chat_tool_call_does_not_infer_bash_from_plain_text_command() {
        let input = "I'll run cargo check and report back.";
        assert_eq!(extract_chat_tool_call(input), None);
    }
}
