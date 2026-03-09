use crate::project_store::missions::{self, Mission, MissionRunEntry, MISSION_AGENT_ID};
use crate::server::{ServerEvent, ServerState};
use chrono::Local;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{debug, info, warn};

/// How often the scheduler checks missions (seconds).
const CHECK_INTERVAL_SECS: u64 = 10;

/// Maximum triggers per mission per day to prevent runaway cost.
const MAX_TRIGGERS_PER_DAY: u32 = 100;

/// Per-mission tracking state.
struct MissionState {
    /// Last minute we fired this mission (to dedup within the same minute).
    last_fire_minute: Option<i64>,
    /// Daily trigger count + the date it applies to.
    daily_count: u32,
    daily_date: Option<chrono::NaiveDate>,
}

impl MissionState {
    fn new() -> Self {
        Self {
            last_fire_minute: None,
            daily_count: 0,
            daily_date: None,
        }
    }

    /// Reset daily count if the date has changed.
    fn maybe_reset_daily(&mut self, today: chrono::NaiveDate) {
        if self.daily_date != Some(today) {
            self.daily_count = 0;
            self.daily_date = Some(today);
        }
    }
}

/// Background loop that evaluates cron missions and triggers agent runs.
pub async fn mission_scheduler_loop(state: Arc<ServerState>) {
    let mut interval = time::interval(Duration::from_secs(CHECK_INTERVAL_SECS));
    let mut mission_states: HashMap<String, MissionState> = HashMap::new();
    let mut migrated = false;

    loop {
        interval.tick().await;

        let now = Local::now();
        let today = now.date_naive();
        let current_minute = now.timestamp() / 60;

        // One-time migrations
        if !migrated {
            let _ = state.manager.missions.migrate_flat_to_dirs();
            let _ = state.manager.missions.migrate_from_project_store(&state.manager.store);
            migrated = true;
        }

        let enabled_missions = match state.manager.missions.list_enabled_missions() {
            Ok(m) => m,
            Err(e) => {
                debug!("Mission scheduler: failed to list missions: {}", e);
                continue;
            }
        };

        for mission in &enabled_missions {
            let mission_key = &mission.id;

            let ms = mission_states
                .entry(mission_key.clone())
                .or_insert_with(MissionState::new);
            ms.maybe_reset_daily(today);

            // Check daily trigger cap
            if ms.daily_count >= MAX_TRIGGERS_PER_DAY {
                debug!(
                    "Mission scheduler: mission '{}' hit daily cap ({}), skipping",
                    mission.id, MAX_TRIGGERS_PER_DAY
                );
                continue;
            }

            // Check dedup: don't fire twice in the same minute
            if ms.last_fire_minute == Some(current_minute) {
                continue;
            }

            // Check if cron matches current time
            if !cron_matches_now(&mission.schedule, &now) {
                continue;
            }

            // Determine project root for this mission
            let project_path = mission
                .project
                .clone()
                .unwrap_or_else(|| {
                    std::env::current_dir()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                });
            let root = std::path::PathBuf::from(&project_path);

            // Check if mission agent is busy
            let agent = match state
                .manager
                .get_or_create_agent(&root, MISSION_AGENT_ID)
                .await
            {
                Ok(a) => a,
                Err(_) => continue,
            };

            if agent.try_lock().is_err() {
                debug!(
                    "Mission scheduler: mission agent is busy, skipping mission '{}'",
                    mission.id
                );
                let entry = MissionRunEntry {
                    run_id: String::new(),
                    session_id: None,
                    triggered_at: crate::util::now_ts_secs(),
                    status: "skipped".to_string(),
                    skipped: true,
                };
                let _ = state
                    .manager
                    .missions
                    .append_mission_run(&mission.id, &entry);
                continue;
            }

            // Fire!
            ms.last_fire_minute = Some(current_minute);
            ms.daily_count += 1;

            info!(
                "Mission scheduler: triggering mission '{}' (project: {:?})",
                mission.id, mission.project
            );

            let _ = state.events_tx.send(ServerEvent::MissionTriggered {
                mission_id: mission.id.clone(),
                agent_id: MISSION_AGENT_ID.to_string(),
                project_root: project_path.clone(),
            });

            state
                .manager
                .update_agent_activity(&project_path, MISSION_AGENT_ID)
                .await;

            let state_clone = state.clone();
            let mission_owned = mission.clone();
            let project_path_owned = project_path.clone();
            let root_owned = root.clone();

            tokio::spawn(async move {
                dispatch_mission_prompt(
                    state_clone,
                    root_owned,
                    &project_path_owned,
                    &mission_owned,
                )
                .await;
            });
        }

        // Clean up state for missions that no longer exist
        let active_keys: std::collections::HashSet<String> = enabled_missions
            .iter()
            .map(|m| m.id.clone())
            .collect();
        mission_states.retain(|k, _| active_keys.contains(k));
    }
}

/// Check if a cron expression matches the current time (within the current minute).
fn cron_matches_now(schedule: &str, now: &chrono::DateTime<Local>) -> bool {
    let cron_schedule = match missions::parse_cron(schedule) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let one_min_ago = *now - chrono::Duration::seconds(60);
    if let Some(next) = cron_schedule.after(&one_min_ago).next() {
        let next_minute = next.timestamp() / 60;
        let now_minute = now.timestamp() / 60;
        next_minute == now_minute
    } else {
        false
    }
}

