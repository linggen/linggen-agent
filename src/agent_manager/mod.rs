use crate::agent_manager::locks::LockManager;
use crate::agent_manager::models::ModelManager;
use crate::config::{AgentSpec, Config};
use crate::db::Db;
use crate::engine::{AgentEngine, AgentOutcome, AgentRole, EngineConfig};
use crate::skills::SkillManager;
use crate::state_fs::{StateFile, StateFs};
use anyhow::Result;
use globset::Glob;
use notify::{EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::warn;

pub mod locks;
pub mod models;

pub struct ProjectContext {
    pub root: PathBuf,
    pub agents: Mutex<HashMap<String, Arc<Mutex<AgentEngine>>>>,
    pub state_fs: StateFs,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

pub struct AgentManager {
    config: Config,
    pub projects: Mutex<HashMap<String, Arc<ProjectContext>>>,
    pub locks: Mutex<LockManager>,
    pub models: Arc<ModelManager>,
    pub db: Arc<Db>,
    pub skill_manager: Arc<SkillManager>,
    events: mpsc::UnboundedSender<AgentEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    TaskUpdate {
        agent_id: String,
        task: String,
    },
    Outcome {
        agent_id: String,
        outcome: AgentOutcome,
    },
    Message {
        from: String,
        to: String,
        content: String,
    },
    StateUpdated,
}

impl AgentManager {
    pub fn new(
        config: Config,
        db: Arc<Db>,
        skill_manager: Arc<SkillManager>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<AgentEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let models = Arc::new(ModelManager::new(config.models.clone()));
        (
            Arc::new(Self {
                config,
                projects: Mutex::new(HashMap::new()),
                locks: Mutex::new(LockManager::new()),
                models,
                db,
                skill_manager,
                events: tx,
            }),
            rx,
        )
    }

    pub async fn get_or_create_project(&self, root: PathBuf) -> Result<Arc<ProjectContext>> {
        let root = root
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("Invalid project path: {}", e))?;
        let mut projects = self.projects.lock().await;
        let key = root.to_string_lossy().to_string();
        if let Some(ctx) = projects.get(&key) {
            return Ok(ctx.clone());
        }

        let state_fs = StateFs::new(root.clone());
        let ctx = Arc::new(ProjectContext {
            root: root.clone(),
            agents: Mutex::new(HashMap::new()),
            state_fs,
            watcher: Mutex::new(None),
        });

        // Setup watcher
        let db_clone = self.db.clone();
        let root_clone = root.clone();
        let events_tx = self.events.clone();

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    let repo_path = root_clone.to_string_lossy().to_string();
                    
                    // Ignore internal and build directories
                    if event.paths.iter().any(|p| {
                        let s = p.to_string_lossy();
                        s.contains("/target/") || 
                        s.contains("/.git/") || 
                        s.contains("/.linggen-agent/") || 
                        s.contains("/node_modules/")
                    }) {
                        return;
                    }

                    match event.kind {
                        EventKind::Remove(_) => {
                            for path in event.paths {
                                if let Ok(rel) = path.strip_prefix(&root_clone) {
                                    let rel_str = rel.to_string_lossy();
                                    tracing::info!("File removed on disk: {}", rel_str);
                                    let _ = db_clone.remove_activity(&repo_path, &rel_str);
                                }
                            }
                            let _ = events_tx.send(AgentEvent::StateUpdated);
                        }
                        EventKind::Modify(notify::event::ModifyKind::Name(
                            notify::event::RenameMode::Both,
                        )) => {
                            if event.paths.len() == 2 {
                                let old = &event.paths[0];
                                let new = &event.paths[1];
                                if let (Ok(old_rel), Ok(new_rel)) =
                                    (old.strip_prefix(&root_clone), new.strip_prefix(&root_clone))
                                {
                                    let old_str = old_rel.to_string_lossy();
                                    let new_str = new_rel.to_string_lossy();
                                    tracing::info!("File renamed on disk: {} -> {}", old_str, new_str);
                                    let _ = db_clone.rename_activity(
                                        &repo_path,
                                        &old_str,
                                        &new_str,
                                    );
                                }
                            }
                            let _ = events_tx.send(AgentEvent::StateUpdated);
                        }
                        _ => {}
                    }
                }
            })?;

        watcher.watch(&root, RecursiveMode::Recursive)?;
        *ctx.watcher.lock().await = Some(watcher);

        projects.insert(key, ctx.clone());

        // Register in DB if not already there
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let _ = self
            .db
            .add_project(root.to_string_lossy().to_string(), name);

        Ok(ctx)
    }

    pub async fn get_or_create_agent(
        self: &Arc<Self>,
        project_root: &PathBuf,
        agent_id: &str,
    ) -> Result<Arc<Mutex<AgentEngine>>> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let ctx = self.get_or_create_project(project_root.clone()).await?;
        let mut agents = ctx.agents.lock().await;

        if let Some(agent) = agents.get(agent_id) {
            return Ok(agent.clone());
        }

        let agent_ref = self
            .config
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found in config", agent_id))?;

        let (spec, _system_prompt) =
            AgentSpec::from_markdown(&PathBuf::from(&agent_ref.spec_path))?;

        let model_id = agent_ref
            .model
            .as_ref()
            .or(spec.model.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                self.config
                    .models
                    .first()
                    .map(|m| m.id.clone())
                    .expect("No models configured")
            });

        let role = match spec.name.as_str() {
            "lead" => AgentRole::Lead,
            "coder" => AgentRole::Coder,
            _ => AgentRole::Operator,
        };

        let mut engine = AgentEngine::new(
            EngineConfig {
                ws_root: project_root.clone(),
                max_iters: self.config.agent.max_iters,
                stream: true,
            },
            self.models.clone(),
            model_id,
            role,
        )?;

        engine.set_spec(agent_id.to_string(), spec);
        engine.set_manager_context(self.clone());

        let agent = Arc::new(Mutex::new(engine));
        agents.insert(agent_id.to_string(), agent.clone());
        Ok(agent)
    }

    pub async fn is_path_allowed(
        &self,
        project_root: &PathBuf,
        agent_id: &str,
        path: &str,
    ) -> bool {
        let projects = self.projects.lock().await;
        let key = project_root.to_string_lossy().to_string();
        if let Some(ctx) = projects.get(&key) {
            let agents = ctx.agents.lock().await;
            if let Some(agent) = agents.get(agent_id) {
                let engine = agent.lock().await;
                if let Some(spec) = engine.get_spec() {
                    if spec.work_globs.is_empty() {
                        return true;
                    }
                    for glob_str in &spec.work_globs {
                        if let Ok(glob) = Glob::new(glob_str) {
                            if glob.compile_matcher().is_match(path) {
                                return true;
                            }
                        }
                    }
                    return false;
                }
            }
        }
        true
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentSpec>> {
        let mut result = Vec::new();
        for agent_ref in &self.config.agents {
            let spec_path = PathBuf::from(&agent_ref.spec_path);
            if spec_path.exists() {
                let (spec, _) = AgentSpec::from_markdown(&spec_path)?;
                result.push(spec);
            } else {
                warn!("Agent spec file not found: {:?}", spec_path);
            }
        }
        Ok(result)
    }

    pub fn get_config(&self) -> &Config {
        &self.config
    }

    #[allow(dead_code)]
    pub async fn send_event(&self, event: AgentEvent) {
        let _ = self.events.send(event);
    }

    pub async fn sync_world_state(&self, project_root: &PathBuf) -> Result<()> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let ctx = self.get_or_create_project(project_root).await?;
        let tasks = ctx.state_fs.list_tasks()?;

        let active_coder_task = tasks.iter().find(|(meta, _)| match meta {
            StateFile::CoderTask {
                status,
                assigned_to,
                ..
            } => status == "active" && assigned_to == "coder",
            _ => false,
        });

        if let Some((StateFile::CoderTask { id, .. }, body)) = active_coder_task {
            let agents = ctx.agents.lock().await;
            if let Some(coder) = agents.get("coder") {
                let mut engine = coder.lock().await;
                let current_task = engine.get_task();
                if current_task.as_deref() != Some(body) {
                    tracing::info!("Syncing active task {} to coder agent", id);
                    engine.set_task(body.clone());
                    let _ = self.events.send(AgentEvent::TaskUpdate {
                        agent_id: "coder".to_string(),
                        task: body.clone(),
                    });
                }
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn broadcast_message(
        &self,
        project_root: &PathBuf,
        from: &str,
        to: &str,
        content: &str,
        task_id: Option<String>,
        session_id: Option<&str>,
    ) -> Result<()> {
        tracing::info!("Agent Message: {} -> {}: {}", from, to, content);
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let ctx = self.get_or_create_project(project_root).await?;
        ctx.state_fs
            .append_message(from, to, content, task_id, session_id)?;

        let _ = self.events.send(AgentEvent::Message {
            from: from.to_string(),
            to: to.to_string(),
            content: content.to_string(),
        });

        Ok(())
    }
}
