use crate::agent_manager::AgentManager;
use crate::config::{AgentKind, AgentPolicy, AgentPolicyCapability};
use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use grep::regex::RegexMatcher;
use grep::searcher::sinks::UTF8;
use grep::searcher::Searcher;
use headless_chrome::Browser;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::info;

#[derive(Debug)]
pub struct ToolCall {
    pub tool: String,
    pub args: Value,
}

#[derive(Debug, Serialize)]
pub enum ToolResult {
    RepoInfo(String),
    FileList(Vec<String>),
    FileContent {
        path: String,
        content: String,
        truncated: bool,
    },
    SearchMatches(Vec<SearchMatch>),
    CommandOutput {
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Screenshot {
        url: String,
        base64: String,
    },
    Success(String),
    LockResult {
        acquired: Vec<(String, String)>,
        denied: Vec<String>,
    },
    AgentOutcome(crate::engine::AgentOutcome),
}

#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub snippet: String,
}

pub struct Tools {
    root: PathBuf,
    manager: Option<Arc<AgentManager>>,
    agent_id: Option<String>,
    agent_kind: AgentKind,
    run_id: Option<String>,
    agent_policy: Option<AgentPolicy>,
}

impl Tools {
    pub fn new(root: PathBuf) -> Result<Self> {
        Ok(Self {
            root,
            manager: None,
            agent_id: None,
            agent_kind: AgentKind::Main,
            run_id: None,
            agent_policy: None,
        })
    }

    pub fn set_context(
        &mut self,
        manager: Arc<AgentManager>,
        agent_id: String,
        agent_kind: AgentKind,
    ) {
        self.manager = Some(manager);
        self.agent_id = Some(agent_id);
        self.agent_kind = agent_kind;
    }

    pub fn set_policy(&mut self, policy: Option<AgentPolicy>) {
        self.agent_policy = policy;
    }

    pub fn set_run_id(&mut self, run_id: Option<String>) {
        self.run_id = run_id;
    }

    pub fn get_manager(&self) -> Option<Arc<AgentManager>> {
        self.manager.clone()
    }

