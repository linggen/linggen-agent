use crate::agent_manager::AgentManager;
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
}

impl Tools {
    pub fn new(root: PathBuf) -> Result<Self> {
        Ok(Self {
            root,
            manager: None,
            agent_id: None,
        })
    }

    pub fn set_context(&mut self, manager: Arc<AgentManager>, agent_id: String) {
        self.manager = Some(manager);
        self.agent_id = Some(agent_id);
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
            "list_files" | "Glob" => {
                let args: ListFilesArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for list_files: {}", e))?;
                self.list_files(args)
            }
            "read_file" | "Read" => {
                let args: ReadFileArgs = serde_json::from_value(normalized_args).map_err(|e| {
                    anyhow::anyhow!(
                        "invalid args for read_file: {}. Expected keys: path|max_bytes|line_range",
                        e
                    )
                })?;
                self.read_file(args)
            }
            "search_rg" | "Grep" => {
                let args: SearchArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for search_rg: {}", e))?;
                self.search_rg(args)
            }
            "run_command" | "Bash" => {
                let args: RunCommandArgs = serde_json::from_value(normalized_args).map_err(|e| {
                    anyhow::anyhow!(
                        "invalid args for run_command: {}. Expected keys: cmd|timeout_ms",
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
            "write_file" | "Write" => {
                let args: WriteFileArgs = serde_json::from_value(normalized_args).map_err(|e| {
                    anyhow::anyhow!(
                        "invalid args for write_file: {}. Expected keys: path|content",
                        e
                    )
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

    fn list_files(&self, args: ListFilesArgs) -> Result<ToolResult> {
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
        let rel = sanitize_rel_path(&self.root, &args.path)?;
        let path = self.root.join(&rel);
        if !path.exists() {
            // Return a structured "not found" message instead of hard erroring.
            // This helps the model self-correct (e.g. by calling Glob first).
            return Ok(ToolResult::Success(format!(
                "file_not_found: {} (tip: use Glob to list available files first)",
                rel
            )));
        }
        let file = fs::File::open(&path)?;
        let max = args.max_bytes.unwrap_or(64 * 1024);
        let mut buf = Vec::new();
        use std::io::Read;
        file.take(max as u64 + 1).read_to_end(&mut buf)?;
        let truncated = buf.len() > max;
        if truncated {
            buf.truncate(max);
        }
        let content = String::from_utf8_lossy(&buf).to_string();
        Ok(ToolResult::FileContent {
            path: rel,
            content,
            truncated,
        })
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

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let agent = manager.get_or_create_agent(&self.root, &args.target_agent_id).await?;
                let mut engine = agent.lock().await;
                engine.set_task(args.task);
                let outcome = engine.run_agent_loop(None).await?;
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
    #[allow(dead_code)]
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
    if rel_path.components().any(|c| matches!(c, Component::ParentDir)) {
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
            "write_file" | "Write" => {
                if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
                    let byte_len = content.len();
                    let line_count = content.lines().count();
                    obj.insert(
                        "content".to_string(),
                        serde_json::json!(format!("<omitted:{} bytes, {} lines>", byte_len, line_count)),
                    );
                }
            }
            "run_command" | "Bash" => {
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
        if matches!(tool, "read_file" | "Read" | "write_file" | "Write") && !obj.contains_key("path") {
            if let Some(fp) = obj.get("filepath").cloned() {
                obj.insert("path".to_string(), fp);
            } else if let Some(file) = obj.get("file").cloned() {
                obj.insert("path".to_string(), file);
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

pub fn tool_schema_json() -> String {
    let schema = serde_json::json!({
        "tools": [
            {
                "name": "get_repo_info",
                "args": {},
                "returns": "string"
            },
            {
                "name": "list_files",
                "args": {"globs": "string[]?", "max_results": "number?"},
                "returns": "string[]"
            },
            {
                "name": "read_file",
                "args": {"path": "string", "max_bytes": "number?", "line_range": "[number,number]?"},
                "returns": "{path,content,truncated}",
                "notes": "Path aliases accepted: path, file, filepath."
            },
            {
                "name": "search_rg",
                "args": {"query": "string", "globs": "string[]?", "max_results": "number?"},
                "returns": "{matches:[{path,line,snippet}]}"
            },
            {
                "name": "write_file",
                "args": {"path": "string", "content": "string"},
                "returns": "success",
                "notes": "Path aliases accepted: path, file, filepath."
            },
            {
                "name": "run_command",
                "args": {"cmd": "string", "timeout_ms": "number?"},
                "returns": "{exit_code,stdout,stderr}",
                "notes": "Alias: Bash. Supports common dev/search/build CLI commands with per-segment allowlist checks."
            },
            {
                "name": "capture_screenshot",
                "args": {"url": "string", "delay_ms": "number?"},
                "returns": "{url,base64}"
            }
        ]
    });
    schema.to_string()
}
