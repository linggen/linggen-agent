use super::types::*;
use crate::config::AgentPolicyCapability;
use crate::engine::tools;
use crate::ollama::ChatMessage;
use std::collections::HashSet;

impl AgentEngine {
    pub(crate) fn system_prompt(&self) -> String {
        let mut prompt = self
            .spec_system_prompt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "You are a helpful AI assistant.".to_string());

        if !self.available_skills_metadata.is_empty() {
            prompt.push_str("\n\n## Available Skills\n\nUse the `Skill` tool to invoke a skill by name. Available skills:");
            for (name, description) in &self.available_skills_metadata {
                prompt.push_str(&format!("\n- **{}**: {}", name, description));
            }
        }

        if let Some(skill) = &self.active_skill {
            prompt.push_str("\n\n--- ACTIVE SKILL ---");
            prompt.push_str(&format!(
                "\nSkill: {}\nDescription: {}",
                skill.name, skill.description
            ));
            prompt.push_str(&format!("\n\n{}", skill.content));
            prompt.push_str("\n-------------------");
        }

        prompt
    }

    /// Build the stable portion of the system prompt (agent spec + project context + memory)
    /// and return (content, hash). This is the cacheable prefix.
    pub(crate) fn build_stable_system_content(&self) -> (String, u64) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut stable = self.system_prompt();

        // --- Project context files (AGENTS.md, CLAUDE.md, .cursorrules) ---
        {
            let context_filenames = ["AGENTS.md", "CLAUDE.md", ".cursorrules"];
            let mut seen: std::collections::HashSet<std::path::PathBuf> =
                std::collections::HashSet::new();
            let mut sections: Vec<(String, String)> = Vec::new();

            let mut dir: Option<&std::path::Path> = Some(self.cfg.ws_root.as_path());
            while let Some(current) = dir {
                for filename in &context_filenames {
                    let filepath = current.join(filename);
                    if let Ok(canonical) = filepath.canonicalize() {
                        if seen.contains(&canonical) {
                            continue;
                        }
                        if let Ok(content) = std::fs::read_to_string(&filepath) {
                            let content = content.trim().to_string();
                            if !content.is_empty() {
                                let label = if current == self.cfg.ws_root.as_path() {
                                    filename.to_string()
                                } else {
                                    format!("{} (from {})", filename, current.display())
                                };
                                sections.push((label, content));
                                seen.insert(canonical);
                            }
                        }
                    }
                }
                dir = current.parent();
            }
            sections.reverse();
            if !sections.is_empty() {
                stable.push_str("\n\n--- PROJECT INSTRUCTIONS ---");
                for (label, content) in &sections {
                    stable.push_str(&format!("\n\n# {}\n\n{}", label, content));
                }
                stable.push_str("\n\n--- END PROJECT INSTRUCTIONS ---");
            }
        }

        // --- Auto Memory ---
        if let Some(memory_dir) = self.tools.memory_dir() {
            let memory_path = memory_dir.join("MEMORY.md");
            let mem_dir_display = memory_dir.display().to_string();
            if let Ok(content) = std::fs::read_to_string(&memory_path) {
                let content = content.trim();
                if !content.is_empty() {
                    let truncated: String =
                        content.lines().take(200).collect::<Vec<_>>().join("\n");
                    stable.push_str(&format!(
                        "\n\n--- AUTO MEMORY ---\n\
                         You have a persistent memory directory at `{}`.\n\
                         Its contents persist across sessions. Use Write/Edit tools to update MEMORY.md.\n\
                         \n\
                         Guidelines:\n\
                         - Save stable patterns, user preferences, key architecture decisions, project structure\n\
                         - Do NOT save session-specific context, in-progress work, or unverified conclusions\n\
                         - Keep MEMORY.md concise (under 200 lines)\n\
                         - When user says \"remember X\", save it immediately\n\
                         \n## MEMORY.md\n\n{}\n\
                         --- END AUTO MEMORY ---",
                        mem_dir_display, truncated
                    ));
                }
            }
            if !stable.contains("AUTO MEMORY") {
                let mem_dir_display = memory_dir.display().to_string();
                stable.push_str(&format!(
                    "\n\n--- AUTO MEMORY ---\n\
                     You have a persistent memory directory at `{}`.\n\
                     Its contents persist across sessions. Create MEMORY.md with Write to start saving memories.\n\
                     \n\
                     Guidelines:\n\
                     - Save stable patterns, user preferences, key architecture decisions, project structure\n\
                     - Do NOT save session-specific context, in-progress work, or unverified conclusions\n\
                     - Keep MEMORY.md concise (under 200 lines)\n\
                     - When user says \"remember X\", save it immediately\n\
                     --- END AUTO MEMORY ---",
                    mem_dir_display
                ));
            }
        }

        let mut hasher = DefaultHasher::new();
        stable.hash(&mut hasher);
        let hash = hasher.finish();
        (stable, hash)
    }

    /// Build the initial message list and read-paths set for the structured agent loop.
    pub(crate) fn prepare_loop_messages(
        &mut self,
        task: &str,
    ) -> (Vec<ChatMessage>, Option<HashSet<String>>, HashSet<String>) {
        // Build stable system content with caching.
        let (stable_content, hash) = self.build_stable_system_content();
        let cache_hit = self.cached_system_prompt.as_ref().map_or(false, |c| c.input_hash == hash);
        if !cache_hit {
            self.cached_system_prompt = Some(CachedSystemPrompt {
                input_hash: hash,
                content: stable_content.clone(),
            });
        }
        let mut system = self.cached_system_prompt.as_ref().unwrap().content.clone();

        // Compute allowed tools early — needed for the response format schema.
        let mut allowed_tools = self.allowed_tool_names();
        if self.plan_mode {
            let read_only: HashSet<String> = [
                "Read", "Glob", "Grep", "WebSearch", "WebFetch", "AskUser", "ExitPlanMode",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            allowed_tools = Some(match allowed_tools {
                Some(existing) => existing.intersection(&read_only).cloned().collect(),
                None => read_only,
            });
        }

        // Dynamic content appended after cached stable prefix.

        // --- Unified Response Format (loaded from prompts/response-format.md) ---
        {
            let tools_json = self.tools.tool_schema_json(allowed_tools.as_ref());
            if let Some(rendered) = self.prompt_store.render(
                crate::prompts::RESPONSE_FORMAT,
                &[("tools", &tools_json)],
            ) {
                system.push_str("\n\n");
                system.push_str(&rendered);
            }
        }

        // Plan mode: restrict to read-only tools and instruct the model to produce a plan.
        if self.plan_mode {
            if let Some(content) = self.prompt_store.get(crate::prompts::PLAN_MODE) {
                system.push_str("\n\n");
                system.push_str(content);
            }
        }

        // If executing an approved plan, inject the plan into the prompt.
        if let Some(plan) = &self.plan {
            if plan.status == PlanStatus::Approved || plan.status == PlanStatus::Executing {
                let plan_content = if let Some(text) = &plan.plan_text {
                    text.clone()
                } else {
                    // Fallback for item-based task lists (progress tracking)
                    plan.items
                        .iter()
                        .enumerate()
                        .map(|(i, item)| {
                            let desc = item.description.as_deref().unwrap_or("");
                            format!(
                                "{}. [{}] {} {}",
                                i + 1,
                                serde_json::to_string(&item.status)
                                    .unwrap_or_default()
                                    .trim_matches('"'),
                                item.title,
                                desc
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                if let Some(rendered) = self.prompt_store.render(
                    crate::prompts::PLAN_EXECUTE,
                    &[("plan_text", &plan_content)],
                ) {
                    system.push_str("\n\n");
                    system.push_str(&rendered);
                }
            }
        }

        let mut messages = vec![ChatMessage::new("system", system)];
        self.push_context_record(
            ContextType::System,
            Some("structured_loop_prompt".to_string()),
            None,
            None,
            messages[0].content.clone(),
            serde_json::json!({ "mode": "structured" }),
        );

        // Include chat history so the model has context of the current conversation.
        messages.extend(self.chat_history.clone());

        for obs in &self.observations {
            messages.push(ChatMessage::new("user", Self::observation_for_model(obs)));
        }

        // Provide workspace info + task (last user message).
        // Tool schema and action format are already in the system prompt.
        let task_content = self.prompt_store.render(
            crate::prompts::TASK_BOOTSTRAP,
            &[
                ("ws_root", &self.cfg.ws_root.display().to_string()),
                ("platform", std::env::consts::OS),
                ("role", &format!("{:?}", self.role)),
                ("task", task),
            ],
        ).unwrap_or_else(|| format!(
            "Autonomous agent loop started.\n\nWorkspace root: {}\nPlatform: {}\nCurrent Role: {:?}\n\nTask: {}",
            self.cfg.ws_root.display(), std::env::consts::OS, self.role, task,
        ));
        let task_msg = ChatMessage::new("user", task_content);
        // Attach any pending images to the task message, then clear them.
        let images = std::mem::take(&mut self.pending_images);
        let task_msg = if images.is_empty() { task_msg } else { task_msg.with_images(images) };
        messages.push(task_msg);
        self.push_context_record(
            ContextType::UserInput,
            Some("structured_bootstrap".to_string()),
            Some("system".to_string()),
            self.agent_id.clone(),
            messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default(),
            serde_json::json!({ "source": "run_agent_loop" }),
        );

        // Pre-populate read_paths from prior context.
        let mut read_paths: HashSet<String> = HashSet::new();
        let ws_root = self.cfg.ws_root.clone();
        let mut ingest_read_file_text = |text: &str| {
            if !text.contains("Read:") || text.contains("tool_error:") {
                return;
            }
            if let Some(start) = text.find("Read: ") {
                let path_part = &text[start + 6..];
                let raw_path = path_part.split_whitespace().next().unwrap_or("");
                if raw_path.is_empty() {
                    return;
                }
                let clean_path = raw_path
                    .trim_end_matches(')')
                    .trim_end_matches(',')
                    .trim_end_matches('.')
                    .to_string();
                if clean_path.is_empty() {
                    return;
                }
                read_paths.insert(clean_path.clone());
                if let Ok(abs) = ws_root.join(&clean_path).canonicalize() {
                    if let Ok(rel) = abs.strip_prefix(&ws_root) {
                        read_paths.insert(rel.to_string_lossy().to_string());
                    }
                }
            }
        };
        for obs in &self.observations {
            if obs.name == "Read" {
                ingest_read_file_text(&obs.content);
            }
        }
        for msg in &self.chat_history {
            ingest_read_file_text(&msg.content);
        }

        (messages, allowed_tools, read_paths)
    }

    // -----------------------------------------------------------------------
    // Tool filtering
    // -----------------------------------------------------------------------

    pub(crate) fn allowed_tool_names(&self) -> Option<HashSet<String>> {
        // When a skill is active and declares allowed-tools, those take
        // precedence — the agent can only use the tools the skill permits.
        if let Some(skill) = &self.active_skill {
            if !skill.allowed_tools.is_empty() {
                let mut allowed = skill
                    .allowed_tools
                    .iter()
                    .filter_map(|tool| {
                        if let Some(name) = tools::canonical_tool_name(tool) {
                            return Some(name.to_string());
                        }
                        if self.tools.has_skill_tool(tool) {
                            return Some(tool.to_string());
                        }
                        None
                    })
                    .collect::<HashSet<String>>();
                // Skill tool is always allowed so the model can discover/invoke skills.
                allowed.insert("Skill".to_string());
                return Some(allowed);
            }
        }

        let spec = self.spec.as_ref()?;
        if spec.tools.is_empty() {
            return None;
        }
        // Wildcard means unrestricted tool access for this agent.
        if spec.tools.iter().any(|tool| tool.trim() == "*") {
            return None;
        }

        let mut allowed = spec
            .tools
            .iter()
            .filter_map(|tool| {
                // Builtin tools are resolved via canonical_tool_name.
                if let Some(name) = tools::canonical_tool_name(tool) {
                    return Some(name.to_string());
                }
                // Skill tools are recognised by the registry.
                if self.tools.has_skill_tool(tool) {
                    return Some(tool.to_string());
                }
                None
            })
            .collect::<HashSet<String>>();

        // Skill tool is always allowed so the model can discover/invoke skills.
        allowed.insert("Skill".to_string());

        Some(allowed)
    }

    pub(crate) fn is_tool_allowed(&self, allowed: &HashSet<String>, requested_tool: &str) -> bool {
        // Builtin tools: check via canonical name.
        if let Some(canonical) = tools::canonical_tool_name(requested_tool) {
            return allowed.contains(canonical);
        }
        // Skill tools: check by exact name.
        allowed.contains(requested_tool)
    }

    pub(crate) fn agent_allows_policy(&self, capability: AgentPolicyCapability) -> bool {
        self.spec
            .as_ref()
            .map(|spec| spec.allows_policy(capability))
            .unwrap_or(false)
    }

    pub(crate) fn render_loop_breaker_prompt(template: &str, tool: &str) -> String {
        let mut rendered = template.replace("{tool}", tool);
        if rendered.contains("{}") {
            rendered = rendered.replacen("{}", tool, 1);
        }
        rendered
    }

    /// Get a rendered nudge prompt from the prompt store, with optional variable substitution.
    pub(crate) fn nudge(&self, key: &str, vars: &[(&str, &str)]) -> String {
        self.prompt_store
            .render(key, vars)
            .unwrap_or_else(|| format!("[missing prompt: {}]", key))
    }
}
