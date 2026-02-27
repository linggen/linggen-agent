use crate::server::UiSseMessage;
use crate::tui_client::TuiClient;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;
use tokio::sync::mpsc;

use super::display::*;
use super::render;

// Compact box-drawing logo (Calvin S figlet style)
const LOGO_1: &str = "╻  ╻┏┓╻┏━╸┏━╸┏━╸┏┓╻";
const LOGO_2: &str = "┃  ┃┃┗┫┃╺┓┃╺┓┣╸ ┃┗┫";
const LOGO_3: &str = "┗━╸╹╹ ╹┗━┛┗━┛┗━╸╹ ╹";

/// SSE connection status for the TUI.
pub enum ConnectionStatus {
    Connected,
    Disconnected,
}

/// In-progress tool group accumulator.
pub struct ActiveToolGroup {
    pub agent_id: String,
    pub steps: Vec<ToolStep>,
}

pub struct App {
    pub client: Arc<TuiClient>,
    pub sse_rx: mpsc::UnboundedReceiver<UiSseMessage>,
    pub input: String,
    pub banner: Vec<Line<'static>>,
    pub blocks: Vec<DisplayBlock>,
    pub project_root: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    /// Shared slot for spawned tasks to write back an auto-created session_id.
    pub session_id_slot: Arc<StdMutex<Option<String>>>,
    // Status bar
    pub status_state: String,
    pub status_tool: Option<String>,
    pub status_agent: String,
    // Token streaming
    pub streaming_buffer: String,
    pub is_streaming: bool,
    // Tool step tracking
    pub active_tool_group: Option<ActiveToolGroup>,
    // Subagent tracking
    pub active_subagents: HashMap<String, SubagentEntry>,
    /// Maps subagent_id → parent_agent_id for routing subagent activity.
    pub subagent_parent_map: HashMap<String, String>,
    // Dedup own messages echoed back via SSE
    pub pending_user_messages: VecDeque<String>,
    // Verbose/compact display mode
    pub verbose_mode: bool,
    /// Scroll offset from the bottom. 0 = follow tail, >0 = scrolled up.
    pub scroll_offset: usize,
    // Metrics tracking per agent
    pub last_context_tokens: HashMap<String, usize>,
    pub last_token_limit: HashMap<String, usize>,
    pub run_start_ts: HashMap<String, Instant>,
    // Interactive prompt (e.g., plan approval)
    pub prompt: Option<InteractivePrompt>,
    /// Base64-encoded images pending to be sent with the next message.
    pub pending_images: Vec<String>,
    /// Question ID for a pending AskUser/permission prompt.
    pub pending_ask_user_id: Option<String>,
    /// SSE connection status (for reconnection indicator).
    pub connection_status: ConnectionStatus,
    /// Last seen SSE sequence number for dedup. Reset on reconnection.
    pub last_seq: u64,
}

/// An interactive prompt shown below the input for user selection.
pub struct InteractivePrompt {
    pub options: Vec<String>,
    pub selected: usize,
}

