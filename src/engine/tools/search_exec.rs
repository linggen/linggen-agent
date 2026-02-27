use super::tool_helpers::{build_globset, to_rel_string, validate_shell_command};
use super::{SearchMatch, ToolResult, Tools};
use anyhow::Result;
use grep::regex::RegexMatcher;
use grep::searcher::sinks::UTF8;
use grep::searcher::Searcher;
use ignore::WalkBuilder;
use serde::Deserialize;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tracing::warn;

#[derive(Debug, Deserialize)]
pub(super) struct SearchArgs {
    pub(super) query: String,
    pub(super) globs: Option<Vec<String>>,
    pub(super) max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RunCommandArgs {
    pub(super) cmd: String,
    pub(super) timeout_ms: Option<u64>,
}

impl Tools {
    pub(super) fn search_rg(&self, args: SearchArgs) -> Result<ToolResult> {
        let globset = build_globset(args.globs.as_deref())?;
        let max_results = args.max_results.unwrap_or(200);

        let matcher = RegexMatcher::new(&args.query).or_else(|_| {
            let escaped = regex::escape(&args.query);
            RegexMatcher::new(&escaped)
                .map_err(|e| anyhow::anyhow!("invalid search pattern '{}': {}", args.query, e))
        })?;

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

    pub(super) fn run_command(&self, args: RunCommandArgs) -> Result<ToolResult> {
        use std::io::BufRead;

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

        // Take stdout/stderr handles for line-by-line streaming.
        let child_stdout = child.stdout.take();
        let child_stderr = child.stderr.take();
        let progress_tx = self.progress_tx.clone();

        // Spawn reader threads that accumulate output and optionally send
        // progress lines through the channel.
        let stdout_handle = std::thread::spawn({
            let tx = progress_tx.clone();
            move || {
                let mut acc = String::new();
                if let Some(stdout) = child_stdout {
                    let reader = std::io::BufReader::new(stdout);
                    for line in reader.lines() {
                        match line {
                            Ok(l) => {
                                if let Some(tx) = &tx {
                                    let _ = tx.send(("Bash".to_string(), "stdout".to_string(), l.clone()));
                                }
                                acc.push_str(&l);
                                acc.push('\n');
                            }
                            Err(_) => break,
                        }
                    }
                }
                acc
            }
        });
        let stderr_handle = std::thread::spawn({
            let tx = progress_tx;
            move || {
                let mut acc = String::new();
                if let Some(stderr) = child_stderr {
                    let reader = std::io::BufReader::new(stderr);
                    for line in reader.lines() {
                        match line {
                            Ok(l) => {
                                if let Some(tx) = &tx {
                                    let _ = tx.send(("Bash".to_string(), "stderr".to_string(), l.clone()));
                                }
                                acc.push_str(&l);
                                acc.push('\n');
                            }
                            Err(_) => break,
                        }
                    }
                }
                acc
            }
        });

        // Wait for the process with timeout.
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

        let exit_status = child.wait()?;

        // Join reader threads and collect accumulated output.
        let stdout = stdout_handle.join().unwrap_or_else(|_| {
            warn!("stdout reader thread panicked for command");
            String::new()
        });
        let mut stderr = stderr_handle.join().unwrap_or_else(|_| {
            warn!("stderr reader thread panicked for command");
            "linggen-agent: internal error reading command output\n".to_string()
        });

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
            exit_code: exit_status.code(),
            stdout,
            stderr,
        })
    }
}