    pub fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        let normalized_args = normalize_tool_args(&call.tool, &call.args);
        info!(
            "Executing tool: {} with args: {}",
            call.tool,
            summarize_tool_args(&call.tool, &normalized_args)
        );
        match call.tool.as_str() {
            "get_repo_info" => Ok(ToolResult::RepoInfo(format!(
                "root={} platform={}",
                self.root.display(),
                std::env::consts::OS
            ))),
            "Glob" => {
                let args: ListFilesArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for Glob: {}", e))?;
                self.list_files(args)
            }
            "Read" => {
                let args: ReadFileArgs = serde_json::from_value(normalized_args).map_err(|e| {
                    anyhow::anyhow!(
                        "invalid args for Read: {}. Expected keys: path|max_bytes|line_range",
                        e
                    )
                })?;
                self.read_file(args)
            }
            "Grep" => {
                let args: SearchArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for Grep: {}", e))?;
                self.search_rg(args)
            }
            "Bash" => {
                let args: RunCommandArgs =
                    serde_json::from_value(normalized_args).map_err(|e| {
                        anyhow::anyhow!(
                            "invalid args for Bash: {}. Expected keys: cmd|timeout_ms",
                            e
                        )
                    })?;
                self.run_command(args)
            }
            "capture_screenshot" => {
                let args: CaptureScreenshotArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for capture_screenshot: {}", e))?;
                self.capture_screenshot(args)
            }
            "Write" => {
                let args: WriteFileArgs = serde_json::from_value(normalized_args).map_err(|e| {
                    anyhow::anyhow!("invalid args for Write: {}. Expected keys: path|content", e)
                })?;
                self.write_file(args)
            }
            "lock_paths" => {
                let args: LockPathsArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for lock_paths: {}", e))?;
                self.lock_paths(args)
            }
            "unlock_paths" => {
                let args: UnlockPathsArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for unlock_paths: {}", e))?;
                self.unlock_paths(args)
            }
            "delegate_to_agent" => {
                let args: DelegateToAgentArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for delegate_to_agent: {}", e))?;
                self.delegate_to_agent(args)
            }
            _ => anyhow::bail!("unknown tool: {}", call.tool),
        }
    }

    pub fn list_files(&self, args: ListFilesArgs) -> Result<ToolResult> {
        let globset = build_globset(args.globs.as_deref())?;
        let max_results = args.max_results.unwrap_or(200);

        let mut out = Vec::new();
        let walker = WalkBuilder::new(&self.root)
            .standard_filters(true)
            .hidden(true)
            .build();

        for entry in walker {
            let entry = match entry {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            let rel = to_rel_string(&self.root, path)?;
            if let Some(gs) = &globset {
                if !gs.is_match(Path::new(&rel)) {
                    continue;
                }
            }
            out.push(rel);
            if out.len() >= max_results {
                break;
            }
        }

        Ok(ToolResult::FileList(out))
    }

    fn read_file(&self, args: ReadFileArgs) -> Result<ToolResult> {
        let rel = sanitize_rel_path(&self.root, &args.path).unwrap_or_else(|_| args.path.clone());
        let filename = Path::new(&rel)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&rel);
        let path = self.root.join(&rel);

        if path.exists() && path.is_dir() {
            anyhow::bail!(
                "path '{}' is a directory. Use Glob to enumerate files, then Read with an exact file path.",
                rel
            );
        }

        if path.exists() && path.is_file() {
            return self.do_read_file(&rel, &path, args.max_bytes, args.line_range);
        }

        // File not found, try smart search candidates.
        info!("File not found: {}. Attempting smart search...", rel);
        let candidates = self.smart_search_candidates(&rel, 10)?;
        if let Some(best_match) = candidates.first() {
            let full_path = self.root.join(best_match);
            let note = if candidates.len() > 1 {
                format!(
                    "Note: Original path '{}' not found. Found {} candidate files, reading '{}'. Others: {}",
                    rel,
                    candidates.len(),
                    best_match,
                    candidates[1..].join(", ")
                )
            } else {
                format!(
                    "Note: Original path '{}' not found. Automatically found and read '{}' instead.",
                    rel, best_match
                )
            };
            return self.do_read_file_with_note(
                best_match,
                &full_path,
                args.max_bytes,
                args.line_range,
                &note,
            );
        }

        // 3. Last resort: tell the model we searched everywhere
        Ok(ToolResult::Success(format!(
            "file_not_found: {} - I searched the whole repository for '{}' (including case-insensitive and partial matches) but found nothing. Please verify the filename or use 'Glob' to explore the directory structure.",
            rel, filename
        )))
    }

    fn smart_search_candidates(&self, query: &str, max_results: usize) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let limit = max_results.max(1);

        let filename = Path::new(query)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(query);

        // 1) Exact basename matches anywhere in the repo.
        if !filename.is_empty() {
            let glob_pattern = format!("**/{}", filename);
            if let Ok(ToolResult::FileList(matches)) = self.list_files(ListFilesArgs {
                globs: Some(vec![glob_pattern]),
                max_results: Some(limit),
            }) {
                for m in matches {
                    if seen.insert(m.clone()) {
                        out.push(m);
                        if out.len() >= limit {
                            return Ok(out);
                        }
                    }
                }
            }
        }

        // 2) Case-insensitive filename/path contains matches.
        let query_lower = query.to_lowercase();
        let filename_lower = filename.to_lowercase();
        let walker = WalkBuilder::new(&self.root)
            .standard_filters(true)
            .hidden(true)
            .build();

        for entry in walker {
            let entry = match entry {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            let rel = match to_rel_string(&self.root, path) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let rel_lower = rel.to_lowercase();
            let name_lower = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_lowercase();

            let name_match = !filename_lower.is_empty()
                && !name_lower.is_empty()
                && (name_lower == filename_lower
                    || name_lower.contains(&filename_lower)
                    || filename_lower.contains(&name_lower));
            let path_match = !query_lower.is_empty() && rel_lower.contains(&query_lower);
            if (name_match || path_match) && seen.insert(rel.clone()) {
                out.push(rel);
                if out.len() >= limit {
                    break;
                }
            }
        }

        Ok(out)
    }

    fn do_read_file(
        &self,
        rel: &str,
        path: &Path,
        max_bytes: Option<usize>,
        line_range: Option<[usize; 2]>,
    ) -> Result<ToolResult> {
        let max = max_bytes.unwrap_or(64 * 1024);

        let (content, truncated) = if let Some([start, end]) = line_range {
            if start == 0 || end < start {
                anyhow::bail!(
                    "invalid line_range [{}, {}]; expected 1-based inclusive range with start <= end",
                    start,
                    end
                );
            }

            use std::io::BufRead;
            let file = fs::File::open(path)?;
            let reader = std::io::BufReader::new(file);
            let mut out = String::new();
            let mut truncated = false;

            for (idx, line_res) in reader.lines().enumerate() {
                let line_no = idx + 1;
                if line_no < start {
                    continue;
                }
                if line_no > end {
                    break;
                }
                let line = line_res?;
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&line);
                if out.len() > max {
                    out.truncate(max);
                    truncated = true;
                    break;
                }
            }

            (out, truncated)
        } else {
            let file = fs::File::open(path)?;
            let mut buf = Vec::new();
            use std::io::Read;
            file.take(max as u64 + 1).read_to_end(&mut buf)?;
            let truncated = buf.len() > max;
            if truncated {
                buf.truncate(max);
            }
            (String::from_utf8_lossy(&buf).to_string(), truncated)
        };

        Ok(ToolResult::FileContent {
            path: rel.to_string(),
            content,
            truncated,
        })
    }

    fn do_read_file_with_note(
        &self,
        rel: &str,
        path: &Path,
        max_bytes: Option<usize>,
        line_range: Option<[usize; 2]>,
        note: &str,
    ) -> Result<ToolResult> {
        match self.do_read_file(rel, path, max_bytes, line_range)? {
            ToolResult::FileContent {
                path,
                content,
                truncated,
            } => Ok(ToolResult::FileContent {
                path: format!("{} ({})", path, note),
                content: format!("/* {} */\n\n{}", note, content),
                truncated,
            }),
            other => Ok(other),
        }
    }

    fn search_rg(&self, args: SearchArgs) -> Result<ToolResult> {
        let globset = build_globset(args.globs.as_deref())?;
        let max_results = args.max_results.unwrap_or(200);

        let matcher = RegexMatcher::new(&args.query).unwrap_or_else(|_| {
            let escaped = regex::escape(&args.query);
            RegexMatcher::new(&escaped).expect("escaped regex")
        });

        let mut matches = Vec::new();
        let mut searcher = Searcher::new();
        let walker = WalkBuilder::new(&self.root)
            .standard_filters(true)
            .hidden(true)
            .build();

        for entry in walker {
            let entry = match entry {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }

            let path = entry.path();
            let rel_path = to_rel_string(&self.root, path)?;
            if let Some(gs) = &globset {
                if !gs.is_match(Path::new(&rel_path)) {
                    continue;
                }
            }
            let mut file_matches = Vec::new();

            let _ = searcher.search_path(
                &matcher,
                path,
                UTF8(|line_num, line_content| {
                    file_matches.push(SearchMatch {
                        path: rel_path.clone(),
                        line: line_num as usize,
                        snippet: line_content.trim_end().to_string(),
                    });
                    if matches.len() + file_matches.len() >= max_results {
                        return Ok(false);
                    }
                    Ok(true)
                }),
            );

            matches.extend(file_matches);
            if matches.len() >= max_results {
                break;
            }
        }

        Ok(ToolResult::SearchMatches(matches))
    }

    fn run_command(&self, args: RunCommandArgs) -> Result<ToolResult> {
        if self.agent_kind == AgentKind::Main && is_repo_discovery_command(&args.cmd) {
            anyhow::bail!(
                "Policy: main agents must delegate repository discovery to a subagent via delegate_to_agent"
            );
        }

        // Hybrid shell mode: support common dev/inspection tools while enforcing
        // an allowlist on every shell segment.
        validate_shell_command(&args.cmd)?;
        let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(30000));

        let mut child = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .args(["/C", &args.cmd])
                .current_dir(&self.root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?
        } else {
            Command::new("sh")
                .arg("-c")
                .arg(&args.cmd)
                .current_dir(&self.root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?
        };

        let start = Instant::now();
        let mut timed_out = false;
        loop {
            if let Some(_status) = child.try_wait()? {
                break;
            }
            if start.elapsed() >= timeout {
                timed_out = true;
                let _ = child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        let output = child.wait_with_output()?;
        let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if timed_out {
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(&format!(
                "linggen-agent: command timed out after {}ms\n",
                timeout.as_millis()
            ));
        }

        Ok(ToolResult::CommandOutput {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr,
        })
    }

    fn capture_screenshot(&self, args: CaptureScreenshotArgs) -> Result<ToolResult> {
        let browser = Browser::default()?;
        let tab = browser.new_tab()?;

        tab.navigate_to(&args.url)?;
        tab.wait_until_navigated()?;

        // Wait a bit for dynamic content
        std::thread::sleep(Duration::from_millis(args.delay_ms.unwrap_or(1000)));

        let png_data = tab.capture_screenshot(
            headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
            None,
            None,
            true,
        )?;

        use base64::prelude::*;
        let b64 = BASE64_STANDARD.encode(png_data);

        Ok(ToolResult::Screenshot {
            url: args.url,
            base64: b64,
        })
    }

    fn write_file(&self, args: WriteFileArgs) -> Result<ToolResult> {
        let rel = sanitize_rel_path(&self.root, &args.path)?;
        let path = self.root.join(&rel);
        let new_content = args.content;

        // Enforcement check
        if let (Some(manager), Some(agent_id)) = (&self.manager, &self.agent_id) {
            // 1. Check work_globs
            let allowed = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(async { manager.is_path_allowed(&self.root, agent_id, &rel).await })
            });

            if !allowed {
                anyhow::bail!(
                    "Path {} is outside the allowed WorkScope for agent {}",
                    rel,
                    agent_id
                );
            }

            // 2. Check locks
            let locked_by_other = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    manager
                        .locks
                        .lock()
                        .await
                        .is_locked_by_other(agent_id, &rel)
                })
            });

            if locked_by_other {
                anyhow::bail!("Path {} is locked by another agent", rel);
            }

            // Record activity in DB
            let _ = manager.db.record_activity(crate::db::FileActivity {
                repo_path: self.root.to_string_lossy().to_string(),
                file_path: rel.clone(),
                agent_id: agent_id.clone(),
                status: "working".to_string(),
                last_modified: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            });

            // Live working-place map for active-path UI (in-memory source of truth).
            if self.run_id.is_some() {
                let repo_path = self.root.to_string_lossy().to_string();
                let run_id = self.run_id.clone();
                let rel_for_map = rel.clone();
                let agent_for_map = agent_id.clone();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        manager
                            .upsert_working_place(&repo_path, &agent_for_map, &rel_for_map, run_id)
                            .await;
                    })
                });
            }
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        if path.exists() {
            let existing = fs::read_to_string(&path).unwrap_or_default();
            if existing == new_content {
                return Ok(ToolResult::Success(format!(
                    "File unchanged (content identical): {}",
                    rel
                )));
            }
        }

        let bytes = new_content.len();
        fs::write(&path, new_content)?;
        Ok(ToolResult::Success(format!(
            "File written: {} ({} bytes)",
            rel, bytes
        )))
    }

    fn lock_paths(&self, args: LockPathsArgs) -> Result<ToolResult> {
        let (manager, agent_id) = match (&self.manager, &self.agent_id) {
            (Some(m), Some(id)) => (m, id),
            _ => anyhow::bail!("Locking requires AgentManager context"),
        };

        let ttl = Duration::from_millis(args.ttl_ms.unwrap_or(300000)); // Default 5 min
        let res = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                manager
                    .locks
                    .lock()
                    .await
                    .acquire(agent_id, args.globs, ttl)
            })
        });

        Ok(ToolResult::LockResult {
            acquired: res.acquired,
            denied: res.denied,
        })
    }

    fn unlock_paths(&self, args: UnlockPathsArgs) -> Result<ToolResult> {
        let (manager, agent_id) = match (&self.manager, &self.agent_id) {
            (Some(m), Some(id)) => (m, id),
            _ => anyhow::bail!("Locking requires AgentManager context"),
        };

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async { manager.locks.lock().await.release(agent_id, args.tokens) })
        });

        Ok(ToolResult::Success("Locks released".to_string()))
    }

    fn delegate_to_agent(&self, args: DelegateToAgentArgs) -> Result<ToolResult> {
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Delegation requires AgentManager context"))?;
        let caller_id = self
            .agent_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Delegation requires caller agent id"))?;
        let target_agent_id = args.target_agent_id;
        let task = args.task;

        if self.agent_kind == AgentKind::Subagent {
            anyhow::bail!(
                "Delegation denied: subagent '{}' cannot spawn subagents (max delegation depth is 1)",
                caller_id
            );
        }
        let policy = self
            .agent_policy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Delegation denied: missing agent policy"))?;
        if !policy.allows(AgentPolicyCapability::Delegate) {
            anyhow::bail!(
                "Delegation denied: agent '{}' policy does not allow Delegate",
                caller_id
            );
        }
        if !policy.allows_delegate_target(&target_agent_id) {
            let allowed = if policy.delegate_targets.is_empty() {
                "(none)".to_string()
            } else {
                policy.delegate_targets.join(", ")
            };
            anyhow::bail!(
                "Delegation denied: target '{}' is not allowed by policy for '{}'. Allowed: {}",
                target_agent_id,
                caller_id,
                allowed
            );
        }

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let target_kind = manager
                    .resolve_agent_kind(&self.root, &target_agent_id)
                    .await
                    .unwrap_or(AgentKind::Main);
                let parent_run_id = if target_kind == AgentKind::Subagent {
                    self.run_id.clone()
                } else {
                    None
                };
                let run_id = manager
                    .begin_agent_run(
                        &self.root,
                        None,
                        &target_agent_id,
                        parent_run_id,
                        Some(format!("delegated by {}", caller_id)),
                    )
                    .await?;

                manager
                    .send_event(crate::agent_manager::AgentEvent::Message {
                        from: caller_id.clone(),
                        to: target_agent_id.clone(),
                        content: format!("Delegated task: {}", task),
                    })
                    .await;

                if target_kind == AgentKind::Subagent {
                    manager
                        .send_event(crate::agent_manager::AgentEvent::SubagentSpawned {
                            parent_id: caller_id.clone(),
                            subagent_id: target_agent_id.clone(),
                            task: task.clone(),
                        })
                        .await;
                }

                let agent = manager
                    .get_or_create_agent(&self.root, &target_agent_id)
                    .await?;
                let mut engine = agent.lock().await;
                if target_kind == AgentKind::Subagent {
                    engine.set_parent_agent(Some(caller_id.clone()));
                } else {
                    engine.set_parent_agent(None);
                }
                engine.set_run_id(Some(run_id.clone()));
                engine.set_task(task);
                let run_result = engine.run_agent_loop(None).await;
                engine.set_run_id(None);
                engine.set_parent_agent(None);

                let (outcome, status, detail) = match run_result {
                    Ok(outcome) => (outcome, "completed", None),
                    Err(err) => {
                        let msg = err.to_string();
                        let status = if msg.to_lowercase().contains("cancel") {
                            "cancelled"
                        } else {
                            "failed"
                        };
                        let _ = manager.finish_agent_run(&run_id, status, Some(msg)).await;
                        return Err(err);
                    }
                };
                let _ = manager.finish_agent_run(&run_id, status, detail).await;

                if target_kind == AgentKind::Subagent {
                    manager
                        .send_event(crate::agent_manager::AgentEvent::SubagentResult {
                            parent_id: caller_id,
                            subagent_id: target_agent_id,
                            outcome: outcome.clone(),
                        })
                        .await;
                }
                Ok(ToolResult::AgentOutcome(outcome))
            })
        })
    }
}