impl App {
    pub fn new(
        client: Arc<TuiClient>,
        sse_rx: mpsc::UnboundedReceiver<UiSseMessage>,
        project_root: String,
        port: u16,
    ) -> Self {
        let version = env!("CARGO_PKG_VERSION");
        let web_url = format!("http://localhost:{port}");

        let logo = Style::default().fg(Color::Cyan);
        let dim = Style::default().fg(Color::DarkGray);
        let label = Style::default().fg(Color::DarkGray);
        let val = Style::default().fg(Color::White);
        let accent = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let sep = Style::default().fg(Color::DarkGray);

        let banner = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  ", dim),
                Span::styled(LOGO_1, logo),
            ]),
            Line::from(vec![
                Span::styled("  ", dim),
                Span::styled(LOGO_2, logo),
                Span::styled(format!("   v{version}"), dim),
            ]),
            Line::from(vec![
                Span::styled("  ", dim),
                Span::styled(LOGO_3, logo),
                Span::styled("   AI Coding Agent", accent),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Web UI     ", label),
                Span::styled(web_url, val),
            ]),
            Line::from(vec![
                Span::styled("  Workspace  ", label),
                Span::styled(project_root.clone(), val),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  /help  /agent <name>  @agent msg  /quit",
                dim,
            )]),
            Line::from(Span::styled(
                "  ──────────────────────────────────────────",
                sep,
            )),
            Line::from(""),
        ];

        Self {
            client,
            sse_rx,
            input: String::new(),
            banner,
            blocks: Vec::new(),
            project_root,
            agent_id: "ling".to_string(),
            session_id: None,
            session_id_slot: Arc::new(StdMutex::new(None)),
            status_state: "idle".to_string(),
            status_tool: None,
            status_agent: "ling".to_string(),
            streaming_buffer: String::new(),
            is_streaming: false,
            active_tool_group: None,
            active_subagents: HashMap::new(),
            subagent_parent_map: HashMap::new(),
            pending_user_messages: VecDeque::new(),
            verbose_mode: false,
            scroll_offset: 0,
            last_context_tokens: HashMap::new(),
            last_token_limit: HashMap::new(),
            run_start_ts: HashMap::new(),
            prompt: None,
            pending_images: Vec::new(),
            pending_ask_user_id: None,
            connection_status: ConnectionStatus::Connected,
            last_seq: 0,
        }
    }

    fn push_user(&mut self, text: &str) {
        self.push_user_with_images(text, 0);
    }

    fn push_user_with_images(&mut self, text: &str, image_count: usize) {
        self.blocks.push(DisplayBlock::UserMessage {
            text: text.to_string(),
            image_count,
        });
    }

    fn push_agent(&mut self, agent: &str, text: &str) {
        self.blocks.push(DisplayBlock::AgentMessage {
            agent_id: agent.to_string(),
            text: text.to_string(),
        });
    }

    fn context_usage_pct(&self) -> usize {
        let tokens = self
            .last_context_tokens
            .get(&self.agent_id)
            .copied()
            .unwrap_or(0);
        let limit = self
            .last_token_limit
            .get(&self.agent_id)
            .copied()
            .unwrap_or(200_000);
        if limit == 0 {
            return 0;
        }
        (tokens * 100) / limit
    }

    fn push_system(&mut self, text: &str) {
        self.blocks.push(DisplayBlock::SystemMessage {
            text: text.to_string(),
        });
    }

    /// Trigger a full state resync from the server via REST APIs.
    /// Spawns a fire-and-forget background task; errors are logged.
    fn trigger_resync(&self) {
        let client = self.client.clone();
        let project_root = self.project_root.clone();
        let session_id = self.session_id.clone();
        tokio::spawn(async move {
            if let Err(e) = client
                .fetch_workspace_state(&project_root, session_id.as_deref())
                .await
            {
                tracing::debug!("Resync workspace state failed: {}", e);
            }
            if let Err(e) = client
                .fetch_agent_runs(&project_root, session_id.as_deref())
                .await
            {
                tracing::debug!("Resync agent runs failed: {}", e);
            }
        });
    }

    /// Grab a PNG image from the system clipboard and return base64.
    /// macOS: uses osascript, Linux: tries wl-paste then xclip.
    fn grab_clipboard_image() -> Result<String> {
        use base64::Engine;
        let tmp_path = std::env::temp_dir().join("linggen_clipboard_img.png");

        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("osascript")
                .arg("-e")
                .arg(format!(
                    "set imgData to the clipboard as «class PNGf»\n\
                     set fp to open for access POSIX file \"{}\" with write permission\n\
                     write imgData to fp\n\
                     close access fp",
                    tmp_path.display()
                ))
                .output()
                .map_err(|e| anyhow::anyhow!("osascript failed: {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("clipboard has no image ({})", stderr.trim());
            }
        }

        #[cfg(target_os = "linux")]
        {
            let wl_ok = std::process::Command::new("wl-paste")
                .args(["--type", "image/png"])
                .stdout(std::fs::File::create(&tmp_path)?)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !wl_ok {
                let xclip_ok = std::process::Command::new("xclip")
                    .args(["-selection", "clipboard", "-target", "image/png", "-o"])
                    .stdout(std::fs::File::create(&tmp_path)?)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if !xclip_ok {
                    anyhow::bail!("no image in clipboard (tried wl-paste and xclip)");
                }
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            anyhow::bail!("clipboard image paste not supported on this platform");
        }

        if !tmp_path.exists() {
            anyhow::bail!("clipboard image file was not created");
        }
        let data = std::fs::read(&tmp_path)?;
        let _ = std::fs::remove_file(&tmp_path);
        if data.is_empty() {
            anyhow::bail!("clipboard image is empty");
        }
        Ok(base64::engine::general_purpose::STANDARD.encode(&data))
    }

    /// Try pasting a clipboard image and push a system status message.
    fn paste_clipboard_image(&mut self) {
        match Self::grab_clipboard_image() {
            Ok(base64) => {
                self.pending_images.push(base64);
                let count = self.pending_images.len();
                self.push_system(&format!("Image pasted from clipboard ({count} pending)"));
            }
            Err(e) => {
                self.push_system(&format!("No image in clipboard: {e}"));
            }
        }
    }

    /// Handle /image and /paste commands.
    fn handle_image_command(&mut self, line: &str) {
        if line == "/paste" {
            self.paste_clipboard_image();
            return;
        }
        if line == "/image" {
            self.push_system("Usage: /image <file_path>  — attach an image file");
            self.push_system("       Ctrl+V             — paste image from clipboard");
            self.push_system(&format!("  {} image(s) pending", self.pending_images.len()));
            return;
        }
        if line == "/image clear" {
            self.pending_images.clear();
            self.push_system("Cleared all pending images.");
            return;
        }
        // /image <path>
        let path = line.strip_prefix("/image ").unwrap_or("").trim();
        if path.is_empty() {
            self.push_system("Usage: /image <file_path>");
            return;
        }
        match Self::load_image_file(path) {
            Ok(base64) => {
                self.pending_images.push(base64);
                let count = self.pending_images.len();
                self.push_system(&format!("Image attached: {path} ({count} pending)"));
            }
            Err(e) => {
                self.push_system(&format!("Failed to load image: {e}"));
            }
        }
    }

    /// Load an image file from disk and return its base64-encoded content.
    fn load_image_file(path: &str) -> Result<String> {
        use base64::Engine;
        let expanded = if path.starts_with('~') {
            if let Some(home) = dirs::home_dir() {
                home.join(path.strip_prefix("~/").unwrap_or(path))
            } else {
                std::path::PathBuf::from(path)
            }
        } else {
            std::path::PathBuf::from(path)
        };
        let data = std::fs::read(&expanded)
            .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", expanded.display(), e))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(&data))
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // When an interactive prompt is active, handle its keys first.
        if let Some(prompt) = &mut self.prompt {
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(true);
                }
                KeyCode::Up | KeyCode::BackTab => {
                    prompt.selected = prompt
                        .selected
                        .checked_sub(1)
                        .unwrap_or(prompt.options.len() - 1);
                }
                KeyCode::Down | KeyCode::Tab => {
                    prompt.selected = (prompt.selected + 1) % prompt.options.len();
                }
                KeyCode::Enter => {
                    let choice = prompt.options[prompt.selected].clone();
                    self.prompt = None;
                    self.handle_prompt_choice(&choice)?;
                }
                KeyCode::Esc => {
                    self.prompt = None;
                }
                _ => {}
            }
            return Ok(false);
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(true);
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.paste_clipboard_image();
            }
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.verbose_mode = !self.verbose_mode;
                for block in &mut self.blocks {
                    if let DisplayBlock::ToolGroup { collapsed, .. } = block {
                        *collapsed = !self.verbose_mode;
                    }
                }
            }
            KeyCode::Up => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            KeyCode::Down => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(20);
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(20);
            }
            KeyCode::Char(ch) => {
                self.scroll_offset = 0;
                self.input.push(ch);
            }
            KeyCode::Backspace => {
                self.scroll_offset = 0;
                self.input.pop();
            }
            KeyCode::Enter => {
                let line = self.input.trim().to_string();
                self.input.clear();
                if !line.is_empty() {
                    self.scroll_offset = 0;
                    if self.handle_command(line)? {
                        return Ok(true);
                    }
                }
            }
            KeyCode::Esc => {
                self.input.clear();
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_prompt_choice(&mut self, choice: &str) -> Result<bool> {
        // Handle AskUser/permission prompts.
        if let Some(question_id) = self.pending_ask_user_id.take() {
            self.push_system(&format!("Selected: {}", choice));
            let client = self.client.clone();
            let selected = choice.to_string();
            tokio::spawn(async move {
                if let Err(e) = client.respond_ask_user(&question_id, &selected).await {
                    tracing::warn!("AskUser response failed: {}", e);
                }
            });
            return Ok(false);
        }

        if choice.starts_with("Start (new session") {
            self.push_system("Clearing context and starting plan execution...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .approve_plan(&project_root, &agent_id, session_id.as_deref(), true)
                    .await
                {
                    tracing::warn!("Plan approve failed: {}", e);
                }
            });
        } else if choice == "Start (continue session)" {
            self.push_system("Starting plan execution...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .approve_plan(&project_root, &agent_id, session_id.as_deref(), false)
                    .await
                {
                    tracing::warn!("Plan approve failed: {}", e);
                }
            });
        } else if choice == "Reject plan" {
            self.push_system("Rejecting plan...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .reject_plan(&project_root, &agent_id, session_id.as_deref())
                    .await
                {
                    tracing::warn!("Plan reject failed: {}", e);
                }
            });
        } else if choice == "Give feedback" {
            self.push_system("Type your feedback and press Enter:");
            self.input = "/plan feedback ".to_string();
        }
        Ok(false)
    }

    fn handle_command(&mut self, line: String) -> Result<bool> {
        if line == "/quit" || line == "/exit" {
            return Ok(true);
        }

        if line == "/help" {
            self.push_system("Commands:");
            self.push_system("  /agent <name>     switch default agent");
            self.push_system("  @agent message    send to specific agent");
            self.push_system("  /plan <task>      ask agent to create a plan (read-only)");
            self.push_system("  /plan approve     approve and execute the pending plan");
            self.push_system("  /plan reject      reject the pending plan");
            self.push_system("  /image <path>     attach an image file");
            self.push_system("  /image clear      remove all pending images");
            self.push_system("  /paste            paste image from clipboard");
            self.push_system("  /quit, /exit      exit");
            self.push_system("  <text>            send message to current agent");
            self.push_system("  Ctrl+V            paste image from clipboard");
            self.push_system("  Ctrl+O            toggle verbose/compact tool display");
            self.push_system("  ↑/↓, scroll       scroll output");
            self.push_system("  PgUp/PgDn         scroll output (fast)");
            return Ok(false);
        }

        // Plan commands
        if line == "/plan" {
            self.push_system("Usage: /plan <task>  — create a plan");
            self.push_system("       /plan approve — approve and execute");
            self.push_system("       /plan reject  — reject the plan");
            return Ok(false);
        }
        if line == "/plan approve" {
            self.push_system("Approving plan...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .approve_plan(&project_root, &agent_id, session_id.as_deref(), false)
                    .await
                {
                    tracing::warn!("Plan approve failed: {}", e);
                }
            });
            return Ok(false);
        }
        if line == "/plan reject" {
            self.push_system("Rejecting plan...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .reject_plan(&project_root, &agent_id, session_id.as_deref())
                    .await
                {
                    tracing::warn!("Plan reject failed: {}", e);
                }
            });
            return Ok(false);
        }
        if line.starts_with("/plan ") {
            self.push_user(&line);
            self.pending_user_messages.push_back(line.clone());
            self.status_state = "sending".to_string();
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            let slot = self.session_id_slot.clone();
            let msg = line.clone();
            tokio::spawn(async move {
                if let Ok(Some(sid)) = client
                    .send_chat(&project_root, &agent_id, &msg, session_id.as_deref(), None)
                    .await
                {
                    let mut guard = slot.lock().unwrap();
                    if guard.is_none() {
                        *guard = Some(sid);
                    }
                }
            });
            return Ok(false);
        }

        // Image commands
        if line == "/image" || line.starts_with("/image ") || line == "/paste" {
            self.handle_image_command(&line);
            return Ok(false);
        }

        if let Some(rest) = line.strip_prefix("/agent ") {
            let name = rest.trim().to_string();
            self.agent_id = name.clone();
            self.status_agent = name.clone();
            self.push_system(&format!("Switched to agent: {name}"));
            return Ok(false);
        }

        // @agent_id message — one-shot to a specific agent
        let (target_agent, message) = if line.starts_with('@') {
            if let Some(pos) = line[1..].find(' ') {
                let agent = line[1..1 + pos].to_string();
                let msg = line[2 + pos..].trim().to_string();
                (agent, msg)
            } else {
                (self.agent_id.clone(), line)
            }
        } else {
            (self.agent_id.clone(), line)
        };

        let images = std::mem::take(&mut self.pending_images);
        let image_count = images.len();
        self.push_user_with_images(&message, image_count);
        self.pending_user_messages.push_back(message.clone());
        self.status_state = "sending".to_string();

        let client = self.client.clone();
        let project_root = self.project_root.clone();
        let session_id = self.session_id.clone();
        let slot = self.session_id_slot.clone();

        let send_images = if images.is_empty() { None } else { Some(images) };
        tokio::spawn(async move {
            if let Ok(Some(sid)) = client
                .send_chat(
                    &project_root,
                    &target_agent,
                    &message,
                    session_id.as_deref(),
                    send_images,
                )
                .await
            {
                let mut guard = slot.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(sid);
                }
            }
        });

        Ok(false)
    }

    pub fn handle_sse(&mut self, msg: UiSseMessage) {
        // Seq-based dedup: skip events we've already processed.
        // Connection events (synthetic, seq=0) are always allowed through.
        if msg.kind != "connection" && msg.seq > 0 && msg.seq <= self.last_seq {
            return;
        }
        if msg.seq > 0 {
            self.last_seq = msg.seq;
        }

        match msg.kind.as_str() {
            "token" => {
                let text = msg.text.unwrap_or_default();
                let done = msg.phase.as_deref() == Some("done");
                let is_thinking = msg
                    .data
                    .as_ref()
                    .and_then(|d| d.get("thinking"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if !text.is_empty() {
                    self.streaming_buffer.push_str(&text);
                    self.is_streaming = true;
                }
                if done {
                    if is_thinking {
                        self.discard_streaming();
                    } else {
                        self.finalize_streaming();
                    }
                }
            }
            "text_segment" => {
                let agent_id = msg.agent_id.unwrap_or_default();
                let text = msg.text.unwrap_or_default();
                if text.is_empty() {
                    return;
                }
                // Skip subagent text segments
                if self.subagent_parent_map.contains_key(&agent_id.to_lowercase()) {
                    return;
                }
                // Discard any thinking tokens being streamed — text_segment is more reliable
                self.discard_streaming();
                // Finalize any active tool group first → creates interleaving
                self.finalize_tool_group();
                self.push_agent(&agent_id, &text);
            }
            "message" => {
                self.discard_streaming();

                let role = msg
                    .data
                    .as_ref()
                    .and_then(|d| d.get("role"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("assistant");
                if role == "user" {
                    if let Some(text) = &msg.text {
                        if let Some(front) = self.pending_user_messages.front() {
                            if front == text {
                                self.pending_user_messages.pop_front();
                                return;
                            }
                        }
                        if !text.is_empty() {
                            self.push_user(text);
                        }
                    }
                    return;
                }
                // Always finalize any active tool/subagent group when a message
                // event arrives, even if the text is stripped — this ensures the
                // tool group moves from "active" to "collapsed" display.
                self.finalize_tool_group();
                self.finalize_subagent_group();
                if let Some(text) = msg.text {
                    let cleaned = Self::strip_internal_json(&text);
                    if !cleaned.is_empty() {
                        let agent = msg.agent_id.unwrap_or_default();
                        self.push_agent(&agent, &cleaned);
                    }
                }
            }
            "activity" => {
                let status = msg
                    .data
                    .as_ref()
                    .and_then(|d| d.get("status"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("working");
                let phase = msg.phase.as_deref().unwrap_or("");
                let text = msg.text.unwrap_or_default();
                let status_id = msg.id.clone();
                let agent = msg
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| self.status_agent.clone());

                // Route subagent activity to its entry instead of the main tool group
                let parent_id = msg
                    .data
                    .as_ref()
                    .and_then(|d| d.get("parent_id"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let is_subagent = self.subagent_parent_map.contains_key(&agent)
                    || parent_id.is_some();

                if is_subagent {
                    if let Some(pid) = &parent_id {
                        self.subagent_parent_map.entry(agent.clone()).or_insert_with(|| pid.clone());
                    }
                    if let Some(entry) = self.active_subagents.get_mut(&agent) {
                        if status == "calling_tool" {
                            if phase == "doing" {
                                entry.tool_count += 1;
                                let (tool_name, args_summary) = parse_activity_text(&text);
                                entry.tool_steps.push(SubagentToolStep {
                                    tool_name,
                                    args_summary,
                                    status: StepStatus::InProgress,
                                });
                            } else if phase == "done" {
                                if let Some(last) = entry.tool_steps.last_mut() {
                                    let is_failed = text.to_lowercase().contains("failed");
                                    last.status = if is_failed {
                                        StepStatus::Failed
                                    } else {
                                        StepStatus::Done
                                    };
                                }
                            }
                        }
                        // Update current_activity for non-tool statuses (thinking, model_loading)
                        if status == "idle" {
                            entry.current_activity = None;
                        } else if status != "calling_tool" && !text.is_empty() {
                            entry.current_activity = Some(text.clone());
                        }
                    }
                    return;
                }

                // Update status bar
                self.status_state = status.to_string();
                self.status_tool = if text.is_empty()
                    || text.eq_ignore_ascii_case(status)
                {
                    None
                } else {
                    Some(text.clone())
                };
                if let Some(aid) = msg.agent_id {
                    self.status_agent = aid;
                }

                // Track run start time on first activity for an agent
                if !self.run_start_ts.contains_key(&agent) {
                    self.run_start_ts.insert(agent.clone(), Instant::now());
                }

                if status == "calling_tool" {
                    if phase == "doing" {
                        let (tool_name, args_summary) = parse_activity_text(&text);

                        // Dedup: if the active group already has a step with the
                        // same status_id, UPDATE it in-place (the server sends
                        // "Reading file: X" then "Read file: X" with the same id).
                        if let Some(group) = &mut self.active_tool_group {
                            if group.agent_id == agent {
                                if let Some(existing) = group
                                    .steps
                                    .iter_mut()
                                    .find(|s| s.status_id == status_id)
                                {
                                    existing.tool_name = tool_name;
                                    existing.args_summary = args_summary;
                                    return;
                                }
                                // New status_id within the same agent group → new step
                                group.steps.push(ToolStep {
                                    status_id,
                                    tool_name,
                                    args_summary,
                                    status: StepStatus::InProgress,
                                });
                                return;
                            }
                        }

                        // Different agent or no active group → start fresh
                        self.finalize_tool_group();
                        self.active_tool_group = Some(ActiveToolGroup {
                            agent_id: agent,
                            steps: vec![ToolStep {
                                status_id,
                                tool_name,
                                args_summary,
                                status: StepStatus::InProgress,
                            }],
                        });
                    } else if phase == "done" {
                        // Mark the matching step as Done (by status_id, or
                        // fall back to the last in-progress step).
                        if let Some(group) = &mut self.active_tool_group {
                            let is_failed = text.to_lowercase().contains("failed");
                            let new_status = if is_failed {
                                StepStatus::Failed
                            } else {
                                StepStatus::Done
                            };
                            // Find by status_id first, then fall back to last InProgress
                            let idx = group
                                .steps
                                .iter()
                                .position(|s| s.status_id == status_id)
                                .or_else(|| {
                                    group
                                        .steps
                                        .iter()
                                        .rposition(|s| s.status == StepStatus::InProgress)
                                });
                            if let Some(idx) = idx {
                                group.steps[idx].status = new_status;
                            }
                        }
                    }
                } else if status == "idle" {
                    // Only finalize on idle (end of run), not on "thinking"
                    // which happens between tool calls within the same turn.
                    self.finalize_tool_group();
                }
            }
            "run" => {
                // Trigger a full state resync on sync, resync, or outcome phases
                match msg.phase.as_deref() {
                    Some("sync") | Some("resync") | Some("outcome") => {
                        self.trigger_resync();
                    }
                    _ => {}
                }
                if msg.phase.as_deref() == Some("context_usage") {
                    if let Some(data) = &msg.data {
                        let agent_key = data
                            .get("agent_id")
                            .and_then(|v| v.as_str())
                            .or(msg.agent_id.as_deref())
                            .unwrap_or("")
                            .to_string();
                        // Route subagent context to its entry
                        if self.subagent_parent_map.contains_key(&agent_key) {
                            if let Some(tokens) = data.get("estimated_tokens").and_then(|v| v.as_u64()) {
                                if let Some(entry) = self.active_subagents.get_mut(&agent_key) {
                                    entry.estimated_tokens = Some(tokens as usize);
                                }
                            }
                        } else {
                            if let Some(tokens) = data.get("estimated_tokens").and_then(|v| v.as_u64())
                            {
                                self.last_context_tokens
                                    .insert(agent_key.clone(), tokens as usize);
                            }
                            if let Some(limit) = data.get("token_limit").and_then(|v| v.as_u64()) {
                                self.last_token_limit
                                    .insert(agent_key, limit as usize);
                            }
                        }
                    }
                }
                if msg.phase.as_deref() == Some("subagent_spawned") {
                    if let Some(data) = &msg.data {
                        let subagent_id = data
                            .get("subagent_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let parent_id = data
                            .get("parent_id")
                            .and_then(|v| v.as_str())
                            .or(msg.agent_id.as_deref())
                            .unwrap_or("")
                            .to_string();
                        let task = data
                            .get("task")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        self.subagent_parent_map.insert(subagent_id.clone(), parent_id);
                        self.active_subagents.insert(
                            subagent_id.clone(),
                            SubagentEntry {
                                subagent_id,
                                task,
                                status: SubagentStatus::Running,
                                tool_count: 0,
                                estimated_tokens: None,
                                current_activity: None,
                                tool_steps: Vec::new(),
                            },
                        );
                    }
                }
                if msg.phase.as_deref() == Some("subagent_result") {
                    if let Some(data) = &msg.data {
                        let subagent_id = data
                            .get("subagent_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(entry) = self.active_subagents.get_mut(&subagent_id) {
                            entry.status = SubagentStatus::Done;
                            entry.current_activity = None;
                        }
                        self.subagent_parent_map.remove(&subagent_id);
                        // Check if all subagents are done
                        let all_done = self
                            .active_subagents
                            .values()
                            .all(|e| e.status == SubagentStatus::Done);
                        if all_done && !self.active_subagents.is_empty() {
                            self.finalize_subagent_group();
                        }
                    }
                }
                if msg.phase.as_deref() == Some("plan_update") {
                    self.discard_streaming();
                    if let Some(data) = &msg.data {
                        if let Some(plan) = data.get("plan") {
                            let summary = plan
                                .get("summary")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Plan")
                                .to_string();
                            let status = plan
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("planned")
                                .to_string();
                            let items: Vec<PlanDisplayItem> = plan
                                .get("items")
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .map(|item| PlanDisplayItem {
                                            title: item
                                                .get("title")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("?")
                                                .to_string(),
                                            status: item
                                                .get("status")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("pending")
                                                .to_string(),
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            // Dedup items: strip "Step N: " prefixes and keep
                            // only one copy of each unique title.
                            let items = dedup_plan_items(items);

                            // Replace the LAST PlanBlock (regardless of summary)
                            // since the agent may update its plan title mid-run.
                            let replaced = self.blocks.iter_mut().rev().any(|block| {
                                if let DisplayBlock::PlanBlock {
                                    summary: existing_summary,
                                    items: existing_items,
                                    status: existing_status,
                                    ..
                                } = block
                                {
                                    *existing_summary = summary.clone();
                                    *existing_items = items.clone();
                                    *existing_status = status.clone();
                                    return true;
                                }
                                false
                            });
                            if replaced {
                                // Clear prompt if the updated plan is no longer pending approval
                                if status != "planned" {
                                    self.prompt = None;
                                }
                            } else {
                                self.blocks.push(DisplayBlock::PlanBlock {
                                    summary,
                                    items,
                                    status: status.clone(),
                                });
                                if status == "planned" {
                                    let ctx_pct = self.context_usage_pct();
                                    let mut options = Vec::new();
                                    if ctx_pct >= 40 {
                                        options.push(format!(
                                            "Start (new session, {}% context used)",
                                            ctx_pct
                                        ));
                                    }
                                    options.push("Start (continue session)".to_string());
                                    options.push("Reject plan".to_string());
                                    options.push("Give feedback".to_string());
                                    self.prompt = Some(InteractivePrompt {
                                        options,
                                        selected: 0,
                                    });
                                } else {
                                    // Clear prompt when plan is no longer pending approval
                                    self.prompt = None;
                                }
                            }
                        }
                    }
                }
                if msg.phase.as_deref() == Some("change_report") {
                    if let Some(data) = &msg.data {
                        let files: Vec<ChangedFile> = data
                            .get("files")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|item| {
                                        let path = item.get("path")?.as_str()?.to_string();
                                        let summary = item
                                            .get("summary")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Updated")
                                            .to_string();
                                        let diff = item
                                            .get("diff")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        Some(ChangedFile { path, summary, diff })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let truncated_count = data
                            .get("truncated_count")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;
                        if !files.is_empty() {
                            self.blocks.push(DisplayBlock::ChangeReport {
                                files,
                                truncated_count,
                            });
                        }
                    }
                }
                if msg.phase.as_deref() == Some("outcome") {
                    self.discard_streaming();
                    self.finalize_tool_group();
                    self.finalize_subagent_group();
                    self.status_state = "idle".to_string();
                    self.status_tool = None;
                    self.prompt = None;
                }
                if self.session_id.is_none() {
                    if let Some(sid) = msg.session_id {
                        self.session_id = Some(sid);
                    }
                }
            }
            "ask_user" => {
                if let Some(data) = &msg.data {
                    let question_id = data
                        .get("question_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let questions = data.get("questions").and_then(|v| v.as_array());
                    if let Some(questions) = questions {
                        let header = questions
                            .first()
                            .and_then(|q| q.get("header"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if header == "Permission" {
                            // Permission prompt: show question and options as InteractivePrompt.
                            let question_text = questions
                                .first()
                                .and_then(|q| q.get("question"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("Permission required");
                            self.push_system(&format!("Permission: {}", question_text));
                            let options: Vec<String> = questions
                                .first()
                                .and_then(|q| q.get("options"))
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|o| {
                                            o.get("label").and_then(|v| v.as_str()).map(String::from)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            if !options.is_empty() {
                                self.pending_ask_user_id = Some(question_id);
                                self.prompt = Some(InteractivePrompt {
                                    options,
                                    selected: 0,
                                });
                            }
                        }
                        // Non-permission AskUser prompts are not handled in TUI yet.
                    }
                }
            }
            "model_fallback" => {
                let text = msg.text.unwrap_or_else(|| "Model switched".to_string());
                self.push_system(&format!("\u{26A0} {text}"));
            }
            "tool_progress" => {
                if let Some(data) = &msg.data {
                    let line = data
                        .get("line")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !line.is_empty() {
                        self.status_tool = Some(line.to_string());
                    }
                }
            }
            "connection" => {
                match msg.phase.as_deref() {
                    Some("connected") => {
                        self.connection_status = ConnectionStatus::Connected;
                        self.last_seq = 0;
                        self.push_system("Reconnected to server");
                        self.trigger_resync();
                    }
                    Some("disconnected") => {
                        self.connection_status = ConnectionStatus::Disconnected;
                        self.push_system("Disconnected from server — reconnecting…");
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn finalize_streaming(&mut self) {
        if self.is_streaming && !self.streaming_buffer.is_empty() {
            let text = std::mem::take(&mut self.streaming_buffer);
            let agent = self.status_agent.clone();
            self.push_agent(&agent, &text);
        }
        self.streaming_buffer.clear();
        self.is_streaming = false;
    }

    fn discard_streaming(&mut self) {
        self.streaming_buffer.clear();
        self.is_streaming = false;
    }

    /// Finalize the active tool group: push it as a ToolGroup display block.
    fn finalize_tool_group(&mut self) {
        if let Some(group) = self.active_tool_group.take() {
            if !group.steps.is_empty() {
                let estimated_tokens = self.last_context_tokens.get(&group.agent_id).copied();
                let duration_secs = self
                    .run_start_ts
                    .get(&group.agent_id)
                    .map(|t| t.elapsed().as_secs());
                self.blocks.push(DisplayBlock::ToolGroup {
                    agent_id: group.agent_id,
                    steps: group.steps,
                    collapsed: !self.verbose_mode,
                    estimated_tokens,
                    duration_secs,
                });
            }
        }
    }

    /// Finalize active subagents into a SubagentGroup display block.
    fn finalize_subagent_group(&mut self) {
        if !self.active_subagents.is_empty() {
            let entries: Vec<SubagentEntry> =
                self.active_subagents.drain().map(|(_, e)| e).collect();
            self.blocks.push(DisplayBlock::SubagentGroup { entries });
        }
    }

    /// Strip internal JSON (tool calls, actions) from a message.
    fn strip_internal_json(text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        if !trimmed.contains('{') {
            return trimmed.to_string();
        }

        let mut result = String::new();
        let bytes = trimmed.as_bytes();
        let mut pos = 0;

        while pos < bytes.len() {
            if bytes[pos] == b'{' {
                if let Some(end) = Self::find_json_object_end(trimmed, pos) {
                    let json_slice = &trimmed[pos..end];
                    let is_internal = (json_slice.contains("\"name\"")
                        && json_slice.contains("\"args\""))
                        || json_slice.contains("\"type\"");
                    if is_internal {
                        pos = end;
                        continue;
                    }
                }
                result.push('{');
                pos += 1;
            } else {
                result.push(bytes[pos] as char);
                pos += 1;
            }
        }

        result.trim().to_string()
    }

    fn find_json_object_end(s: &str, start: usize) -> Option<usize> {
        let bytes = s.as_bytes();
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escape = false;

        for i in start..bytes.len() {
            let c = bytes[i];
            if escape {
                escape = false;
                continue;
            }
            if in_string {
                if c == b'\\' {
                    escape = true;
                } else if c == b'"' {
                    in_string = false;
                }
                continue;
            }
            match c {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub fn render(&mut self, f: &mut ratatui::Frame) {
        use ratatui::layout::{Constraint, Direction, Layout};

        // Fixed bottom: divider(1) + input(1) + divider(1) + status(1) = 4 lines
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),   // Scrollable content
                Constraint::Length(4), // Fixed input area
            ])
            .split(f.area());

        let content_area = chunks[0];
        let input_area = chunks[1];
        let width = content_area.width;
        let content_height = content_area.height as usize;

        // ── Scrollable content ──────────────────────────────────────
        let mut all_lines: Vec<Line<'static>> = Vec::new();
        all_lines.extend(self.banner.iter().cloned());

        for block in &self.blocks {
            // Skip inline plan blocks that are executing — sticky footer handles them.
            if matches!(block, DisplayBlock::PlanBlock { status, .. } if status != "planned" && status != "completed") {
                continue;
            }
            all_lines.extend(render::render_block(block, width));
            // Show interactive prompt right after a pending plan block
            if matches!(block, DisplayBlock::PlanBlock { status, .. } if status == "planned") {
                if let Some(prompt) = &self.prompt {
                    all_lines.push(Line::from(""));
                    for (i, option) in prompt.options.iter().enumerate() {
                        let marker = if i == prompt.selected { ">" } else { " " };
                        let label = format!("  {} {}. {}", marker, i + 1, option);
                        let style = if i == prompt.selected {
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        all_lines.push(Line::from(Span::styled(label, style)));
                    }
                    all_lines.push(Line::from(""));
                    all_lines.push(Line::from(Span::styled(
                        "  ↑↓ to select, Enter to confirm",
                        Style::default().fg(Color::DarkGray),
                    )));
                    all_lines.push(Line::from(""));
                }
            }
        }

        // Active (in-progress) tool group
        if let Some(group) = &self.active_tool_group {
            all_lines.extend(render::render_tool_group_active(&group.steps));
        }

        // Active (in-progress) subagent tree — rendered live
        if !self.active_subagents.is_empty() {
            let entries: Vec<&SubagentEntry> = self.active_subagents.values().collect();
            all_lines.extend(render::render_subagent_group_live(&entries));
        }

        // Streaming buffer (dim text)
        if self.is_streaming && !self.streaming_buffer.is_empty() {
            for l in self.streaming_buffer.lines() {
                all_lines.push(Line::from(Span::styled(
                    l.to_string(),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }

        // Sticky plan: show the active executing plan at the bottom of content
        // (like Claude Code's todo list). Skip "planned" (shown inline) and "completed".
        let active_plan = self.blocks.iter().rev().find_map(|b| {
            if let DisplayBlock::PlanBlock {
                summary,
                items,
                status,
            } = b
            {
                if status != "completed" && status != "planned" {
                    return Some((summary.clone(), items.clone()));
                }
            }
            None
        });
        if let Some((summary, items)) = active_plan {
            all_lines.push(Line::from(""));
            all_lines.extend(render::render_plan_sticky(&summary, &items));
        }

        // Compute total wrapped rows (accounts for line wrapping)
        let total_wrapped: usize = all_lines
            .iter()
            .map(|line| {
                let w = line.width();
                if w == 0 || width == 0 {
                    1
                } else {
                    (w + width as usize - 1) / width as usize
                }
            })
            .sum();

        let max_scroll = total_wrapped.saturating_sub(content_height);
        self.scroll_offset = self.scroll_offset.min(max_scroll);
        let scroll_y = max_scroll.saturating_sub(self.scroll_offset);

        let text = Text::from(all_lines);
        let output = Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((scroll_y as u16, 0));
        f.render_widget(output, content_area);

        // Scroll indicator overlay when not at the bottom
        if self.scroll_offset > 0 {
            let indicator = Paragraph::new(Line::from(Span::styled(
                format!("  ··· {} more rows above ···", scroll_y),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
            let indicator_area = ratatui::layout::Rect {
                x: content_area.x,
                y: content_area.y,
                width: content_area.width,
                height: 1,
            };
            f.render_widget(indicator, indicator_area);
        }

        // ── Fixed input area (always visible at bottom) ─────────────
        let divider = "─".repeat(width as usize);
        let mut input_spans = vec![
            Span::styled(
                "> ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(self.input.clone()),
        ];
        if !self.pending_images.is_empty() {
            input_spans.push(Span::styled(
                format!("  [{} image{}]", self.pending_images.len(), if self.pending_images.len() == 1 { "" } else { "s" }),
                Style::default().fg(Color::Magenta),
            ));
        }
        let input_line = Line::from(input_spans);

        let state_color = match self.status_state.as_str() {
            "thinking" => Color::Blue,
            "calling_tool" => Color::Yellow,
            "working" | "sending" => Color::Green,
            "model_loading" => Color::Magenta,
            _ => Color::DarkGray,
        };
        let mut status_spans = vec![
            Span::styled(
                format!(" {} ", self.status_agent),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw("  "),
            Span::styled(
                format!(" {} ", self.status_state),
                Style::default().fg(Color::White).bg(state_color),
            ),
        ];
        if let Some(tool) = &self.status_tool {
            status_spans.push(Span::styled("  ", Style::default()));
            status_spans.push(Span::styled(
                tool.clone(),
                Style::default().fg(Color::Yellow),
            ));
        }
        if let ConnectionStatus::Disconnected = &self.connection_status {
            status_spans.push(Span::styled("  ", Style::default()));
            status_spans.push(Span::styled(
                " DISCONNECTED ",
                Style::default().fg(Color::White).bg(Color::Red),
            ));
        }

        let bottom = Paragraph::new(vec![
            Line::from(Span::styled(divider.clone(), Style::default().fg(Color::DarkGray))),
            input_line,
            Line::from(Span::styled(divider, Style::default().fg(Color::DarkGray))),
            Line::from(status_spans),
        ]);
        f.render_widget(bottom, input_area);

        // Position cursor on the input line (2nd line of input_area)
        let cursor_y = input_area.y + 1;
        let cursor_x = input_area.x + 2 + self.input.len() as u16;
        f.set_cursor_position((cursor_x, cursor_y));
    }
}

/// Parse activity text from tool_status_line format into (tool_name, args_summary).
///
/// Maps patterns like:
///   "Reading file: src/main.rs" → ("Read", "src/main.rs")
///   "Running command: cargo test" → ("Bash", "cargo test")
///   "Searching: pattern" → ("Grep", "pattern")
fn parse_activity_text(text: &str) -> (String, String) {
    let mappings: &[(&str, &str)] = &[
        ("Reading file: ", "Read"),
        ("Read file: ", "Read"),
        ("Read failed: ", "Read"),
        ("Writing file: ", "Write"),
        ("Wrote file: ", "Write"),
        ("Write failed: ", "Write"),
        ("Editing file: ", "Edit"),
        ("Edited file: ", "Edit"),
        ("Edit failed: ", "Edit"),
        ("Running command: ", "Bash"),
        ("Ran command: ", "Bash"),
        ("Command failed: ", "Bash"),
        ("Searching: ", "Grep"),
        ("Searched: ", "Grep"),
        ("Search failed: ", "Grep"),
        ("Listing files: ", "Glob"),
        ("Listed files: ", "Glob"),
        ("List files failed: ", "Glob"),
        ("Delegating to subagent: ", "Delegate"),
        ("Delegated to subagent: ", "Delegate"),
        ("Delegation failed: ", "Delegate"),
        ("Fetching URL: ", "WebFetch"),
        ("Fetched URL: ", "WebFetch"),
        ("Fetch failed: ", "WebFetch"),
        ("Searching web: ", "WebSearch"),
        ("Searched web: ", "WebSearch"),
        ("Web search failed: ", "WebSearch"),
        ("Calling tool: ", "Tool"),
        ("Used tool: ", "Tool"),
        ("Tool failed: ", "Tool"),
    ];

    for (prefix, tool_name) in mappings {
        if let Some(rest) = text.strip_prefix(prefix) {
            return (tool_name.to_string(), rest.to_string());
        }
    }

    // Fallback: try to find a colon separator
    if let Some(colon_pos) = text.find(": ") {
        let label = &text[..colon_pos];
        let args = &text[colon_pos + 2..];
        // Use the label as the tool name (capitalize first letter)
        let tool = if label.is_empty() {
            "Tool".to_string()
        } else {
            let mut chars = label.chars();
            match chars.next() {
                None => "Tool".to_string(),
                Some(first) => {
                    let rest: String = chars.collect();
                    format!("{}{}", first.to_uppercase(), rest)
                }
            }
        };
        return (tool, args.to_string());
    }

    // Last resort: entire text is the tool name
    ("Tool".to_string(), text.to_string())
}

/// Strip "Step N: " prefix from a plan item title, returning the rest.
fn strip_step_prefix(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("Step ") {
        if let Some(colon_pos) = rest.find(": ") {
            let num_part = &rest[..colon_pos];
            if num_part.chars().all(|c| c.is_ascii_digit()) {
                return &rest[colon_pos + 2..];
            }
        }
    }
    s
}

/// Deduplicate plan items: normalize by stripping "Step N: " prefixes,
/// then keep only the first occurrence of each unique title.
fn dedup_plan_items(items: Vec<PlanDisplayItem>) -> Vec<PlanDisplayItem> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| {
            let normalized = strip_step_prefix(&item.title).to_string();
            seen.insert(normalized)
        })
        .collect()
}
