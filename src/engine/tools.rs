use crate::agent_manager::AgentManager;
use crate::config::{AgentPolicy, AgentPolicyCapability};
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

/// Check if a hostname falls in the RFC 1918 172.16.0.0/12 range (172.16.x.x – 172.31.x.x).
fn is_rfc1918_172(host: &str) -> bool {
    if let Some(rest) = host.strip_prefix("172.") {
        if let Some(second_octet) = rest.split('.').next() {
            if let Ok(n) = second_octet.parse::<u8>() {
                return (16..=31).contains(&n);
            }
        }
    }
    false
}

#[derive(Debug)]
pub struct ToolCall {
    pub tool: String,
    pub args: Value,
}

#[derive(Debug, Serialize)]
pub enum ToolResult {
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
    WebSearchResults {
        query: String,
        results: Vec<super::web_search::WebSearchResult>,
    },
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
    delegation_depth: usize,
    max_delegation_depth: usize,
    run_id: Option<String>,
    agent_policy: Option<AgentPolicy>,
    memory_dir: Option<PathBuf>,
}

impl Tools {
    pub fn new(root: PathBuf) -> Result<Self> {
        Ok(Self {
            root,
            manager: None,
            agent_id: None,
            delegation_depth: 0,
            max_delegation_depth: 2,
            run_id: None,
            agent_policy: None,
            memory_dir: None,
        })
    }

    pub fn set_context(
        &mut self,
        manager: Arc<AgentManager>,
        agent_id: String,
    ) {
        self.manager = Some(manager);
        self.agent_id = Some(agent_id);
    }

    pub fn set_delegation_depth(&mut self, depth: usize) {
        self.delegation_depth = depth;
    }

    pub fn set_max_delegation_depth(&mut self, max_depth: usize) {
        self.max_delegation_depth = max_depth;
    }

    pub fn delegation_depth(&self) -> usize {
        self.delegation_depth
    }

    pub fn max_delegation_depth(&self) -> usize {
        self.max_delegation_depth
    }

    pub fn set_policy(&mut self, policy: Option<AgentPolicy>) {
        self.agent_policy = policy;
    }

    pub fn set_run_id(&mut self, run_id: Option<String>) {
        self.run_id = run_id;
    }

    pub fn set_memory_dir(&mut self, dir: PathBuf) {
        self.memory_dir = Some(dir);
    }

    pub fn memory_dir(&self) -> Option<&PathBuf> {
        self.memory_dir.as_ref()
    }

    pub fn get_manager(&self) -> Option<Arc<AgentManager>> {
        self.manager.clone()
    }

    pub(crate) fn workspace_root(&self) -> &Path {
        &self.root
    }

