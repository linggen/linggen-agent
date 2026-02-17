use crate::agent_manager::locks::LockManager;
use crate::agent_manager::models::ModelManager;
use crate::config::{AgentKind, AgentPolicyCapability, AgentSpec, Config};
use crate::db::{AgentRunRecord, Db, ProjectMode, ProjectSettings};
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
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{info, warn};

pub mod locks;
pub mod models;

pub struct ProjectContext {
    pub agents: Mutex<HashMap<String, Arc<Mutex<AgentEngine>>>>,
    pub state_fs: StateFs,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

pub struct AgentManager {
    config: RwLock<Config>,
    config_dir: Option<PathBuf>,
    pub projects: Mutex<HashMap<String, Arc<ProjectContext>>>,
    pub locks: Mutex<LockManager>,
    pub models: RwLock<Arc<ModelManager>>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpecFile {
    pub agent_id: String,
    pub spec: AgentSpec,
    pub spec_path: PathBuf,
    #[serde(skip)]
    pub system_prompt: String,
}

impl AgentManager {
    fn normalize_agent_id(agent_id: &str) -> String {
        agent_id.trim().to_lowercase()
    }

    fn canonical_project_root(project_root: &PathBuf) -> PathBuf {
        project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone())
    }

    fn agent_specs_dir(project_root: &PathBuf) -> PathBuf {
        project_root.join("agents")
    }

    fn model_override_for_agent(config: &Config, agent_id: &str) -> Option<String> {
        config
            .agents
            .iter()
            .find(|a| a.id.eq_ignore_ascii_case(agent_id))
            .and_then(|a| a.model.clone())
    }

    fn normalize_model_choice(raw: Option<String>) -> Option<String> {
        raw.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("inherit") {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    }

    fn load_agent_specs_for_project(project_root: &PathBuf) -> Result<Vec<AgentSpecFile>> {
        let agents_dir = Self::agent_specs_dir(project_root);
        if !agents_dir.exists() {
            return Ok(Vec::new());
        }

        let mut paths: Vec<PathBuf> = std::fs::read_dir(&agents_dir)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.eq_ignore_ascii_case("md"))
                        .unwrap_or(false)
            })
            .collect();
        paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        let mut seen = HashSet::new();
        let mut specs = Vec::new();
        for spec_path in paths {
            let (spec, system_prompt) = match AgentSpec::from_markdown(&spec_path) {
                Ok(parsed) => parsed,
                Err(err) => {
                    warn!(
                        "Skipping invalid agent spec {}: {}",
                        spec_path.display(),
                        err
                    );
                    continue;
                }
            };
            let raw_name = spec.name.trim();
            let fallback = spec_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("agent");
            let agent_id = Self::normalize_agent_id(if raw_name.is_empty() {
                fallback
            } else {
                raw_name
            });
            if agent_id.is_empty() {
                warn!(
                    "Skipping agent spec {}: resolved to empty agent id",
                    spec_path.display()
                );
                continue;
            }
            if !seen.insert(agent_id.clone()) {
                warn!(
                    "Skipping agent spec {}: duplicate agent id '{}' in agents directory {}",
                    spec_path.display(),
                    agent_id,
                    agents_dir.display()
                );
                continue;
            }
            specs.push(AgentSpecFile {
                agent_id,
                spec,
                spec_path,
                system_prompt,
            });
        }

