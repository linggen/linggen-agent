use crate::agent_manager::models::ModelManager;
use crate::check;
use crate::engine::{AgentEngine, AgentOutcome, AgentRole, EngineConfig, ReplEvent, TaskPacket};
use crate::logging;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::info;

pub struct CoderConfig {
    pub ws_root: PathBuf,
    pub ollama_url: String,
    pub model: String,
    pub max_iters: usize,
    pub stream: bool,
}

pub async fn run_coder_repl(cfg: CoderConfig) -> Result<()> {
    setup_tracing();
    info!("Starting linggen-agent CLI REPL...");
    let mut terminal = setup_terminal()?;
    let mut app = App::new(cfg)?;

    let tick_rate = Duration::from_millis(100);

    loop {
        terminal.draw(|f| app.render(f))?;

        tokio::select! {
            _ = tokio::time::sleep(tick_rate) => {
                // Poll for terminal key events (non-blocking).
                while event::poll(Duration::ZERO)? {
                    if let Event::Key(key) = event::read()? {
                        if app.handle_key(key)? {
                            restore_terminal(terminal)?;
                            return Ok(());
                        }
                    }
                }
            }
            Some(repl_event) = recv_repl_event(&mut app.repl_rx) => {
                match repl_event {
                    ReplEvent::Status { status, detail } => {
                        app.status_state = status;
                        app.status_tool = detail;
                    }
                    ReplEvent::Iteration { current, max } => {
                        app.status_iteration = Some((current, max));
                    }
                }
            }
            Some(result) = recv_run_result(&mut app.run_result_rx) => {
                if let Ok((engine, outcome)) = result {
                    app.engine = Some(engine);
                    app.status_state = "idle".to_string();
                    app.status_iteration = None;
                    app.status_tool = None;
                    app.handle_run_outcome(outcome);
                } else {
                    app.log.push("Error: run task channel dropped.".to_string());
                    app.status_state = "idle".to_string();
                }
            }
        }
    }
}

/// Helper to receive from an optional mpsc receiver.
async fn recv_repl_event(rx: &mut Option<mpsc::UnboundedReceiver<ReplEvent>>) -> Option<ReplEvent> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Helper to receive from an optional oneshot receiver.
async fn recv_run_result(
    rx: &mut Option<oneshot::Receiver<(AgentEngine, Result<AgentOutcome>)>>,
) -> Option<Result<(AgentEngine, Result<AgentOutcome>), oneshot::error::RecvError>> {
    match rx.take() {
        Some(rx) => Some(rx.await),
        None => std::future::pending().await,
    }
}

struct App {
    engine: Option<AgentEngine>,
    input: String,
    log: Vec<String>,
    ws_root: PathBuf,
    current_task_packet: Option<TaskPacket>,
    // Non-blocking run state
    run_result_rx: Option<oneshot::Receiver<(AgentEngine, Result<AgentOutcome>)>>,
    repl_rx: Option<mpsc::UnboundedReceiver<ReplEvent>>,
    // Status bar fields
    status_role: String,
    status_model: String,
    status_iteration: Option<(usize, usize)>,
    status_tool: Option<String>,
    status_state: String,
    status_task_preview: Option<String>,
}

impl App {
    fn new(cfg: CoderConfig) -> Result<Self> {
        let model_manager = Arc::new(ModelManager::new(vec![crate::config::ModelConfig {
            id: "repl".to_string(),
            provider: "ollama".to_string(),
            url: cfg.ollama_url,
            model: cfg.model,
            api_key: None,
            keep_alive: None,
        }]));
        let ws_root = cfg.ws_root.clone();
        let engine = AgentEngine::new(
            EngineConfig {
                ws_root: cfg.ws_root,
                max_iters: cfg.max_iters,
                stream: cfg.stream,
                write_safety_mode: crate::config::WriteSafetyMode::Warn,
                prompt_loop_breaker: None,
            },
            model_manager,
            "repl".to_string(),
            AgentRole::Lead,
        )?;

        let mut log = Vec::new();
        log.push("linggen-agent (multi-agent)".to_string());
        log.push("Starting in PM role. Set a task to begin planning.".to_string());
        log.push("Commands: /task <text>, /run, /check <cmd>, /approve, /help, /quit".to_string());

        Ok(Self {
            engine: Some(engine),
            input: String::new(),
            log,
            ws_root,
            current_task_packet: None,
            run_result_rx: None,
            repl_rx: None,
            status_role: "Lead".to_string(),
            status_model: "repl".to_string(),
            status_iteration: None,
            status_tool: None,
            status_state: "idle".to_string(),
            status_task_preview: None,
        })
    }