/// Create a session title from the mission prompt.
fn mission_session_title(mission: &Mission) -> String {
    let prompt_preview: String = mission.prompt.chars().take(50).collect();
    let suffix = if mission.prompt.chars().count() > 50 {
        "..."
    } else {
        ""
    };
    let time = Local::now().format("%Y-%m-%d %H:%M");
    format!("Mission: {}{} — {}", prompt_preview, suffix, time)
}

/// Create a new session for a mission run.
async fn create_mission_session(
    state: &Arc<ServerState>,
    project_path: &str,
    mission: &Mission,
) -> Option<String> {
    let session_id = format!(
        "sess-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let root = match std::path::PathBuf::from(project_path).canonicalize() {
        Ok(r) => r,
        Err(_) => std::path::PathBuf::from(project_path),
    };
    match state.manager.get_or_create_project(root).await {
        Ok(ctx) => {
            let meta = crate::state_fs::sessions::SessionMeta {
                id: session_id.clone(),
                title: mission_session_title(mission),
                created_at: crate::util::now_ts_secs(),
            };
            match ctx.sessions.add_session(&meta) {
                Ok(_) => Some(session_id),
                Err(e) => {
                    warn!("Mission scheduler: failed to create session: {}", e);
                    None
                }
            }
        }
        Err(e) => {
            warn!("Mission scheduler: failed to get project for session: {}", e);
            None
        }
    }
}

/// Public wrapper for triggering a mission manually (from API).
pub async fn dispatch_mission_prompt_public(
    state: Arc<ServerState>,
    root: std::path::PathBuf,
    project_path: &str,
    mission: &Mission,
) {
    dispatch_mission_prompt(state, root, project_path, mission).await;
}

/// Dispatch a mission prompt to the mission agent.
async fn dispatch_mission_prompt(
    state: Arc<ServerState>,
    root: std::path::PathBuf,
    project_path: &str,
    mission: &Mission,
) {
    use crate::server::AgentStatusKind;

    let agent_id = MISSION_AGENT_ID;

    let agent = match state.manager.get_or_create_agent(&root, agent_id).await {
        Ok(a) => a,
        Err(e) => {
            warn!(
                "Mission scheduler: failed to get mission agent: {}",
                e
            );
            record_mission_run(&state, mission, "", None, "failed", false);
            return;
        }
    };

    let Ok(mut engine) = agent.try_lock() else {
        debug!("Mission scheduler: mission agent became busy before dispatch");
        record_mission_run(&state, mission, "", None, "skipped", true);
        return;
    };

    // Create a session for this run
    let session_id = create_mission_session(&state, project_path, mission).await;

    let manager = state.manager.clone();
    let events_tx = state.events_tx.clone();

    // Begin a run record
    let run_id = match manager
        .begin_agent_run(
            &root,
            session_id.as_deref(),
            agent_id,
            None,
            Some(format!("mission:{}", mission.id)),
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            warn!(
                "Mission scheduler: failed to begin run: {}",
                e
            );
            return;
        }
    };

    // Construct the mission message
    let message = format!(
        "[Mission: {}]\n\n{}",
        mission.id, mission.prompt
    );

    // Persist and emit the mission prompt as a system message
    crate::server::chat_helpers::persist_and_emit_message(
        &manager,
        &events_tx,
        &root,
        agent_id,
        "system",
        agent_id,
        &message,
        session_id.as_deref(),
        false,
    )
    .await;

    state
        .send_agent_status(
            agent_id.to_string(),
            AgentStatusKind::Working,
            Some("Processing mission".to_string()),
            None,
        )
        .await;

    engine.observations.clear();
    engine.task = Some(message.clone());
    engine.set_parent_agent(None);
    engine.set_run_id(Some(run_id.clone()));
    let result = engine.run_agent_loop(session_id.as_deref()).await;
    engine.set_run_id(None);

    let status = match result {
        Ok(outcome) => {
            let _ = manager
                .finish_agent_run(
                    &run_id,
                    crate::project_store::AgentRunStatus::Completed,
                    None,
                )
                .await;
            let _ = events_tx.send(ServerEvent::Outcome {
                agent_id: agent_id.to_string(),
                outcome,
            });
            "completed"
        }
        Err(err) => {
            let msg = err.to_string();
            let run_status = if msg.to_lowercase().contains("cancel") {
                crate::project_store::AgentRunStatus::Cancelled
            } else {
                crate::project_store::AgentRunStatus::Failed
            };
            let _ = manager
                .finish_agent_run(&run_id, run_status, Some(msg))
                .await;
            "failed"
        }
    };

    state
        .send_agent_status(
            agent_id.to_string(),
            AgentStatusKind::Idle,
            Some("Idle".to_string()),
            None,
        )
        .await;

    manager
        .update_agent_activity(project_path, agent_id)
        .await;

    // Record mission run
    record_mission_run(&state, mission, &run_id, session_id.as_deref(), status, false);
}

fn record_mission_run(
    state: &Arc<ServerState>,
    mission: &Mission,
    run_id: &str,
    session_id: Option<&str>,
    status: &str,
    skipped: bool,
) {
    let entry = MissionRunEntry {
        run_id: run_id.to_string(),
        session_id: session_id.map(|s| s.to_string()),
        triggered_at: crate::util::now_ts_secs(),
        status: status.to_string(),
        skipped,
    };
    let _ = state
        .manager
        .missions
        .append_mission_run(&mission.id, &entry);
}
