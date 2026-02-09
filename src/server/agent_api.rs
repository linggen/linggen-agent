use crate::server::{ServerEvent, ServerState};
use crate::state_fs::StateFile;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Deserialize)]
pub(crate) struct TaskRequest {
    project_root: String,
    agent_id: String,
    task: String,
}

pub(crate) async fn set_task(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<TaskRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    match state
        .manager
        .get_or_create_agent(&root, &req.agent_id)
        .await
    {
        Ok(agent) => {
            let mut engine = agent.lock().await;
            engine.set_task(req.task.clone());

            // Persist Lead task if it's Lead
            if req.agent_id == "lead" {
                if let Ok(ctx) = state.manager.get_or_create_project(root).await {
                    let lead_task = StateFile::PmTask {
                        id: format!(
                            "lead-{}",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs()
                        ),
                        status: "active".to_string(),
                        assigned_tasks: Vec::new(),
                    };
                    let _ = ctx.state_fs.write_file("active.md", &lead_task, &req.task);
                    let _ = state.events_tx.send(ServerEvent::StateUpdated);
                }
            }

            StatusCode::OK
        }
        Err(_) => StatusCode::NOT_FOUND,
    }
}

#[derive(Deserialize)]
pub(crate) struct RunRequest {
    project_root: String,
    agent_id: String,
    session_id: Option<String>,
}

pub(crate) async fn run_agent(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let agent_id = req.agent_id.clone();
    let session_id = req.session_id.clone();
    let events_tx = state.events_tx.clone();
    let manager = state.manager.clone();

    match state
        .manager
        .get_or_create_agent(&root, &req.agent_id)
        .await
    {
        Ok(agent) => {
            tokio::spawn(async move {
                let _ = events_tx.send(ServerEvent::AgentStatus {
                    agent_id: agent_id.clone(),
                    status: "working".to_string(),
                    detail: Some("Running".to_string()),
                });
                let mut engine = agent.lock().await;
                let outcome = engine
                    .run_agent_loop(session_id.as_deref())
                    .await
                    .unwrap_or(crate::engine::AgentOutcome::None);

                // If Lead finalized a task, save it
                if agent_id == "lead" {
                    if let crate::engine::AgentOutcome::Task(packet) = &outcome {
                        if let Ok(ctx) = manager.get_or_create_project(root).await {
                            let task_id = format!(
                                "task-{}",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs()
                            );
                            let coder_task = StateFile::CoderTask {
                                id: task_id.clone(),
                                status: "queued".to_string(),
                                story_id: None,
                                assigned_to: "coder".to_string(),
                            };
                            let body = format!(
                                "## {}\n\n### User Stories\n{}\n\n### Acceptance Criteria\n{}",
                                packet.title,
                                packet.user_stories.join("\n"),
                                packet.acceptance_criteria.join("\n")
                            );
                            let _ = ctx.state_fs.write_file(
                                &format!("tasks/{}.md", task_id),
                                &coder_task,
                                &body,
                            );
                            let _ = events_tx.send(ServerEvent::StateUpdated);
                        }
                    }
                }

                let _ = events_tx.send(ServerEvent::Outcome {
                    agent_id: agent_id.clone(),
                    outcome,
                });
                let _ = events_tx.send(ServerEvent::AgentStatus {
                    agent_id: agent_id.clone(),
                    status: "idle".to_string(),
                    detail: Some("Idle".to_string()),
                });
            });

            Json(serde_json::json!({ "status": "started" })).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