    /// Returns true if we should exit the REPL.
    fn is_running(&self) -> bool {
        self.engine.is_none()
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // While the engine is running, only allow Ctrl+C.
        if self.is_running() {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(true);
            }
            return Ok(false);
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(true);
            }
            KeyCode::Char(ch) => {
                self.input.push(ch);
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Enter => {
                let line = self.input.trim().to_string();
                self.input.clear();
                if !line.is_empty() {
                    if self.run_command(line)? {
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

    fn run_command(&mut self, line: String) -> Result<bool> {
        if line == "/quit" || line == "/exit" {
            return Ok(true);
        }

        if line == "/help" {
            self.log
                .push("/task <text>  set the current task".to_string());
            self.log
                .push("/run          run the current agent (PM or Coder)".to_string());
            self.log
                .push("/approve      approve the PM's plan and switch to Coder".to_string());
            self.log
                .push("/check <cmd>  run allowlisted verification command".to_string());
            self.log.push("/quit         exit".to_string());
            return Ok(false);
        }

        if let Some(rest) = line.strip_prefix("/task ") {
            let task = rest.trim().to_string();
            if let Some(engine) = &mut self.engine {
                engine.set_task(task.clone());
            }
            self.log.push(format!("Task set: {}", rest));
            self.status_task_preview = Some(task);
            return Ok(false);
        }

        if line == "/run" {
            let Some(mut engine) = self.engine.take() else {
                self.log.push("Engine is busy (already running).".to_string());
                return Ok(false);
            };

            self.log.push("Running agent...".to_string());
            self.status_state = "working".to_string();

            // Set up the REPL event channel so the engine can send status updates.
            let (repl_tx, repl_rx) = mpsc::unbounded_channel();
            engine.repl_events_tx = Some(repl_tx);
            self.repl_rx = Some(repl_rx);

            let (result_tx, result_rx) = oneshot::channel();
            self.run_result_rx = Some(result_rx);

            tokio::spawn(async move {
                let result = engine.run_agent_loop(None).await;
                engine.repl_events_tx = None;
                let _ = result_tx.send((engine, result));
            });

            return Ok(false);
        }

        if line == "/approve" {
            if let Some(packet) = self.current_task_packet.take() {
                self.log
                    .push("Plan approved. Switching to Coder role.".to_string());
                if let Some(engine) = &mut self.engine {
                    engine.set_role(AgentRole::Coder);
                    engine.set_task(format!(
                        "Implement the following task:\nTitle: {}\nUser Stories:\n{}\nAcceptance Criteria:\n{}",
                        packet.title,
                        packet.user_stories.join("\n"),
                        packet.acceptance_criteria.join("\n")
                    ));
                }
                self.status_role = "Coder".to_string();
            } else {
                self.log
                    .push("No plan to approve. Run /run in PM role first.".to_string());
            }
            return Ok(false);
        }

        if let Some(rest) = line.strip_prefix("/check ") {
            match check::run_check(rest.trim(), &self.ws_root) {
                Ok(out) => self.log.push(out),
                Err(e) => self.log.push(format!("check denied/failed: {}", e)),
            }
            return Ok(false);
        }

        // Default: treat as task shortcut.
        if let Some(engine) = &mut self.engine {
            engine.set_task(line.clone());
        }
        self.log.push(format!("Task set: {}", line));
        Ok(false)
    }

    fn handle_run_outcome(&mut self, outcome: Result<AgentOutcome>) {
        match outcome {
            Ok(AgentOutcome::Task(packet)) => {
                self.log.push("--- TASK PACKET RECEIVED ---".to_string());
                self.log.push(format!("Title: {}", packet.title));
                self.log.push("User Stories:".to_string());
                for s in &packet.user_stories {
                    self.log.push(format!("  - {}", s));
                }
                self.log.push("Acceptance Criteria:".to_string());
                for c in &packet.acceptance_criteria {
                    self.log.push(format!("  - {}", c));
                }
                if let Some(w) = &packet.mermaid_wireframe {
                    self.log.push("Wireframe (Mermaid):".to_string());
                    self.log.push(w.clone());
                }
                self.log
                    .push("Use /approve to start coding.".to_string());
                self.current_task_packet = Some(packet);
            }
            Ok(AgentOutcome::Patch(diff)) => {
                self.log.push("--- PATCH BEGIN ---".to_string());
                self.log.push(diff);
                self.log.push("--- PATCH END ---".to_string());
            }
            Ok(AgentOutcome::None) => {
                self.log.push("No outcome produced.".to_string());
            }
            Err(e) => {
                self.log.push(format!("Error: {}", e));
            }
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Min(3),    // Output
                    Constraint::Length(1),  // Status bar
                    Constraint::Length(3),  // Input
                ]
                .as_ref(),
            )
            .split(f.area());

        // --- Output ---
        let mut text = Text::default();
        for line in &self.log {
            text.lines.push(Line::from(line.as_str()));
        }

        let output = Paragraph::new(text)
            .block(Block::default().title("Output").borders(Borders::ALL))
            .wrap(Wrap { trim: false });
        f.render_widget(output, chunks[0]);

        // --- Status bar ---
        let state_color = match self.status_state.as_str() {
            "thinking" => Color::Blue,
            "calling_tool" => Color::Yellow,
            "working" => Color::Green,
            _ => Color::DarkGray,
        };

        let mut spans = vec![
            Span::styled(
                format!(" {} ", self.status_role),
                Style::default().fg(Color::White).bg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                format!(" {} ", self.status_model),
                Style::default().fg(Color::White).bg(Color::DarkGray),
            ),
            Span::raw(" "),
            Span::styled(
                format!(" {} ", self.status_state),
                Style::default().fg(Color::White).bg(state_color),
            ),
        ];

        if let Some((current, max)) = self.status_iteration {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("Step {}/{}", current, max),
                Style::default().fg(Color::Cyan),
            ));
        }

        if let Some(tool) = &self.status_tool {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                tool.clone(),
                Style::default().fg(Color::Yellow),
            ));
        }

        if let Some(preview) = &self.status_task_preview {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
            let truncated: String = preview.chars().take(40).collect();
            spans.push(Span::styled(truncated, Style::default().fg(Color::Gray)));
        }

        let status_bar = Paragraph::new(Line::from(spans));
        f.render_widget(status_bar, chunks[1]);

        // --- Input ---
        let input = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::raw(self.input.clone()),
        ]))
        .block(Block::default().title("Input").borders(Borders::ALL));
        f.render_widget(input, chunks[2]);
        f.set_cursor_position((chunks[2].x + 3 + self.input.len() as u16, chunks[2].y + 1));
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn setup_tracing() {
    let _ = logging::setup_tracing_with_settings(logging::LoggingSettings {
        level: None,
        directory: None,
        retention_days: None,
    });
}
