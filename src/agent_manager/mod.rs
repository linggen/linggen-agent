use crate::agent_manager::locks::LockManager;
use crate::agent_manager::models::ModelManager;
use crate::config::{AgentKind, AgentSpec, Config};
use crate::db::{AgentRunRecord, Db, ProjectSettings};
use crate::engine::{AgentEngine, AgentOutcome, AgentRole, EngineConfig, PromptMode};
use crate::skills::SkillManager;
use crate::state_fs::{StateFile, StateFs};
use anyhow::Result;
use globset::Glob;
use ignore::gitignore::GitignoreBuilder;
use notify::{EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::warn;

pub mod locks;
pub mod models;

pub struct ProjectContext {
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
    working_places: Mutex<HashMap<String, HashMap<String, WorkingPlaceEntry>>>,
    cancelled_runs: Mutex<HashSet<String>>,
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
    AgentStatus {
        agent_id: String,
        status: String,
        detail: Option<String>,
    },
    SubagentSpawned {
        parent_id: String,
        subagent_id: String,
        task: String,
    },
    SubagentResult {
        parent_id: String,
        subagent_id: String,
        outcome: AgentOutcome,
    },
    ContextUsage {
        agent_id: String,
        stage: String,
        message_count: usize,
        char_count: usize,
        estimated_tokens: usize,
        #[serde(default)]
        token_limit: Option<usize>,
        compressed: bool,
        summary_count: usize,
    },
    StateUpdated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingPlaceEntry {
    pub repo_path: String,
    pub file_path: String,
    pub agent_id: String,
    pub run_id: Option<String>,
    pub last_modified: u64,
}

impl AgentManager {
    fn make_run_id(agent_id: &str) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        format!(
            "run-{}-{}-{}",
            agent_id,
            now.as_secs(),
            now.subsec_nanos()
        )
    }

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
                working_places: Mutex::new(HashMap::new()),
                cancelled_runs: Mutex::new(HashSet::new()),
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
            agents: Mutex::new(HashMap::new()),
            state_fs,
            watcher: Mutex::new(None),
        });

        // Setup watcher
        let db_clone = self.db.clone();
        let root_clone = root.clone();
        let events_tx = self.events.clone();
        let mut gitignore_builder = GitignoreBuilder::new(&root);
        let root_gitignore = root.join(".gitignore");
        if root_gitignore.exists() {
            if let Some(path_str) = root_gitignore.to_str() {
                if let Some(err) = gitignore_builder.add(path_str) {
                    tracing::warn!("Failed to load .gitignore: {}", err);
                }
            }
        }
        let gitignore = Arc::new(gitignore_builder.build()?);

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    let repo_path = root_clone.to_string_lossy().to_string();
                    let gitignore = gitignore.clone();
                    let is_ignored = |path: &std::path::Path| -> bool {
                        match path.strip_prefix(&root_clone) {
                            Ok(rel) => gitignore
                                .matched_path_or_any_parents(rel, false)
                                .is_ignore(),
                            Err(_) => false,
                        }
                    };
                    
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
                            let mut changed = false;
                            for path in event.paths {
                                if is_ignored(&path) {
                                    continue;
                                }
                                if let Ok(rel) = path.strip_prefix(&root_clone) {
                                    let rel_str = rel.to_string_lossy();
                                    tracing::info!("File removed on disk: {}", rel_str);
                                    let _ = db_clone.remove_activity(&repo_path, &rel_str);
                                    changed = true;
                                }
                            }
                            if changed {
                                let _ = events_tx.send(AgentEvent::StateUpdated);
                            }
                        }
                        EventKind::Modify(notify::event::ModifyKind::Name(
                            notify::event::RenameMode::Both,
                        )) => {
                            let mut changed = false;
                            if event.paths.len() == 2 {
                                let old = &event.paths[0];
                                let new = &event.paths[1];
                                let old_ignored = is_ignored(old);
                                let new_ignored = is_ignored(new);
                                if let (Ok(old_rel), Ok(new_rel)) =
                                    (old.strip_prefix(&root_clone), new.strip_prefix(&root_clone))
                                {
                                    let old_str = old_rel.to_string_lossy();
                                    let new_str = new_rel.to_string_lossy();
                                    if !old_ignored && !new_ignored {
                                        tracing::info!("File renamed on disk: {} -> {}", old_str, new_str);
                                        let _ = db_clone.rename_activity(
                                            &repo_path,
                                            &old_str,
                                            &new_str,
                                        );
                                        changed = true;
                                    } else if !old_ignored && new_ignored {
                                        let _ = db_clone.remove_activity(&repo_path, &old_str);
                                        changed = true;
                                    }
                                }
                            }
                            if changed {
                                let _ = events_tx.send(AgentEvent::StateUpdated);
                            }
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

        let (spec, system_prompt) =
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
                write_safety_mode: self.config.agent.write_safety_mode,
                prompt_loop_breaker: self.config.agent.prompt_loop_breaker.clone(),
            },
            self.models.clone(),
            model_id,
            role,
        )?;

        engine.set_spec(agent_id.to_string(), spec, system_prompt);
        engine.set_manager_context(self.clone());
        if let Ok(settings) = self
            .db
            .get_project_settings(&project_root.to_string_lossy())
        {
            let mode = if settings.mode == "chat" {
                PromptMode::Chat
            } else {
                PromptMode::Structured
            };
            engine.set_prompt_mode(mode);
        }

        let agent = Arc::new(Mutex::new(engine));
        agents.insert(agent_id.to_string(), agent.clone());
        Ok(agent)
    }

    pub async fn is_path_allowed(
        &self,
        _project_root: &PathBuf,
        agent_id: &str,
        path: &str,
    ) -> bool {
        // Important: do NOT lock a live agent engine here.
        // `write_file` is called while the engine mutex is already held by run_agent_loop,
        // and re-locking that same engine causes a deadlock.
        let Some(agent_ref) = self.config.agents.iter().find(|a| a.id == agent_id) else {
            return true;
        };

        let spec_path = PathBuf::from(&agent_ref.spec_path);
        let Ok((spec, _)) = AgentSpec::from_markdown(&spec_path) else {
            return true;
        };

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
        false
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

    pub async fn resolve_agent_kind(&self, agent_id: &str) -> Option<AgentKind> {
        let agent_ref = self.config.agents.iter().find(|a| a.id == agent_id)?;
        let spec_path = PathBuf::from(&agent_ref.spec_path);
        let Ok((spec, _)) = AgentSpec::from_markdown(&spec_path) else {
            return None;
        };
        Some(spec.kind)
    }

    pub async fn upsert_working_place(
        &self,
        repo_path: &str,
        agent_id: &str,
        file_path: &str,
        run_id: Option<String>,
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let entry = WorkingPlaceEntry {
            repo_path: repo_path.to_string(),
            file_path: file_path.to_string(),
            agent_id: agent_id.to_string(),
            run_id,
            last_modified: now,
        };
        let mut places = self.working_places.lock().await;
        let repo = places.entry(repo_path.to_string()).or_default();
        repo.insert(agent_id.to_string(), entry);
    }

    pub async fn clear_working_place_for_agent(&self, repo_path: &str, agent_id: &str) {
        let mut places = self.working_places.lock().await;
        if let Some(repo_map) = places.get_mut(repo_path) {
            repo_map.remove(agent_id);
            if repo_map.is_empty() {
                places.remove(repo_path);
            }
        }
    }

    pub async fn clear_working_place_for_run(&self, run_id: &str) {
        let mut places = self.working_places.lock().await;
        let repos: Vec<String> = places.keys().cloned().collect();
        for repo in repos {
            if let Some(repo_map) = places.get_mut(&repo) {
                repo_map.retain(|_, entry| entry.run_id.as_deref() != Some(run_id));
                if repo_map.is_empty() {
                    places.remove(&repo);
                }
            }
        }
    }

    pub async fn list_working_places_for_repo(&self, repo_path: &str) -> Vec<WorkingPlaceEntry> {
        let places = self.working_places.lock().await;
        places
            .get(repo_path)
            .map(|repo| repo.values().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn begin_agent_run(
        &self,
        project_root: &PathBuf,
        session_id: Option<&str>,
        agent_id: &str,
        parent_run_id: Option<String>,
        detail: Option<String>,
    ) -> Result<String> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let kind = self
            .resolve_agent_kind(agent_id)
            .await
            .unwrap_or(AgentKind::Main);
        let run_id = Self::make_run_id(agent_id);
        let started_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let repo_path = project_root.to_string_lossy().to_string();

        let record = AgentRunRecord {
            run_id: run_id.clone(),
            repo_path: repo_path.clone(),
            session_id: session_id.unwrap_or("default").to_string(),
            agent_id: agent_id.to_string(),
            agent_kind: match kind {
                AgentKind::Main => "main".to_string(),
                AgentKind::Subagent => "subagent".to_string(),
            },
            parent_run_id,
            status: "running".to_string(),
            detail,
            started_at,
            ended_at: None,
        };
        self.db.add_agent_run(record)?;
        self.clear_working_place_for_agent(&repo_path, agent_id).await;
        self.cancelled_runs.lock().await.remove(&run_id);
        Ok(run_id)
    }

    pub async fn finish_agent_run(
        &self,
        run_id: &str,
        status: &str,
        detail: Option<String>,
    ) -> Result<()> {
        let ended_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
        );
        self.db.update_agent_run(run_id, status, detail, ended_at)?;
        self.clear_working_place_for_run(run_id).await;
        let _ = self.events.send(AgentEvent::StateUpdated);
        self.cancelled_runs.lock().await.remove(run_id);
        Ok(())
    }

    pub async fn list_agent_runs(
        &self,
        project_root: &PathBuf,
        session_id: Option<&str>,
    ) -> Result<Vec<AgentRunRecord>> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        self.db
            .list_agent_runs(&project_root.to_string_lossy(), session_id)
            .map_err(Into::into)
    }

    pub async fn list_agent_children(&self, parent_run_id: &str) -> Result<Vec<AgentRunRecord>> {
        self.db.list_agent_children(parent_run_id).map_err(Into::into)
    }

    pub async fn get_agent_run(&self, run_id: &str) -> Result<Option<AgentRunRecord>> {
        self.db.get_agent_run(run_id).map_err(Into::into)
    }

    pub async fn is_run_cancelled(&self, run_id: &str) -> bool {
        self.cancelled_runs.lock().await.contains(run_id)
    }

    pub async fn cancel_run_tree(&self, run_id: &str) -> Result<Vec<AgentRunRecord>> {
        let mut stack = vec![run_id.to_string()];
        let mut seen = HashSet::new();
        let mut runs = Vec::new();

        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(run) = self.db.get_agent_run(&id)? else {
                continue;
            };
            for child in self.db.list_agent_children(&id)? {
                stack.push(child.run_id.clone());
            }
            runs.push(run);
        }

        let to_cancel: Vec<AgentRunRecord> = runs
            .into_iter()
            .filter(|run| run.status == "running")
            .collect();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        {
            let mut cancelled = self.cancelled_runs.lock().await;
            for run in &to_cancel {
                cancelled.insert(run.run_id.clone());
            }
        }
        for run in &to_cancel {
            let _ = self.db.update_agent_run(
                &run.run_id,
                "cancelled",
                Some("cancelled by user".to_string()),
                Some(now),
            );
            self.clear_working_place_for_run(&run.run_id).await;
        }
        if !to_cancel.is_empty() {
            let _ = self.events.send(AgentEvent::StateUpdated);
        }

        Ok(to_cancel)
    }

    pub fn get_config(&self) -> &Config {
        &self.config
    }

    pub async fn get_project_settings(&self, project_root: &PathBuf) -> Result<ProjectSettings> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        self.db
            .get_project_settings(&project_root.to_string_lossy())
            .map_err(Into::into)
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

    pub async fn set_project_prompt_mode(
        &self,
        project_root: &PathBuf,
        mode: PromptMode,
    ) -> Result<()> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let ctx = self.get_or_create_project(project_root).await?;
        let agents = ctx.agents.lock().await;
        for agent in agents.values() {
            let mut engine = agent.lock().await;
            engine.set_prompt_mode(mode);
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
