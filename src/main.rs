mod agent_manager;
mod check;
mod config;
mod db;
mod engine;
mod logging;
mod ollama;
mod repl;
mod server;
mod skills;
mod state_fs;
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let (config, config_path) =
        Config::load_with_path().unwrap_or_else(|_| (Config::default(), None));
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
    let cli = Cli::parse();

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
            let (manager, rx) = agent_manager::AgentManager::new(config, db, skill_manager.clone());

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

            let config = manager.get_config();
            tracing::info!("Max Tool Iterations: {}", config.agent.max_iters);

            let models = manager.models.list_models();
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

            let agents = manager.list_agents(&ws_root).await?;
            tracing::info!("Active Agents ({}):", agents.len());
            for a in agents {
                tracing::info!("  - Name: {}, Tools: {:?}", a.name, a.tools);
            }
            tracing::info!("------------------------------");

            server::start_server(manager, skill_manager, port, dev, rx).await?;
        }
    }

    Ok(())
}
