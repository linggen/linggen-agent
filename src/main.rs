mod agent_manager;
mod check;
mod cli;
mod config;
mod db;
mod engine;
mod eval;
mod logging;
mod ollama;
mod openai;
mod repl;
mod server;
mod skills;
mod state_fs;
mod util;
mod workspace;

use crate::config::{Config, ModelConfig};
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "linggen-agent")]
#[command(about = "Linggen Agent (multi-agent) - autonomous prototype", long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Interactive multi-agent autonomous agent (CLI)
    Agent {
        /// Ollama base URL
        #[arg(long)]
        ollama_url: Option<String>,

        /// Ollama model name
        #[arg(long)]
        model: Option<String>,

        /// Workspace root. If omitted, detects by walking up for .git.
        #[arg(long)]
        root: Option<std::path::PathBuf>,

        /// Max agent tool iterations per /run
        #[arg(long)]
        max_iters: Option<usize>,

        /// Disable streaming model output (uses non-streaming requests)
        #[arg(long, default_value_t = false)]
        no_stream: bool,
    },
    /// Run eval tasks against the agent
    Eval {
        /// Workspace root. If omitted, detects by walking up for .git.
        #[arg(long)]
        root: Option<std::path::PathBuf>,

        /// Filter tasks by name (substring match)
        #[arg(long)]
        filter: Option<String>,

        /// Override max iterations per task
        #[arg(long)]
        max_iters: Option<usize>,

        /// Per-task timeout in seconds (default 300)
        #[arg(long, default_value_t = 300)]
        timeout: u64,

        /// Override agent_id for all tasks
        #[arg(long)]
        agent: Option<String>,

        /// Print agent messages during execution
        #[arg(long, default_value_t = false)]
        verbose: bool,
    },
    /// Start the agent server with Web UI (Service mode)
    Serve {
        /// Port to listen on
        #[arg(long)]
        port: Option<u16>,

        /// Ollama base URL
        #[arg(long)]
        ollama_url: Option<String>,

        /// Ollama model name
        #[arg(long)]
        model: Option<String>,

        /// Workspace root. If omitted, detects by walking up for .git.
        #[arg(long)]
        root: Option<std::path::PathBuf>,

        /// Enable dev mode (proxy static assets)
        #[arg(long, default_value_t = false)]
        dev: bool,
    },
    /// Diagnose installation health
    Doctor,
    /// Start server as a background daemon
    Start {
        /// Port to listen on
        #[arg(long)]
        port: Option<u16>,

        /// Workspace root
        #[arg(long)]
        root: Option<std::path::PathBuf>,
    },
    /// Stop the background daemon
    Stop,
    /// Show server status
    Status,
    /// Install all skills from linggen/skills repository
    Init {
        /// Install to global ~/.linggen/skills/ instead of project
        #[arg(long, default_value_t = false)]
        global: bool,

        /// Workspace root (for project-scoped install)
        #[arg(long)]
        root: Option<std::path::PathBuf>,
    },
    /// Self-update the binary to the latest release
    #[command(alias = "update")]
    Install,
    /// Manage skills
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
}

