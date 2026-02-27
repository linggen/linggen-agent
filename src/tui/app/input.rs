use super::super::display::*;
use super::App;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
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
                    match block {
                        DisplayBlock::ToolGroup { collapsed, .. }
                        | DisplayBlock::SubagentGroup { collapsed, .. } => {
                            *collapsed = !self.verbose_mode;
                        }
                        _ => {}
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

    pub(super) fn handle_prompt_choice(&mut self, choice: &str) -> Result<bool> {
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

    pub(super) fn handle_command(&mut self, line: String) -> Result<bool> {
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
}
