use crate::server::UiSseMessage;
use crate::tui_client::TuiClient;
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
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub async fn run_tui(port: u16, project_root: String) -> Result<()> {
    let client = TuiClient::new(port);

    // Wait for server health
    for _ in 0..50 {
        if client.health_check().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !client.health_check().await {
        anyhow::bail!("Server did not become healthy in time");
    }

    let sse_rx = client.subscribe_sse();
    let client = Arc::new(client);
    let mut terminal = setup_terminal()?;
    let mut app = App::new(client, sse_rx, project_root);

    let tick_rate = Duration::from_millis(100);

    loop {
        terminal.draw(|f| app.render(f))?;

        tokio::select! {
            _ = tokio::time::sleep(tick_rate) => {
                while event::poll(Duration::ZERO)? {
                    if let Event::Key(key) = event::read()? {
                        if app.handle_key(key)? {
                            restore_terminal(terminal)?;
                            return Ok(());
                        }
                    }
                }
            }
            Some(msg) = app.sse_rx.recv() => {
                app.handle_sse(msg);
            }
        }
    }
}

struct App {
    client: Arc<TuiClient>,
    sse_rx: mpsc::UnboundedReceiver<UiSseMessage>,
    input: String,
    log: Vec<String>,
    project_root: String,
    agent_id: String,
    session_id: Option<String>,
    // Status bar
    status_state: String,
    status_tool: Option<String>,
    status_agent: String,
    // Token streaming
    streaming_buffer: String,
    is_streaming: bool,
}

impl App {
    fn new(
        client: Arc<TuiClient>,
        sse_rx: mpsc::UnboundedReceiver<UiSseMessage>,
        project_root: String,
    ) -> Self {
        let mut log = Vec::new();
        log.push("linggen-agent (unified TUI + server)".to_string());
        log.push("Commands: /help, /agent <name>, @agent message, /quit".to_string());

        Self {
            client,
            sse_rx,
            input: String::new(),
            log,
            project_root,
            agent_id: "ling".to_string(),
            session_id: None,
            status_state: "idle".to_string(),
            status_tool: None,
            status_agent: "ling".to_string(),
            streaming_buffer: String::new(),
            is_streaming: false,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
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

    fn handle_command(&mut self, line: String) -> Result<bool> {
        if line == "/quit" || line == "/exit" {
            return Ok(true);
        }

        if line == "/help" {
            self.log.push("Commands:".to_string());
            self.log
                .push("  /agent <name>     switch default agent".to_string());
            self.log
                .push("  @agent message    send to specific agent".to_string());
            self.log
                .push("  /quit, /exit      exit".to_string());
            self.log
                .push("  <text>            send message to current agent".to_string());
            return Ok(false);
        }

        if let Some(rest) = line.strip_prefix("/agent ") {
            let name = rest.trim().to_string();
            self.agent_id = name.clone();
            self.status_agent = name.clone();
            self.log.push(format!("Switched to agent: {}", name));
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

        self.log.push(format!("> {}", message));
        self.status_state = "sending".to_string();

        // Fire-and-forget chat request
        let client = self.client.clone();
        let project_root = self.project_root.clone();
        let session_id = self.session_id.clone();

        tokio::spawn(async move {
            let _ = client
                .send_chat(
                    &project_root,
                    &target_agent,
                    &message,
                    session_id.as_deref(),
                )
                .await;
        });

        Ok(false)
    }

    fn handle_sse(&mut self, msg: UiSseMessage) {
        match msg.kind.as_str() {
            "token" => {
                let text = msg.text.unwrap_or_default();
                let done = msg.phase.as_deref() == Some("done");
                if !text.is_empty() {
                    self.streaming_buffer.push_str(&text);
                    self.is_streaming = true;
                }
                if done {
                    self.finalize_streaming();
                }
            }
            "message" => {
                // Finalize any pending stream first
                self.finalize_streaming();

                let role = msg
                    .data
                    .as_ref()
                    .and_then(|d| d.get("role"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("assistant");
                if role == "user" {
                    // Already shown as "> ..." on send
                    return;
                }
                if let Some(text) = msg.text {
                    if !text.is_empty() {
                        let agent = msg.agent_id.unwrap_or_default();
                        self.log.push(format!("[{}] {}", agent, text));
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
                self.status_state = status.to_string();
                self.status_tool = msg.text;
                if let Some(aid) = msg.agent_id {
                    self.status_agent = aid;
                }
            }
            "run" => {
                if msg.phase.as_deref() == Some("outcome") {
                    self.finalize_streaming();
                    self.status_state = "idle".to_string();
                    self.status_tool = None;
                }
                // Capture session_id from the first event that carries one
                if self.session_id.is_none() {
                    if let Some(sid) = msg.session_id {
                        self.session_id = Some(sid);
                    }
                }
            }
            _ => {}
        }
    }

    fn finalize_streaming(&mut self) {
        if self.is_streaming && !self.streaming_buffer.is_empty() {
            let text = std::mem::take(&mut self.streaming_buffer);
            for line in text.lines() {
                self.log.push(line.to_string());
            }
        }
        self.streaming_buffer.clear();
        self.is_streaming = false;
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Min(3),   // Output
                    Constraint::Length(1), // Status bar
                    Constraint::Length(3), // Input
                ]
                .as_ref(),
            )
            .split(f.area());

        // --- Output ---
        let output_height = chunks[0].height.saturating_sub(2) as usize; // borders
        let total_lines = self.log.len()
            + if self.is_streaming && !self.streaming_buffer.is_empty() {
                self.streaming_buffer.lines().count().max(1)
            } else {
                0
            };

        let mut text = Text::default();
        let skip = total_lines.saturating_sub(output_height);
        let mut idx = 0;

        for line in &self.log {
            if idx >= skip {
                text.lines.push(Line::from(line.as_str()));
            }
            idx += 1;
        }
        // Show streaming buffer as in-progress
        if self.is_streaming && !self.streaming_buffer.is_empty() {
            for line in self.streaming_buffer.lines() {
                if idx >= skip {
                    text.lines.push(Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                idx += 1;
            }
        }

        let output = Paragraph::new(text)
            .block(Block::default().title("Output").borders(Borders::ALL))
            .wrap(Wrap { trim: false });
        f.render_widget(output, chunks[0]);

        // --- Status bar ---
        let state_color = match self.status_state.as_str() {
            "thinking" => Color::Blue,
            "calling_tool" => Color::Yellow,
            "working" | "sending" => Color::Green,
            "model_loading" => Color::Magenta,
            _ => Color::DarkGray,
        };

        let mut spans = vec![
            Span::styled(
                format!(" {} ", self.status_agent),
                Style::default().fg(Color::White).bg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                format!(" {} ", self.status_state),
                Style::default().fg(Color::White).bg(state_color),
            ),
        ];

        if let Some(tool) = &self.status_tool {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                tool.clone(),
                Style::default().fg(Color::Yellow),
            ));
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