#[derive(Subcommand, Debug)]
enum SkillsAction {
    /// Install a skill
    Add {
        /// Skill name
        name: String,

        /// GitHub repository URL
        #[arg(long)]
        repo: Option<String>,

        /// Git ref (branch/tag)
        #[arg(long, alias = "ref")]
        git_ref: Option<String>,

        /// Install globally (~/.linggen/skills/)
        #[arg(long, default_value_t = false)]
        global: bool,

        /// Overwrite existing installation
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// Remove an installed skill
    Remove {
        /// Skill name
        name: String,

        /// Remove from global scope
        #[arg(long, default_value_t = false)]
        global: bool,
    },
    /// List installed skills
    List,
    /// Search the marketplace
    Search {
        /// Search query
        query: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let (config, config_path) = Config::load_with_path().unwrap_or_else(|e| {
        eprintln!("Warning: failed to load config, using defaults: {e}");
        (Config::default(), None)
    });

    let cli = Cli::parse();

    // Lightweight subcommands — no tracing/AgentManager needed.
    match &cli.cmd {
        Command::Doctor => {
            return cli::doctor::run(&config, config_path.as_deref()).await;
        }
        Command::Start { port, root } => {
            return cli::daemon::start(&config, *port, root.clone()).await;
        }
        Command::Stop => {
            return cli::daemon::stop().await;
        }
        Command::Status => {
            return cli::daemon::status(&config, config_path.as_deref()).await;
        }
        Command::Init { global, root } => {
            return cli::init::run(*global, root.clone()).await;
        }
        Command::Install => {
            return cli::self_update::run().await;
        }
        Command::Skills { action } => {
            let sa = match action {
                SkillsAction::Add {
                    name,
                    repo,
                    git_ref,
                    global,
                    force,
                } => cli::skills_cmd::SkillsAction::Add {
                    name: name.clone(),
                    repo: repo.clone(),
                    git_ref: git_ref.clone(),
                    global: *global,
                    force: *force,
                },
                SkillsAction::Remove { name, global } => cli::skills_cmd::SkillsAction::Remove {
                    name: name.clone(),
                    global: *global,
                },
                SkillsAction::List => cli::skills_cmd::SkillsAction::List,
                SkillsAction::Search { query } => cli::skills_cmd::SkillsAction::Search {
                    query: query.clone(),
                },
            };
            return cli::skills_cmd::run(sa, &config).await;
        }
        _ => {}
    }

    // Full subcommands — need tracing.
    let log_dir = match logging::setup_tracing_with_settings(logging::LoggingSettings {
        level: config.logging.level.as_deref(),
        directory: config.logging.directory.as_deref(),
        retention_days: config.logging.retention_days,
    }) {
        Ok(path) => Some(path),
        Err(err) => {
            eprintln!("Failed to initialize logging: {err}");
            None
        }
    };

    match cli.cmd {
        Command::Agent {
            ollama_url,
            model,
            root,
            max_iters,
            no_stream,
        } => {
            let ws_root = workspace::resolve_workspace_root(root)?;
            let default_model = config
                .models
                .first()
                .cloned()
                .unwrap_or_else(|| ModelConfig {
                    id: "default".to_string(),
                    provider: "ollama".to_string(),
                    url: "http://127.0.0.1:11434".to_string(),
                    model: "qwen3-coder".to_string(),
                    api_key: None,
                    keep_alive: None,
                    context_window: None,
                });
            let cfg = repl::CoderConfig {
                ws_root,
                ollama_url: ollama_url.unwrap_or(default_model.url),
                model: model.unwrap_or(default_model.model),
                max_iters: max_iters.unwrap_or(config.agent.max_iters),
                stream: !no_stream,
            };
            repl::run_coder_repl(cfg).await?;
        }
        Command::Serve {
            port,
            ollama_url: _,
            model: _,
            root,
            dev,
        } => {
            let ws_root = workspace::resolve_workspace_root(root)?;
            let port = port.unwrap_or(config.server.port);

            let db = Arc::new(db::Db::new()?);
            let skill_manager = Arc::new(skills::SkillManager::new());
            let config_dir = config_path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
            let (manager, rx) = agent_manager::AgentManager::new(config, config_dir, db, skill_manager.clone());

            // Load skills for initial project
            let _ = skill_manager.load_all(Some(&ws_root)).await;

            // Register initial project
            let _ = manager.get_or_create_project(ws_root.clone()).await?;

            // Log startup info
            tracing::info!("--- Linggen Agent Startup ---");
            if let Some(path) = config_path.as_ref() {
                tracing::info!("Config File: {}", path.display());
            } else {
                tracing::info!("Config File: (default)");
            }
            tracing::info!("Workspace Root: {}", ws_root.display());
            tracing::info!("Server Port: {}", port);
            if let Some(dir) = log_dir.as_ref() {
                tracing::info!("Log Directory: {}", dir.display());
            }

            let config_snap = manager.get_config_snapshot().await;
            tracing::info!("Max Tool Iterations: {}", config_snap.agent.max_iters);

            {
                let models_guard = manager.models.read().await;
                let models = models_guard.list_models();
                tracing::info!("Configured Models ({}):", models.len());
                for m in models {
                    tracing::info!(
                        "  - ID: {}, Provider: {}, Model: {}, URL: {}",
                        m.id,
                        m.provider,
                        m.model,
                        m.url
                    );
                }
            }

            let agents = manager.list_agents(&ws_root).await?;
            tracing::info!("Active Agents ({}):", agents.len());
            for a in agents {
                tracing::info!("  - Name: {}, Tools: {:?}", a.name, a.tools);
            }
            tracing::info!("------------------------------");

            server::start_server(manager, skill_manager, port, dev, rx).await?;
        }
        Command::Eval {
            root,
            filter,
            max_iters,
            timeout,
            agent,
            verbose: _,
        } => {
            let ws_root = workspace::resolve_workspace_root(root)?;
            let eval_cfg = eval::EvalConfig {
                ws_root,
                filter,
                max_iters,
                timeout,
                agent_override: agent,
            };
            let summary = eval::run_eval(eval_cfg).await?;
            if summary.failed > 0 {
                std::process::exit(1);
            }
        }
        // Lightweight commands already handled above.
        Command::Doctor
        | Command::Start { .. }
        | Command::Stop
        | Command::Status
        | Command::Init { .. }
        | Command::Install
        | Command::Skills { .. } => unreachable!(),
    }

    Ok(())
}