        Ok(specs)
    }

    fn find_agent_spec_for_project(
        &self,
        project_root: &PathBuf,
        agent_id: &str,
    ) -> Result<Option<AgentSpecFile>> {
        let wanted = Self::normalize_agent_id(agent_id);
        Ok(Self::load_agent_specs_for_project(project_root)?
            .into_iter()
            .find(|entry| entry.agent_id == wanted))
    }

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
        config_dir: Option<PathBuf>,
        db: Arc<Db>,
        skill_manager: Arc<SkillManager>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<AgentEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let models = Arc::new(ModelManager::new(config.models.clone()));
        (
            Arc::new(Self {
                config: RwLock::new(config),
                config_dir,
                projects: Mutex::new(HashMap::new()),
                locks: Mutex::new(LockManager::new()),
                models: RwLock::new(models),
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
                        s.contains("/target/")
                            || s.contains("/.git/")
                            || s.contains("/.linggen-agent/")
                            || s.contains("/node_modules/")
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
                                        tracing::info!(
                                            "File renamed on disk: {} -> {}",
                                            old_str,
                                            new_str
                                        );
                                        let _ = db_clone
                                            .rename_activity(&repo_path, &old_str, &new_str);
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
        let project_root = Self::canonical_project_root(project_root);
        let ctx = self.get_or_create_project(project_root.clone()).await?;
        let mut agents = ctx.agents.lock().await;
        let normalized_id = Self::normalize_agent_id(agent_id);

        if let Some(agent) = agents.get(&normalized_id) {
            return Ok(agent.clone());
        }

        let agent_spec = self
            .find_agent_spec_for_project(&project_root, &normalized_id)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Agent '{}' not found in {}/agents",
                    normalized_id,
                    project_root.display()
                )
            })?;

        let config = self.config.read().await.clone();
        let models = self.models.read().await.clone();

        let model_id =
            Self::normalize_model_choice(Self::model_override_for_agent(&config, &normalized_id))
                .or_else(|| Self::normalize_model_choice(agent_spec.spec.model.clone()))
                .unwrap_or_else(|| {
                    config
                        .models
                        .first()
                        .map(|m| m.id.clone())
                        .expect("No models configured")
                });

        let role = if agent_spec
            .spec
            .allows_policy(AgentPolicyCapability::Finalize)
        {
            AgentRole::Lead
        } else if agent_spec.spec.allows_policy(AgentPolicyCapability::Patch) {
            AgentRole::Coder
        } else {
            AgentRole::Operator
        };

        let mut engine = AgentEngine::new(
            EngineConfig {
                ws_root: project_root.clone(),
                max_iters: config.agent.max_iters,
                stream: true,
                write_safety_mode: config.agent.write_safety_mode,
                prompt_loop_breaker: config.agent.prompt_loop_breaker.clone(),
            },
            models,
            model_id,
            role,
        )?;

        engine.set_spec(
            normalized_id.clone(),
            agent_spec.spec,
            agent_spec.system_prompt,
        );
        engine.set_manager_context(self.clone());
        if let Ok(settings) = self
            .db
            .get_project_settings(&project_root.to_string_lossy())
        {
            let mode = if settings.mode == ProjectMode::Chat {
                PromptMode::Chat
            } else {
                PromptMode::Structured
            };
            engine.set_prompt_mode(mode);
        }

        let agent = Arc::new(Mutex::new(engine));
        agents.insert(normalized_id, agent.clone());
        Ok(agent)
    }

    pub async fn is_path_allowed(
        &self,
        project_root: &PathBuf,
        agent_id: &str,
        path: &str,
    ) -> bool {
        // Important: do NOT lock a live agent engine here.
        // `write_file` is called while the engine mutex is already held by run_agent_loop,
        // and re-locking that same engine causes a deadlock.
        let project_root = Self::canonical_project_root(project_root);
        let Ok(Some(agent_spec)) = self.find_agent_spec_for_project(&project_root, agent_id) else {
            return true;
        };

        if agent_spec.spec.work_globs.is_empty() {
            return true;
        }

        for glob_str in &agent_spec.spec.work_globs {
            if let Ok(glob) = Glob::new(glob_str) {
                if glob.compile_matcher().is_match(path) {
                    return true;
                }
            }
        }
        false
    }

    pub async fn list_agent_specs(&self, project_root: &PathBuf) -> Result<Vec<AgentSpecFile>> {
        let project_root = Self::canonical_project_root(project_root);
        Self::load_agent_specs_for_project(&project_root)
    }

    pub async fn list_agents(&self, project_root: &PathBuf) -> Result<Vec<AgentSpec>> {
        let mut out = Vec::new();
        for entry in self.list_agent_specs(project_root).await? {
            out.push(entry.spec);
        }
        Ok(out)
    }

    pub async fn resolve_agent_kind(
        &self,
        project_root: &PathBuf,
        agent_id: &str,
    ) -> Option<AgentKind> {
        let project_root = Self::canonical_project_root(project_root);
        let Ok(found) = self.find_agent_spec_for_project(&project_root, agent_id) else {
            return None;
        };
        found.map(|entry| entry.spec.kind)
    }

    pub async fn invalidate_agent_cache(
        &self,
        project_root: &PathBuf,
        agent_id: Option<&str>,
    ) -> Result<()> {
        let project_root = Self::canonical_project_root(project_root);
        let key = project_root.to_string_lossy().to_string();
        let ctx = {
            let projects = self.projects.lock().await;
            projects.get(&key).cloned()
        };
        let Some(ctx) = ctx else {
            return Ok(());
        };

        let mut agents = ctx.agents.lock().await;
        if let Some(agent_id) = agent_id {
            let normalized = Self::normalize_agent_id(agent_id);
            agents.remove(&normalized);
            info!("Invalidated cached agent '{}' for {}", normalized, key);
        } else {
            agents.clear();
            info!("Invalidated all cached agents for {}", key);
        }
        Ok(())
    }

    pub async fn upsert_working_place(
        &self,
        repo_path: &str,
        agent_id: &str,
        file_path: &str,
        run_id: Option<String>,
    ) {
        let now = crate::util::now_ts_secs();
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
            .resolve_agent_kind(&project_root, agent_id)
            .await
            .unwrap_or(AgentKind::Main);
        let run_id = Self::make_run_id(agent_id);
        let started_at = crate::util::now_ts_secs();
        let repo_path = project_root.to_string_lossy().to_string();

        let record = AgentRunRecord {
            run_id: run_id.clone(),
            repo_path: repo_path.clone(),
            session_id: session_id.unwrap_or("default").to_string(),
            agent_id: agent_id.to_string(),
            agent_kind: kind,
            parent_run_id,
            status: crate::db::AgentRunStatus::Running,
            detail,
            started_at,
            ended_at: None,
        };
        self.db.add_agent_run(record)?;
        self.clear_working_place_for_agent(&repo_path, agent_id)
            .await;
        self.cancelled_runs.lock().await.remove(&run_id);
        Ok(run_id)
    }

    pub async fn finish_agent_run(
        &self,
        run_id: &str,
        status: crate::db::AgentRunStatus,
        detail: Option<String>,
    ) -> Result<()> {
        let ended_at = Some(crate::util::now_ts_secs());
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
        self.db
            .list_agent_children(parent_run_id)
            .map_err(Into::into)
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
            .filter(|run| run.status == crate::db::AgentRunStatus::Running)
            .collect();

        let now = crate::util::now_ts_secs();
        {
            let mut cancelled = self.cancelled_runs.lock().await;
            for run in &to_cancel {
                cancelled.insert(run.run_id.clone());
            }
        }
        for run in &to_cancel {
            let _ = self.db.update_agent_run(
                &run.run_id,
                crate::db::AgentRunStatus::Cancelled,
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

    pub async fn get_config_snapshot(&self) -> Config {
        self.config.read().await.clone()
    }

    pub async fn apply_config(&self, new_config: Config) -> Result<()> {
        new_config.validate()?;
        // Write to disk first — if this fails, in-memory state remains unchanged.
        new_config.save_runtime(self.config_dir.as_deref())?;
        let new_models = Arc::new(ModelManager::new(new_config.models.clone()));
        *self.models.write().await = new_models;
        *self.config.write().await = new_config.clone();

        // Invalidate all cached agents so they pick up new config on next use
        let keys: Vec<String> = {
            let projects = self.projects.lock().await;
            projects.keys().cloned().collect()
        };
        for key in keys {
            let root = PathBuf::from(&key);
            let _ = self.invalidate_agent_cache(&root, None).await;
        }

        let _ = self.events.send(AgentEvent::StateUpdated);
        Ok(())
    }

    pub async fn get_project_settings(&self, project_root: &PathBuf) -> Result<ProjectSettings> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        self.db
            .get_project_settings(&project_root.to_string_lossy())
            .map_err(Into::into)
    }

    pub async fn send_event(&self, event: AgentEvent) {
        let _ = self.events.send(event);
    }

    pub async fn sync_world_state(&self, project_root: &PathBuf) -> Result<()> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let ctx = self.get_or_create_project(project_root.clone()).await?;
        let tasks = ctx.state_fs.list_tasks()?;
        let patch_agent_id = self
            .list_agent_specs(&project_root)
            .await?
            .into_iter()
            .find(|entry| {
                entry.spec.kind == AgentKind::Main
                    && entry.spec.allows_policy(AgentPolicyCapability::Patch)
            })
            .map(|entry| entry.agent_id);

        let active_patch_task = tasks.iter().find(|(meta, _)| match meta {
            StateFile::CoderTask {
                status,
                assigned_to,
                ..
            } => {
                status == "active"
                    && patch_agent_id
                        .as_ref()
                        .map(|agent_id| assigned_to == agent_id)
                        .unwrap_or(false)
            }
            _ => false,
        });

        if let Some((StateFile::CoderTask { id, .. }, body)) = active_patch_task {
            let Some(patch_agent_id) = patch_agent_id else {
                return Ok(());
            };
            let agents = ctx.agents.lock().await;
            if let Some(worker) = agents.get(&patch_agent_id) {
                let mut engine = worker.lock().await;
                let current_task = engine.get_task();
                if current_task.as_deref() != Some(body) {
                    tracing::info!("Syncing active task {} to {} agent", id, patch_agent_id);
                    engine.set_task(body.clone());
                    let _ = self.events.send(AgentEvent::TaskUpdate {
                        agent_id: patch_agent_id,
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

}

#[cfg(test)]
mod tests {
    use super::AgentManager;
    use std::fs;
    use std::path::PathBuf;

    fn temp_root(prefix: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        dir.push(format!(
            "linggen-agent-{prefix}-{}-{ts}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp root");
        dir
    }

    fn valid_agent_md(name: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: test agent\ntools: [Read]\nkind: main\npolicy: []\n---\n\nYou are {name}.\n"
        )
    }

    #[test]
    fn normalize_model_choice_treats_inherit_as_none() {
        assert_eq!(AgentManager::normalize_model_choice(None), None);
        assert_eq!(
            AgentManager::normalize_model_choice(Some("inherit".to_string())),
            None
        );
        assert_eq!(
            AgentManager::normalize_model_choice(Some("  InHeRiT ".to_string())),
            None
        );
        assert_eq!(
            AgentManager::normalize_model_choice(Some(" local_ollama ".to_string())),
            Some("local_ollama".to_string())
        );
    }

    #[test]
    fn load_agent_specs_skips_invalid_files_and_duplicates() {
        let root = temp_root("agent-specs");
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).expect("create agents dir");

        fs::write(agents_dir.join("a.md"), valid_agent_md("alpha")).expect("write alpha");
        fs::write(agents_dir.join("bad.md"), "this is not frontmatter").expect("write bad");
        fs::write(agents_dir.join("z.md"), valid_agent_md("alpha")).expect("write duplicate");

        let specs = AgentManager::load_agent_specs_for_project(&root).expect("load agent specs");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].agent_id, "alpha");
        assert!(specs[0].spec_path.ends_with("a.md"));

        let _ = fs::remove_dir_all(&root);
    }
}