#[derive(Debug, Deserialize)]
struct WriteFileArgs {
    #[serde(alias = "file", alias = "filepath")]
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct LockPathsArgs {
    globs: Vec<String>,
    ttl_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct UnlockPathsArgs {
    tokens: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ListFilesArgs {
    globs: Option<Vec<String>>,
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ReadFileArgs {
    #[serde(alias = "file", alias = "filepath")]
    path: String,
    max_bytes: Option<usize>,
    line_range: Option<[usize; 2]>,
}

#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
    globs: Option<Vec<String>>,
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RunCommandArgs {
    cmd: String,
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct CaptureScreenshotArgs {
    url: String,
    delay_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DelegateToAgentArgs {
    target_agent_id: String,
    task: String,
}

fn build_globset(globs: Option<&[String]>) -> Result<Option<GlobSet>> {
    let Some(globs) = globs else {
        return Ok(None);
    };
    if globs.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for g in globs {
        builder.add(Glob::new(g)?);
    }
    Ok(Some(builder.build()?))
}

fn sanitize_rel_path(root: &Path, path: &str) -> Result<String> {
    use std::path::Component;

    if path.is_empty() {
        anyhow::bail!("empty path");
    }
    let raw = Path::new(path);
    let rel_path = if raw.is_absolute() {
        raw.strip_prefix(root)
            .map_err(|_| anyhow::anyhow!("absolute path must be inside workspace root"))?
            .to_path_buf()
    } else {
        raw.to_path_buf()
    };

    if rel_path.as_os_str().is_empty() {
        anyhow::bail!("empty path");
    }
    if rel_path
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        anyhow::bail!("path traversal not allowed");
    }
    if rel_path
        .components()
        .any(|c| matches!(c, Component::RootDir | Component::Prefix(_)))
    {
        anyhow::bail!("path must resolve inside workspace root");
    }

    Ok(rel_path.to_string_lossy().to_string())
}

fn to_rel_string(root: &Path, path: &Path) -> Result<String> {
    let rel = path.strip_prefix(root)?;
    Ok(rel.to_string_lossy().to_string())
}

fn summarize_tool_args(tool: &str, args: &Value) -> String {
    let mut safe_args = args.clone();
    if let Some(obj) = safe_args.as_object_mut() {
        match tool {
            "Write" => {
                if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
                    let byte_len = content.len();
                    let line_count = content.lines().count();
                    obj.insert(
                        "content".to_string(),
                        serde_json::json!(format!(
                            "<omitted:{} bytes, {} lines>",
                            byte_len, line_count
                        )),
                    );
                }
            }
            "Bash" => {
                if let Some(cmd) = obj.get("cmd").and_then(|v| v.as_str()) {
                    let preview = if cmd.len() > 160 {
                        format!("{}... (truncated, {} chars)", &cmd[..160], cmd.len())
                    } else {
                        cmd.to_string()
                    };
                    obj.insert("cmd".to_string(), serde_json::json!(preview));
                }
            }
            _ => {}
        }
    }
    safe_args.to_string()
}

fn normalize_tool_args(tool: &str, args: &Value) -> Value {
    let mut normalized = args.clone();
    if let Some(obj) = normalized.as_object_mut() {
        if matches!(tool, "Read" | "Write") && !obj.contains_key("path") {
            if let Some(fp) = obj.get("filepath").cloned() {
                obj.insert("path".to_string(), fp);
            } else if let Some(file) = obj.get("file").cloned() {
                obj.insert("path".to_string(), file);
            }
        }

        if matches!(tool, "Grep") && !obj.contains_key("query") {
            if let Some(path) = obj.get("path").cloned() {
                obj.insert("query".to_string(), path);
            } else if let Some(fp) = obj.get("filepath").cloned() {
                obj.insert("query".to_string(), fp);
            } else if let Some(file) = obj.get("file").cloned() {
                obj.insert("query".to_string(), file);
            }
        }

        if matches!(tool, "Grep" | "Glob")
            && obj.get("globs").map(|v| v.is_string()).unwrap_or(false)
        {
            if let Some(glob) = obj.get("globs").and_then(|v| v.as_str()) {
                obj.insert("globs".to_string(), serde_json::json!([glob]));
            }
        }
    }
    normalized
}

fn validate_shell_command(cmd: &str) -> Result<()> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty command");
    }

    // Disallow common shell injection patterns.
    for banned in ["$(", "`", "\n", "\r"] {
        if trimmed.contains(banned) {
            anyhow::bail!("command contains disallowed shell construct: {}", banned);
        }
    }

    let allowed: HashSet<&str> = [
        "ls", "pwd", "cat", "head", "tail", "wc", "cut", "sort", "uniq", "tr", "sed", "awk",
        "find", "fd", "rg", "grep", "git", "cargo", "rustc", "npm", "pnpm", "yarn", "node",
        "python3", "pytest", "go", "make", "just",
    ]
    .into_iter()
    .collect();

    for segment in split_shell_segments(trimmed) {
        let token = first_segment_token(segment)
            .ok_or_else(|| anyhow::anyhow!("invalid command segment: '{}'", segment))?;
        if !allowed.contains(token) {
            anyhow::bail!(
                "Command not allowed: {} (allowed tools are code/search/build/test commands)",
                token
            );
        }
    }

    Ok(())
}

fn is_repo_discovery_command(cmd: &str) -> bool {
    split_shell_segments(cmd).iter().any(|segment| {
        let Some(token) = first_segment_token(segment) else {
            return false;
        };

        if matches!(token, "rg" | "grep" | "fd" | "find") {
            return true;
        }

        if token == "git" {
            return segment.split_whitespace().any(|part| part == "grep");
        }

        false
    })
}

fn split_shell_segments(cmd: &str) -> Vec<&str> {
    cmd.split(|c| c == '|' || c == ';')
        .flat_map(|part| part.split("&&"))
        .flat_map(|part| part.split("||"))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

fn first_segment_token(segment: &str) -> Option<&str> {
    segment
        .split_whitespace()
        .next()
        .map(|token| token.trim_start_matches('('))
}

pub fn canonical_tool_name(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "get_repo_info" => "get_repo_info",
        "Glob" => "Glob",
        "Read" => "Read",
        "Grep" => "Grep",
        "Write" => "Write",
        "Bash" => "Bash",
        "capture_screenshot" => "capture_screenshot",
        "lock_paths" => "lock_paths",
        "unlock_paths" => "unlock_paths",
        "delegate_to_agent" => "delegate_to_agent",
        _ => return None,
    })
}

fn full_tool_schema_entries() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": "get_repo_info",
            "args": {},
            "returns": "string"
        }),
        serde_json::json!({
            "name": "Glob",
            "args": {"globs": "string[]?", "max_results": "number?"},
            "returns": "string[]"
        }),
        serde_json::json!({
            "name": "Read",
            "args": {"path": "string", "max_bytes": "number?", "line_range": "[number,number]?"},
            "returns": "{path,content,truncated}",
            "notes": "Path aliases accepted: path, file, filepath."
        }),
        serde_json::json!({
            "name": "Grep",
            "args": {"query": "string", "globs": "string[]?", "max_results": "number?"},
            "returns": "{matches:[{path,line,snippet}]}",
            "notes": "Query aliases accepted: query, path, file, filepath."
        }),
        serde_json::json!({
            "name": "Write",
            "args": {"path": "string", "content": "string"},
            "returns": "success",
            "notes": "Path aliases accepted: path, file, filepath."
        }),
        serde_json::json!({
            "name": "Bash",
            "args": {"cmd": "string", "timeout_ms": "number?"},
            "returns": "{exit_code,stdout,stderr}",
            "notes": "Runs allowlisted dev/search/build shell commands with per-segment validation."
        }),
        serde_json::json!({
            "name": "capture_screenshot",
            "args": {"url": "string", "delay_ms": "number?"},
            "returns": "{url,base64}"
        }),
        serde_json::json!({
            "name": "delegate_to_agent",
            "args": {"target_agent_id": "string", "task": "string"},
            "returns": "{agent_outcome}",
            "notes": "Only main agents can delegate. Subagents cannot spawn subagents."
        }),
    ]
}

pub fn tool_schema_json(allowed_tools: Option<&HashSet<String>>) -> String {
    let mut tools = full_tool_schema_entries();
    if let Some(allowed) = allowed_tools {
        tools.retain(|entry| {
            entry
                .get("name")
                .and_then(|v| v.as_str())
                .map(|name| allowed.contains(name))
                .unwrap_or(false)
        });
    }
    serde_json::json!({ "tools": tools }).to_string()
}