    pub fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        let normalized_args = normalize_tool_args(&call.tool, &call.args);
        info!(
            "Executing tool: {} with args: {}",
            call.tool,
            summarize_tool_args(&call.tool, &normalized_args)
        );
        match call.tool.as_str() {
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
            "Edit" => {
                let args: EditFileArgs = serde_json::from_value(normalized_args).map_err(|e| {
                    anyhow::anyhow!(
                        "invalid args for Edit: {}. Expected keys: path|old_string|new_string|replace_all?",
                        e
                    )
                })?;
                self.edit_file(args)
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
            "WebSearch" => {
                let args: WebSearchArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for WebSearch: {}", e))?;
                let max = args.max_results.unwrap_or(5).min(10);
                let results = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(super::web_search::duckduckgo_search(&args.query, max))
                })?;
                Ok(ToolResult::WebSearchResults {
                    query: args.query,
                    results,
                })
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
        // Check if this is a memory path (absolute, outside workspace)
        let abs_path = Path::new(&args.path);
        if abs_path.is_absolute() && self.is_memory_path(abs_path) {
            if abs_path.exists() && abs_path.is_file() {
                return self.do_read_file(&args.path, abs_path, args.max_bytes, args.line_range);
            }
            return Ok(ToolResult::Success(format!(
                "file_not_found: {} - Memory file does not exist yet. Use Write to create it.",
                args.path
            )));
        }

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
        // Validate URL to prevent SSRF: only allow http/https and block private IPs.
        let parsed_url: reqwest::Url = args
            .url
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;
        match parsed_url.scheme() {
            "http" | "https" => {}
            scheme => anyhow::bail!("Disallowed URL scheme: {}", scheme),
        }
        if let Some(host) = parsed_url.host_str() {
            let is_private = host == "localhost"
                || host == "127.0.0.1"
                || host == "::1"
                || host == "0.0.0.0"
                || host.starts_with("10.")
                || is_rfc1918_172(host)
                || host.starts_with("192.168.")
                || host.ends_with(".local");
            if is_private {
                anyhow::bail!(
                    "Disallowed URL host (private/internal address): {}",
                    host
                );
            }
        }

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

    fn enforce_write_access(&self, rel: &str) -> Result<()> {
        if let (Some(manager), Some(agent_id)) = (&self.manager, &self.agent_id) {
            // 1. Check work_globs
            let allowed = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(async { manager.is_path_allowed(&self.root, agent_id, rel).await })
            });

            if !allowed {
                anyhow::bail!(
                    "Path {} is outside the allowed WorkScope for agent {}",
                    rel, agent_id
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

            // Live working-place map for active-path UI (in-memory source of truth).
            if self.run_id.is_some() {
                let repo_path = self.root.to_string_lossy().to_string();
                let run_id = self.run_id.clone();
                let rel_for_map = rel.to_string();
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
        Ok(())
    }

    /// Check if an absolute path is inside the memory directory.
    fn is_memory_path(&self, path: &Path) -> bool {
        if let Some(ref mem_dir) = self.memory_dir {
            if let (Ok(canonical_path), Ok(canonical_mem)) = (
                path.canonicalize().or_else(|_| {
                    // File may not exist yet — canonicalize parent
                    path.parent()
                        .and_then(|p| p.canonicalize().ok())
                        .map(|p| p.join(path.file_name().unwrap_or_default()))
                        .ok_or(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "no parent",
                        ))
                }),
                mem_dir
                    .canonicalize()
                    .or_else(|_| Ok::<PathBuf, std::io::Error>(mem_dir.clone())),
            ) {
                return canonical_path.starts_with(&canonical_mem);
            }
        }
        false
    }

    fn write_file(&self, args: WriteFileArgs) -> Result<ToolResult> {
        let abs_path = Path::new(&args.path);

        // Check if this is a memory path (absolute, outside workspace)
        if abs_path.is_absolute() && self.is_memory_path(abs_path) {
            if let Some(parent) = abs_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if abs_path.exists() {
                let existing = fs::read_to_string(abs_path).unwrap_or_default();
                if existing == args.content {
                    return Ok(ToolResult::Success(format!(
                        "File unchanged (content identical): {}",
                        args.path
                    )));
                }
            }
            let bytes = args.content.len();
            fs::write(abs_path, &args.content)?;
            return Ok(ToolResult::Success(format!(
                "Memory file written: {} ({} bytes)",
                args.path, bytes
            )));
        }

        let rel = sanitize_rel_path(&self.root, &args.path)?;
        let path = self.root.join(&rel);
        let new_content = args.content;
        self.enforce_write_access(&rel)?;

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

    fn edit_file(&self, args: EditFileArgs) -> Result<ToolResult> {
        if args.old_string.is_empty() {
            anyhow::bail!("old_string must not be empty");
        }

        let abs_path = Path::new(&args.path);

        // Check if this is a memory path (absolute, outside workspace)
        if abs_path.is_absolute() && self.is_memory_path(abs_path) {
            if !abs_path.exists() {
                anyhow::bail!("file not found: {}", args.path);
            }
            let existing = fs::read_to_string(abs_path)?;
            let match_count = existing.matches(&args.old_string).count();
            if match_count == 0 {
                anyhow::bail!("old_string was not found in file: {}", args.path);
            }
            let replace_all = args.replace_all.unwrap_or(false);
            if match_count > 1 && !replace_all {
                anyhow::bail!(
                    "old_string matched {} locations in {}. Provide a more specific old_string or set replace_all=true.",
                    match_count,
                    args.path
                );
            }
            let updated = if replace_all {
                existing.replace(&args.old_string, &args.new_string)
            } else {
                existing.replacen(&args.old_string, &args.new_string, 1)
            };
            if updated == existing {
                return Ok(ToolResult::Success(format!(
                    "File unchanged (no effective edit): {}",
                    args.path
                )));
            }
            fs::write(abs_path, updated)?;
            let replaced = if replace_all { match_count } else { 1 };
            return Ok(ToolResult::Success(format!(
                "Edited memory file: {} ({} replacement{})",
                args.path,
                replaced,
                if replaced == 1 { "" } else { "s" }
            )));
        }

        let rel = sanitize_rel_path(&self.root, &args.path)?;
        let path = self.root.join(&rel);
        if !path.exists() {
            anyhow::bail!("file not found: {}", rel);
        }
        if path.is_dir() {
            anyhow::bail!(
                "path '{}' is a directory. Use Glob to enumerate files, then Edit with an exact file path.",
                rel
            );
        }

        self.enforce_write_access(&rel)?;

        let existing = fs::read_to_string(&path)?;
        let match_count = existing.matches(&args.old_string).count();
        if match_count == 0 {
            anyhow::bail!("old_string was not found in file: {}", rel);
        }

        let replace_all = args.replace_all.unwrap_or(false);
        if match_count > 1 && !replace_all {
            anyhow::bail!(
                "old_string matched {} locations in {}. Provide a more specific old_string or set replace_all=true.",
                match_count,
                rel
            );
        }

        let updated = if replace_all {
            existing.replace(&args.old_string, &args.new_string)
        } else {
            existing.replacen(&args.old_string, &args.new_string, 1)
        };

        if updated == existing {
            return Ok(ToolResult::Success(format!(
                "File unchanged (no effective edit): {}",
                rel
            )));
        }

        fs::write(&path, updated)?;
        let replaced = if replace_all { match_count } else { 1 };
        Ok(ToolResult::Success(format!(
            "Edited file: {} ({} replacement{})",
            rel,
            replaced,
            if replaced == 1 { "" } else { "s" }
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

    /// Validate delegation policy/depth/target without executing.
    /// Returns the manager and caller agent id on success.
    pub(crate) fn validate_delegation(
        &self,
        args: &DelegateToAgentArgs,
    ) -> Result<(Arc<AgentManager>, String)> {
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Delegation requires AgentManager context"))?;
        let caller_id = self
            .agent_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Delegation requires caller agent id"))?;

        if self.delegation_depth >= self.max_delegation_depth {
            anyhow::bail!(
                "Delegation denied: max delegation depth ({}) reached",
                self.max_delegation_depth
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
        if !policy.allows_delegate_target(&args.target_agent_id) {
            let allowed = if policy.delegate_targets.is_empty() {
                "(none)".to_string()
            } else {
                policy.delegate_targets.join(", ")
            };
            anyhow::bail!(
                "Delegation denied: target '{}' is not allowed by policy for '{}'. Allowed: {}",
                args.target_agent_id,
                caller_id,
                allowed
            );
        }

        Ok((manager.clone(), caller_id))
    }

    fn delegate_to_agent(&self, args: DelegateToAgentArgs) -> Result<ToolResult> {
        let (manager, caller_id) = self.validate_delegation(&args)?;
        let delegation_depth = self.delegation_depth;
        let max_delegation_depth = self.max_delegation_depth;
        let ws_root = self.root.clone();
        let parent_run_id = self.run_id.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(run_delegation(
                manager,
                ws_root,
                caller_id,
                args.target_agent_id,
                args.task,
                parent_run_id,
                delegation_depth,
                max_delegation_depth,
            ))
        })
    }
}

/// Execute a single delegation on a fresh, ephemeral engine.
///
/// This is a standalone async function (not a method) so it can be spawned onto
/// a `JoinSet` for parallel execution.  Each call creates its own `AgentEngine`
/// via `AgentManager::spawn_delegation_engine`, runs the agent loop, and drops
/// the engine when done.
pub(crate) async fn run_delegation(
    manager: Arc<AgentManager>,
    ws_root: PathBuf,
    caller_id: String,
    target_agent_id: String,
    task: String,
    parent_run_id: Option<String>,
    delegation_depth: usize,
    max_delegation_depth: usize,
) -> Result<ToolResult> {
    let run_id = manager
        .begin_agent_run(
            &ws_root,
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

    manager
        .send_event(crate::agent_manager::AgentEvent::SubagentSpawned {
            parent_id: caller_id.clone(),
            subagent_id: target_agent_id.clone(),
            task: task.clone(),
        })
        .await;

    let engine_result = manager
        .spawn_delegation_engine(&ws_root, &target_agent_id)
        .await;
    let mut engine = match engine_result {
        Ok(e) => e,
        Err(err) => {
            let _ = manager
                .finish_agent_run(
                    &run_id,
                    crate::project_store::AgentRunStatus::Failed,
                    Some(err.to_string()),
                )
                .await;
            return Err(err);
        }
    };

    engine.set_parent_agent(Some(caller_id.clone()));
    engine.set_delegation_depth(delegation_depth + 1, max_delegation_depth);
    engine.set_run_id(Some(run_id.clone()));
    engine.set_task(task);

    let run_result = engine.run_agent_loop(None).await;
    // Engine is dropped here — no cleanup of cached state needed.

    let (outcome, status, detail) = match run_result {
        Ok(outcome) => (outcome, crate::project_store::AgentRunStatus::Completed, None),
        Err(err) => {
            let msg = err.to_string();
            let status = if msg.to_lowercase().contains("cancel") {
                crate::project_store::AgentRunStatus::Cancelled
            } else {
                crate::project_store::AgentRunStatus::Failed
            };
            let _ = manager
                .finish_agent_run(&run_id, status, Some(msg.clone()))
                .await;
            return Err(err);
        }
    };
    let _ = manager.finish_agent_run(&run_id, status, detail).await;

    manager
        .send_event(crate::agent_manager::AgentEvent::SubagentResult {
            parent_id: caller_id,
            subagent_id: target_agent_id,
            outcome: outcome.clone(),
        })
        .await;

    Ok(ToolResult::AgentOutcome(outcome))
}

#[derive(Debug, Deserialize)]
struct WriteFileArgs {
    #[serde(alias = "file", alias = "filepath")]
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct EditFileArgs {
    #[serde(alias = "file", alias = "filepath")]
    path: String,
    #[serde(
        alias = "old",
        alias = "old_text",
        alias = "oldText",
        alias = "search",
        alias = "from"
    )]
    old_string: String,
    #[serde(
        alias = "new",
        alias = "new_text",
        alias = "newText",
        alias = "replace",
        alias = "to"
    )]
    new_string: String,
    #[serde(alias = "all")]
    replace_all: Option<bool>,
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
pub(crate) struct DelegateToAgentArgs {
    pub(crate) target_agent_id: String,
    pub(crate) task: String,
}

#[derive(Debug, Deserialize)]
struct WebSearchArgs {
    #[serde(alias = "q")]
    query: String,
    max_results: Option<usize>,
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

pub(crate) fn summarize_tool_args(tool: &str, args: &Value) -> String {
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
            "Edit" => {
                for key in ["old_string", "new_string", "old", "new", "old_text", "new_text", "oldText", "newText", "search", "replace", "from", "to"] {
                    if let Some(content) = obj.get(key).and_then(|v| v.as_str()) {
                        let byte_len = content.len();
                        let line_count = content.lines().count();
                        obj.insert(
                            key.to_string(),
                            serde_json::json!(format!(
                                "<omitted:{} bytes, {} lines>",
                                byte_len, line_count
                            )),
                        );
                    }
                }
            }
            "Bash" => {
                if let Some(cmd) = obj.get("cmd").and_then(|v| v.as_str()) {
                    let preview = if cmd.len() > 160 {
                        // Find a char boundary at or before 160 to avoid UTF-8 panic.
                        let end = cmd
                            .char_indices()
                            .map(|(i, _)| i)
                            .take_while(|&i| i <= 160)
                            .last()
                            .unwrap_or(0);
                        format!("{}... (truncated, {} chars)", &cmd[..end], cmd.len())
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

pub(crate) fn normalize_tool_args(tool: &str, args: &Value) -> Value {
    let mut normalized = args.clone();
    if let Some(obj) = normalized.as_object_mut() {
        if matches!(tool, "Bash") && !obj.contains_key("cmd") {
            if let Some(command) = obj.get("command").cloned() {
                obj.insert("cmd".to_string(), command);
            }
        }

        if matches!(tool, "Read" | "Write" | "Edit") && !obj.contains_key("path") {
            if let Some(fp) = obj.get("filepath").cloned() {
                obj.insert("path".to_string(), fp);
            } else if let Some(file) = obj.get("file").cloned() {
                obj.insert("path".to_string(), file);
            }
        }

        if matches!(tool, "Edit") {
            if !obj.contains_key("old_string") {
                if let Some(v) = obj.get("old").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("old_text").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("oldText").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("search").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("from").cloned() {
                    obj.insert("old_string".to_string(), v);
                }
            }
            if !obj.contains_key("new_string") {
                if let Some(v) = obj.get("new").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("new_text").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("newText").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("replace").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("to").cloned() {
                    obj.insert("new_string".to_string(), v);
                }
            }
            if !obj.contains_key("replace_all") {
                if let Some(v) = obj.get("all").cloned() {
                    obj.insert("replace_all".to_string(), v);
                }
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

pub(crate) fn validate_shell_command(cmd: &str) -> Result<()> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty command");
    }

    // Disallow common shell injection patterns.
    for banned in ["$(", "`", "\n", "\r", "<(", ">("] {
        if trimmed.contains(banned) {
            anyhow::bail!("command contains disallowed shell construct: {}", banned);
        }
    }
    // Block output redirection (but allow `>` inside grep patterns etc. via `--`).
    // We only block bare `>` or `>>` that appear as shell operators.
    for op in [" > ", " >> ", "\t>\t", "\t>>\t", " >|"] {
        if trimmed.contains(op) {
            anyhow::bail!("command contains disallowed shell redirection");
        }
    }
    // Block input redirection `< file`.
    if trimmed.contains(" < ") {
        anyhow::bail!("command contains disallowed shell redirection");
    }

    let allowed: HashSet<&str> = [
        "ls", "pwd", "cat", "head", "tail", "wc", "cut", "sort", "uniq", "tr", "sed", "awk",
        "find", "fd", "rg", "grep", "git", "cargo", "rustc", "npm", "pnpm", "yarn", "node",
        "python", "python3", "pip", "pip3", "pytest", "go", "make", "just",
        "bash", "sh", "curl", "jq",
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

pub fn canonical_tool_name(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "Glob" => "Glob",
        "Read" => "Read",
        "Grep" => "Grep",
        "Write" => "Write",
        "Edit" => "Edit",
        "Bash" => "Bash",
        "capture_screenshot" => "capture_screenshot",
        "lock_paths" => "lock_paths",
        "unlock_paths" => "unlock_paths",
        "delegate_to_agent" => "delegate_to_agent",
        "WebSearch" | "web_search" => "WebSearch",
        _ => return None,
    })
}

pub(crate) fn full_tool_schema_entries() -> Vec<Value> {
    vec![
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
            "name": "Edit",
            "args": {"path": "string", "old_string": "string", "new_string": "string", "replace_all": "boolean?"},
            "returns": "success",
            "notes": "Applies an exact string replacement. Path aliases accepted: path, file, filepath."
        }),
        serde_json::json!({
            "name": "Bash",
            "args": {"cmd": "string", "timeout_ms": "number?"},
            "returns": "{exit_code,stdout,stderr}",
            "notes": "Runs allowlisted dev/search/build shell commands with per-segment validation. Command alias accepted: command."
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
            "notes": "Delegates a task to another agent. Subject to max delegation depth."
        }),
        serde_json::json!({
            "name": "WebSearch",
            "args": {"query": "string", "max_results": "number?"},
            "returns": "{results:[{title,url,snippet}]}",
            "notes": "Search the web via DuckDuckGo. Default 5 results, max 10."
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
