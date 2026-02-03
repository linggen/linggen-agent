use crate::agent_manager::models::ModelManager;
use crate::check;
use crate::engine::{AgentEngine, AgentOutcome, AgentRole, EngineConfig, TaskPacket};
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
use std::time::{Duration, Instant};
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
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| app.render(f))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if app.handle_key(key).await? {
                    break;
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }

    restore_terminal(terminal)?;
    Ok(())
}

struct App {
    engine: AgentEngine,
    input: String,
    log: Vec<String>,
    ws_root: PathBuf,
    current_task_packet: Option<TaskPacket>,
}

impl App {
    fn new(cfg: CoderConfig) -> Result<Self> {
        let model_manager = Arc::new(ModelManager::new(vec![crate::config::ModelConfig {
            id: "repl".to_string(),
            provider: "ollama".to_string(),
            url: cfg.ollama_url,
            model: cfg.model,
            api_key: None,
        }]));
        let ws_root = cfg.ws_root.clone();
        let engine = AgentEngine::new(
            EngineConfig {
                ws_root: cfg.ws_root,
                max_iters: cfg.max_iters,
                stream: cfg.stream,
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
            engine,
            input: String::new(),
            log,
            ws_root,
            current_task_packet: None,
        })
    }

    async fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
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
                    if self.run_command(line).await? {
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

    async fn run_command(&mut self, line: String) -> Result<bool> {
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
            self.engine.set_task(rest.trim().to_string());
            self.log.push(format!("Task set: {}", rest));
            return Ok(false);
        }

        if line == "/run" {
            self.log.push("Running agent...".to_string());
            match self.engine.run_agent_loop(None).await {
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
                    self.log.push("Use /approve to start coding.".to_string());
                    self.current_task_packet = Some(packet);
                }
                Ok(AgentOutcome::Patch(diff)) => {
                    self.log.push("--- PATCH BEGIN ---".to_string());
                    self.log.push(diff);
                    self.log.push("--- PATCH END ---".to_string());
                }
                Ok(AgentOutcome::Ask(question)) => {
                    self.log.push(format!("Agent question: {}", question));
                }
                Ok(AgentOutcome::None) => {
                    self.log.push("No outcome produced.".to_string());
                }
                Err(e) => {
                    self.log.push(format!("Error: {}", e));
                }
            }
            return Ok(false);
        }

        if line == "/approve" {
            if let Some(packet) = self.current_task_packet.take() {
                self.log
                    .push("Plan approved. Switching to Coder role.".to_string());
                self.engine.set_role(AgentRole::Coder);
                self.engine.set_task(format!(
                    "Implement the following task:\nTitle: {}\nUser Stories:\n{}\nAcceptance Criteria:\n{}",
                    packet.title,
                    packet.user_stories.join("\n"),
                    packet.acceptance_criteria.join("\n")
                ));
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
        self.engine.set_task(line.clone());
        self.log.push(format!("Task set: {}", line));
        Ok(false)
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Min(3), Constraint::Length(3)].as_ref())
            .split(f.area());

        let mut text = Text::default();
        for line in &self.log {
            text.lines.push(Line::from(line.as_str()));
        }

        let output = Paragraph::new(text)
            .block(Block::default().title("Output").borders(Borders::ALL))
            .wrap(Wrap { trim: false });
        f.render_widget(output, chunks[0]);

        let input = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::raw(self.input.clone()),
        ]))
        .block(Block::default().title("Input").borders(Borders::ALL));
        f.render_widget(input, chunks[1]);
        f.set_cursor_position((chunks[1].x + 3 + self.input.len() as u16, chunks[1].y + 1));
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
    // Only initialize if not already initialized
    let _ = tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .compact()
        .try_init();
}
