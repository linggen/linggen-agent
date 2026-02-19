use crate::agent_manager::AgentManager;
use crate::server::chat_helpers::{
    emit_outcome_event, emit_queue_updated, persist_and_emit_message,
    persist_message_only, queue_key, queue_preview,
};
use crate::server::{AgentStatusKind, QueuedChatItem, ServerEvent, ServerState};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::broadcast;

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
                    // 3. Structured agent loop (always)
                    run_structured_loop(&ctx, &mut engine).await;
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
    use super::parse_explicit_target_prefix;

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
}
